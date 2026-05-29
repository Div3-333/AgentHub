//! Tree-sitter AST indexer for auto-context (blueprint §13.1).
//!
//! Walks the workspace (respecting `.gitignore`), parses supported languages with
//! tree-sitter, and keeps an in-memory symbol map. Watches for file changes via
//! `notify`, with a polling fallback when watching is unavailable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ignore::WalkBuilder;
use notify::event::{CreateKind, ModifyKind, RemoveKind};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use tracing::{debug, trace, warn};
use tree_sitter::{Language, Node, Parser};

/// Kind of indexed symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Class,
    Type,
    Module,
    Import,
}

/// A named span inside a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: SymbolKind,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Resolved symbol for context injection.
#[derive(Debug, Clone)]
pub struct SymbolMatch {
    pub name: String,
    pub path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// In-memory AST / file index (alias used by the context injector).
pub type ContextIndexer = AstIndexer;

/// Detected source language for a file path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLanguage {
    Rust,
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Go,
    PlainText,
}

impl SourceLanguage {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        Some(match ext {
            "rs" => Self::Rust,
            "py" => Self::Python,
            "ts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "js" => Self::JavaScript,
            "jsx" => Self::Jsx,
            "go" => Self::Go,
            "md" | "toml" | "json" | "sql" => Self::PlainText,
            _ => return None,
        })
    }

    fn is_code(self) -> bool {
        !matches!(self, Self::PlainText)
    }

    fn tree_sitter_language(self) -> Option<Language> {
        match self {
            Self::Rust => Some(tree_sitter_rust::language()),
            Self::Python => Some(tree_sitter_python::language()),
            Self::TypeScript => Some(tree_sitter_typescript::language_typescript()),
            Self::Tsx => Some(tree_sitter_typescript::language_tsx()),
            Self::JavaScript | Self::Jsx => Some(tree_sitter_javascript::language()),
            Self::Go => Some(tree_sitter_go::language()),
            Self::PlainText => None,
        }
    }
}

/// In-memory AST / file index for the workspace.
pub struct AstIndexer {
    cwd: PathBuf,
    symbols: RwLock<HashMap<PathBuf, Vec<SymbolEntry>>>,
    plain_files: RwLock<Vec<PathBuf>>,
    pending: RwLock<Vec<PathBuf>>,
    last_poll: RwLock<HashMap<PathBuf, std::time::SystemTime>>,
}

/// Maximum time from file change to re-index (blueprint §13.1).
pub const REINDEX_SLA: Duration = Duration::from_secs(2);

/// Debounce quiet period before flushing pending paths (must be ≤ [`REINDEX_SLA`]).
pub const REINDEX_DEBOUNCE: Duration = Duration::from_millis(500);

/// Polling interval when `notify` cannot be used (with debounce, total ≤ [`REINDEX_SLA`]).
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

fn is_trackable(path: &Path) -> bool {
    SourceLanguage::from_path(path).is_some()
}

impl AstIndexer {
    /// Empty in-memory index with cwd `.` (for manual [`Self::register_file`]).
    #[must_use]
    pub fn in_memory() -> Self {
        Self::new(".")
    }

    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            symbols: RwLock::new(HashMap::new()),
            plain_files: RwLock::new(Vec::new()),
            pending: RwLock::new(Vec::new()),
            last_poll: RwLock::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Full workspace walk and index (respects `.gitignore`).
    pub fn index_all(&self) -> crate::Result<()> {
        let mut symbol_map = HashMap::new();
        let mut plain = Vec::new();

        let walker = WalkBuilder::new(&self.cwd)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .build();

        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&self.cwd)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| entry.path().to_path_buf());
            self.index_path_into(&rel, &mut symbol_map, &mut plain)?;
        }

        symbol_map.shrink_to_fit();
        plain.sort();
        plain.dedup();

        *self.symbols.write() = symbol_map;
        *self.plain_files.write() = plain;
        self.pending.write().clear();
        debug!(cwd = %self.cwd.display(), "AST index rebuild complete");
        Ok(())
    }

    /// Index or refresh a single file.
    pub fn index_file(&self, path: impl AsRef<Path>) -> crate::Result<()> {
        let path = path.as_ref();
        let rel = self.relative_path(path)?;
        let mut symbol_map = self.symbols.write();
        let mut plain = self.plain_files.write();
        self.index_path_into(&rel, &mut symbol_map, &mut plain)
    }

    /// Remove a file from the index (e.g. after deletion).
    pub fn remove_file(&self, path: impl AsRef<Path>) {
        let Ok(rel) = self.relative_path(path.as_ref()) else {
            return;
        };
        self.symbols.write().remove(&rel);
        self.plain_files.write().retain(|p| p != &rel);
        self.last_poll.write().remove(&rel);
    }

    #[must_use]
    pub fn symbols_for(&self, path: impl AsRef<Path>) -> Vec<SymbolEntry> {
        let Ok(rel) = self.relative_path(path.as_ref()) else {
            return Vec::new();
        };
        self.symbols.read().get(&rel).cloned().unwrap_or_default()
    }

    #[must_use]
    pub fn plain_files(&self) -> Vec<PathBuf> {
        self.plain_files.read().clone()
    }

    /// All symbol entries whose name matches exactly.
    #[must_use]
    pub fn find_symbol(&self, name: &str) -> Vec<(PathBuf, SymbolEntry)> {
        let symbols = self.symbols.read();
        let mut out = Vec::new();
        for (path, entries) in symbols.iter() {
            for entry in entries {
                if entry.name == name {
                    out.push((path.clone(), entry.clone()));
                }
            }
        }
        out
    }

    /// First exact-name match for context injection.
    #[must_use]
    pub fn lookup_symbol(&self, name: &str) -> Option<SymbolMatch> {
        let (rel, entry) = self.find_symbol(name).into_iter().next()?;
        let path = if rel.is_absolute() {
            rel
        } else {
            self.cwd.join(rel)
        };
        Some(SymbolMatch {
            name: entry.name,
            path,
            start_byte: entry.start_byte,
            end_byte: entry.end_byte,
        })
    }

    /// Whether `token` resolves to a regular file under `cwd` (direct path or indexed basename).
    #[must_use]
    pub fn resolve_file(&self, token: &str, cwd: &Path) -> Option<PathBuf> {
        let direct = cwd.join(token);
        if direct.is_file() {
            return Some(direct);
        }

        let rel = Path::new(token);
        let symbols = self.symbols.read();
        let plain = self.plain_files.read();

        if symbols.contains_key(rel) || plain.iter().any(|p| p == rel) {
            let abs = self.cwd.join(rel);
            if abs.is_file() {
                return Some(abs);
            }
        }

        let name = rel.file_name()?;
        for path in symbols.keys().chain(plain.iter()) {
            if path.file_name() == Some(name) {
                let abs = self.cwd.join(path);
                if abs.is_file() {
                    return Some(abs);
                }
            }
        }
        None
    }

    #[must_use]
    pub fn symbol_map(&self) -> HashMap<PathBuf, Vec<SymbolEntry>> {
        self.symbols.read().clone()
    }

    /// Manually register symbols for a path (used by injector tests and partial updates).
    pub fn register_file(&self, path: impl AsRef<Path>, entries: Vec<SymbolEntry>) {
        self.symbols
            .write()
            .insert(path.as_ref().to_path_buf(), entries);
    }

    fn relative_path(&self, path: &Path) -> crate::Result<PathBuf> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        let rel = abs
            .strip_prefix(&self.cwd)
            .map(Path::to_path_buf)
            .map_err(|_| {
                crate::AgentHubError::Config(format!(
                    "path {} is outside workspace {}",
                    abs.display(),
                    self.cwd.display()
                ))
            })?;
        Ok(rel)
    }

    fn index_path_into(
        &self,
        rel: &Path,
        symbol_map: &mut HashMap<PathBuf, Vec<SymbolEntry>>,
        plain: &mut Vec<PathBuf>,
    ) -> crate::Result<()> {
        let abs = self.cwd.join(rel);
        if !abs.is_file() {
            symbol_map.remove(rel);
            plain.retain(|p| p != rel);
            return Ok(());
        }

        let Some(lang) = SourceLanguage::from_path(rel) else {
            symbol_map.remove(rel);
            plain.retain(|p| p != rel);
            return Ok(());
        };

        if lang.is_code() {
            plain.retain(|p| p != rel);
            let source = std::fs::read_to_string(&abs).map_err(|e| {
                crate::AgentHubError::Config(format!("read {}: {e}", abs.display()))
            })?;
            let entries = extract_symbols(lang, &source)?;
            trace!(file = %rel.display(), count = entries.len(), "indexed symbols");
            if entries.is_empty() {
                symbol_map.remove(rel);
            } else {
                symbol_map.insert(rel.to_path_buf(), entries);
            }
        } else {
            symbol_map.remove(rel);
            if !plain.iter().any(|p| p == rel) {
                plain.push(rel.to_path_buf());
            }
        }
        Ok(())
    }

    fn schedule_reindex(&self, path: PathBuf) {
        let Ok(rel) = self.relative_path(&path) else {
            return;
        };
        let mut pending = self.pending.write();
        if !pending.iter().any(|p| p == &rel) {
            pending.push(rel);
        }
    }

    /// Flush paths queued by `schedule_reindex` (notify/polling). Used by background loops.
    pub fn drain_pending(&self) -> crate::Result<()> {
        let paths: Vec<PathBuf> = {
            let mut pending = self.pending.write();
            std::mem::take(&mut *pending)
        };
        for path in paths {
            let abs = self.cwd.join(&path);
            if abs.is_file() {
                self.index_file(&path)?;
            } else {
                self.remove_file(&path);
            }
        }
        Ok(())
    }

    /// Start background watcher: initial full index, then notify (or polling).
    pub async fn run_background(self: Arc<Self>) -> crate::Result<()> {
        self.index_all()?;
        if try_notify_watch(Arc::clone(&self)).is_ok() {
            debug!("AST indexer using notify watcher");
            loop {
                tokio::time::sleep(REINDEX_DEBOUNCE).await;
                if let Err(e) = self.drain_pending() {
                    warn!("AST re-index failed: {e}");
                }
            }
        } else {
            warn!("AST indexer falling back to polling (notify unavailable)");
            self.run_polling_loop().await
        }
    }

    async fn run_polling_loop(self: Arc<Self>) -> crate::Result<()> {
        loop {
            self.poll_for_changes()?;
            tokio::time::sleep(REINDEX_DEBOUNCE).await;
            if let Err(e) = self.drain_pending() {
                warn!("AST re-index failed: {e}");
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    fn poll_for_changes(&self) -> crate::Result<()> {
        let walker = WalkBuilder::new(&self.cwd)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .build();

        let mut seen: HashMap<PathBuf, std::time::SystemTime> = HashMap::new();
        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = match entry.path().strip_prefix(&self.cwd) {
                Ok(p) => p.to_path_buf(),
                Err(_) => continue,
            };
            if !is_trackable(&rel) {
                continue;
            }
            let Some(mtime) = std::fs::metadata(entry.path())
                .ok()
                .and_then(|m| m.modified().ok())
            else {
                continue;
            };
            let changed = match self.last_poll.read().get(&rel) {
                Some(prev) => mtime > *prev,
                None => true,
            };
            if changed {
                self.schedule_reindex(self.cwd.join(&rel));
            }
            seen.insert(rel, mtime);
        }

        for old in self
            .symbols
            .read()
            .keys()
            .chain(self.plain_files.read().iter())
            .cloned()
            .collect::<Vec<_>>()
        {
            if !seen.contains_key(&old) {
                self.schedule_reindex(self.cwd.join(&old));
            }
        }

        *self.last_poll.write() = seen;
        Ok(())
    }
}

fn try_notify_watch(indexer: Arc<AstIndexer>) -> crate::Result<()> {
    let cwd = indexer.cwd.clone();
    let (tx, rx) = crossbeam::channel::unbounded();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        Config::default(),
    )
    .map_err(|e| crate::AgentHubError::Config(format!("notify watcher: {e}")))?;

    watcher
        .watch(&cwd, RecursiveMode::Recursive)
        .map_err(|e| crate::AgentHubError::Config(format!("notify watch path: {e}")))?;

    let indexer_clone = Arc::clone(&indexer);
    std::thread::spawn(move || {
        let _watcher = watcher;
        while let Ok(event) = rx.recv() {
            if should_handle_event(&event) {
                for path in event.paths {
                    indexer_clone.schedule_reindex(path);
                }
            }
        }
    });

    Ok(())
}

fn should_handle_event(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(CreateKind::Any)
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Remove(RemoveKind::Any)
    )
}

fn extract_symbols(lang: SourceLanguage, source: &str) -> crate::Result<Vec<SymbolEntry>> {
    if !lang.is_code() {
        return Ok(Vec::new());
    }

    let mut parser = Parser::new();
    let ts_lang = lang.tree_sitter_language().ok_or_else(|| {
        crate::AgentHubError::Config(format!("no tree-sitter grammar for {lang:?}"))
    })?;
    parser
        .set_language(&ts_lang)
        .map_err(|e| crate::AgentHubError::Config(format!("set_language: {e}")))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| crate::AgentHubError::Config("tree-sitter parse failed".into()))?;

    let mut out = Vec::new();
    collect_symbols(lang, source, tree.root_node(), &mut out);
    out.sort_by_key(|a| a.start_byte);
    out.dedup_by(|a, b| a.start_byte == b.start_byte && a.name == b.name);
    Ok(out)
}

fn collect_symbols(lang: SourceLanguage, source: &str, node: Node, out: &mut Vec<SymbolEntry>) {
    let kind = node.kind();
    if let Some(entry) = symbol_from_node(lang, source, node, kind) {
        out.push(entry);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(lang, source, child, out);
    }
}

fn symbol_from_node(
    lang: SourceLanguage,
    source: &str,
    node: Node,
    kind: &str,
) -> Option<SymbolEntry> {
    let (symbol_kind, name_node_kind) = match lang {
        SourceLanguage::Rust => match kind {
            "function_item" => (SymbolKind::Function, "identifier"),
            "struct_item" => (SymbolKind::Struct, "type_identifier"),
            "enum_item" => (SymbolKind::Type, "type_identifier"),
            "trait_item" => (SymbolKind::Type, "type_identifier"),
            "type_item" => (SymbolKind::Type, "type_identifier"),
            "impl_item" => (SymbolKind::Type, "type_identifier"),
            "mod_item" => (SymbolKind::Module, "identifier"),
            "use_declaration" => (SymbolKind::Import, "identifier"),
            _ => return None,
        },
        SourceLanguage::Python => match kind {
            "function_definition" => (SymbolKind::Function, "identifier"),
            "class_definition" => (SymbolKind::Class, "identifier"),
            "import_statement" | "import_from_statement" => (SymbolKind::Import, "dotted_name"),
            _ => return None,
        },
        SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::JavaScript
        | SourceLanguage::Jsx => match kind {
            "function_declaration" | "generator_function_declaration" => {
                (SymbolKind::Function, "identifier")
            }
            "method_definition" => (SymbolKind::Method, "property_identifier"),
            "class_declaration" => (SymbolKind::Class, "identifier"),
            "interface_declaration" => (SymbolKind::Type, "type_identifier"),
            "type_alias_declaration" => (SymbolKind::Type, "type_identifier"),
            "import_statement" => (SymbolKind::Import, "import_clause"),
            _ => return None,
        },
        SourceLanguage::Go => match kind {
            "function_declaration" => (SymbolKind::Function, "identifier"),
            "method_declaration" => (SymbolKind::Method, "field_identifier"),
            "type_declaration" => (SymbolKind::Type, "type_identifier"),
            "import_declaration" => (SymbolKind::Import, "import_spec"),
            _ => return None,
        },
        SourceLanguage::PlainText => return None,
    };

    let name = node_name(source, node, name_node_kind, lang, kind)?;
    Some(SymbolEntry {
        name,
        kind: symbol_kind,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    })
}

fn node_name(
    source: &str,
    node: Node,
    preferred: &str,
    lang: SourceLanguage,
    node_kind: &str,
) -> Option<String> {
    for kind in [preferred, "identifier", "name", "type_identifier"] {
        if let Some(child) = find_child_named(node, kind) {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                let text = text.trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }

    match (lang, node_kind) {
        (SourceLanguage::Python, "import_statement" | "import_from_statement") => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.lines().next().unwrap_or(s).trim().to_string()),
        (
            SourceLanguage::TypeScript
            | SourceLanguage::Tsx
            | SourceLanguage::JavaScript
            | SourceLanguage::Jsx,
            "import_statement",
        ) => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.lines().next().unwrap_or(s).trim().to_string()),
        (SourceLanguage::Go, "import_declaration") => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.lines().next().unwrap_or(s).trim().to_string()),
        (SourceLanguage::Rust, "use_declaration") => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.trim().to_string()),
        _ => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    }
}

fn find_child_named<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
        if let Some(found) = find_child_named(child, kind) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant};

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/indexer")
    }

    fn write_fixture_tree(root: &Path) {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            r#"
pub struct Widget {
    id: u32,
}

pub fn spin(widget: &Widget) -> u32 {
    widget.id
}

mod inner {
    use std::io;
}
"#,
        )
        .unwrap();

        fs::write(
            root.join("app.py"),
            r#"
def greet(name: str) -> str:
    return f"hello {name}"

class Greeter:
    def run(self):
        pass

import os
"#,
        )
        .unwrap();

        fs::write(
            root.join("util.ts"),
            r#"
export function add(a: number, b: number): number {
    return a + b;
}

export interface Pair {
    a: number;
    b: number;
}

import { readFile } from "fs";
"#,
        )
        .unwrap();

        fs::write(
            root.join("component.tsx"),
            r#"
export function render(): null {
    return null;
}

export class TsxWidget {
    label: string;
}
"#,
        )
        .unwrap();

        fs::write(
            root.join("widget.jsx"),
            r#"
export function mount() {
    return null;
}

export class JsxWidget {}
"#,
        )
        .unwrap();

        fs::write(
            root.join("main.go"),
            r#"
package main

import "fmt"

type Counter struct {
    n int
}

func (c *Counter) Inc() {
    c.n++
}

func main() {
    fmt.Println("hi")
}
"#,
        )
        .unwrap();

        fs::write(root.join("README.md"), "# fixture\n").unwrap();
        fs::write(root.join("ignored.rs"), "// should not be indexed\n").unwrap();
        fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
    }

    #[test]
    fn index_all_respects_gitignore_and_plain_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());

        let indexer = AstIndexer::new(tmp.path());
        indexer.index_all().unwrap();

        let plain = indexer.plain_files();
        assert!(
            plain.iter().any(|p| p == Path::new("README.md")),
            "expected README.md in plain_files, got {plain:?}"
        );

        let symbols = indexer.symbol_map();
        assert!(!symbols.contains_key(Path::new("ignored.rs")));

        let rs = indexer.symbols_for("src/lib.rs");
        let names: Vec<&str> = rs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.iter().any(|n| *n == "Widget"), "names: {names:?}");
        assert!(names.iter().any(|n| *n == "spin"), "names: {names:?}");
    }

    #[test]
    fn extracts_python_typescript_and_go_symbols() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());
        let indexer = AstIndexer::new(tmp.path());
        indexer.index_all().unwrap();

        let py = indexer.symbols_for("app.py");
        assert!(py
            .iter()
            .any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
        assert!(py
            .iter()
            .any(|s| s.name == "Greeter" && s.kind == SymbolKind::Class));

        let ts = indexer.symbols_for("util.ts");
        assert!(ts
            .iter()
            .any(|s| s.name == "add" && s.kind == SymbolKind::Function));
        assert!(ts
            .iter()
            .any(|s| s.name == "Pair" && s.kind == SymbolKind::Type));

        let go = indexer.symbols_for("main.go");
        assert!(go
            .iter()
            .any(|s| s.name == "Counter" && s.kind == SymbolKind::Type));
        assert!(go
            .iter()
            .any(|s| s.name == "main" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn find_symbol_returns_exact_matches() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());
        let indexer = AstIndexer::new(tmp.path());
        indexer.index_all().unwrap();

        let hits = indexer.find_symbol("spin");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, PathBuf::from("src/lib.rs"));
        assert_eq!(hits[0].1.kind, SymbolKind::Function);
    }

    #[test]
    fn indexer_populates_within_two_seconds() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());
        let indexer = AstIndexer::new(tmp.path());
        let start = Instant::now();
        indexer.index_all().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "index_all took {:?}",
            start.elapsed()
        );
        assert!(!indexer.symbol_map().is_empty());
    }

    #[test]
    fn resolve_file_matches_indexed_basename() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());
        let indexer = AstIndexer::new(tmp.path());
        indexer.index_all().unwrap();

        let resolved = indexer
            .resolve_file("lib.rs", tmp.path())
            .expect("lib.rs under src/");
        assert!(resolved.ends_with("src/lib.rs"));
    }

    #[test]
    fn reindex_plain_file_after_modification() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());
        let indexer = AstIndexer::new(tmp.path());
        indexer.index_all().unwrap();

        let readme = tmp.path().join("README.md");
        fs::write(&readme, "# updated fixture\n").unwrap();
        indexer.index_file("README.md").unwrap();

        assert!(
            indexer
                .plain_files()
                .iter()
                .any(|p| p == Path::new("README.md")),
            "plain file still tracked after re-index"
        );
    }

    #[test]
    fn extracts_tsx_and_jsx_symbols() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());
        let indexer = AstIndexer::new(tmp.path());
        indexer.index_all().unwrap();

        let tsx = indexer.symbols_for("component.tsx");
        assert!(tsx
            .iter()
            .any(|s| s.name == "render" && s.kind == SymbolKind::Function));
        assert!(tsx
            .iter()
            .any(|s| s.name == "TsxWidget" && s.kind == SymbolKind::Class));

        let jsx = indexer.symbols_for("widget.jsx");
        assert!(jsx
            .iter()
            .any(|s| s.name == "mount" && s.kind == SymbolKind::Function));
        assert!(jsx
            .iter()
            .any(|s| s.name == "JsxWidget" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn pending_reindex_drains_within_sla() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());
        let indexer = AstIndexer::new(tmp.path());
        indexer.index_all().unwrap();
        assert!(indexer.find_symbol("revise").is_empty());

        let path = tmp.path().join("src/lib.rs");
        let mut content = fs::read_to_string(&path).unwrap();
        content.push_str("\npub fn revise() {}\n");
        fs::write(&path, content).unwrap();

        indexer.schedule_reindex(path);
        let start = Instant::now();
        indexer.drain_pending().unwrap();
        assert!(
            start.elapsed() < REINDEX_SLA,
            "drain_pending took {:?}",
            start.elapsed()
        );
        assert_eq!(indexer.find_symbol("revise").len(), 1);
    }

    #[test]
    fn reindex_file_after_modification() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture_tree(tmp.path());
        let indexer = AstIndexer::new(tmp.path());
        indexer.index_all().unwrap();

        assert!(indexer.find_symbol("revise").is_empty());

        let path = tmp.path().join("src/lib.rs");
        let mut content = fs::read_to_string(&path).unwrap();
        content.push_str("\npub fn revise() {}\n");
        fs::write(&path, content).unwrap();

        thread::sleep(Duration::from_millis(50));
        indexer.index_file("src/lib.rs").unwrap();

        let hits = indexer.find_symbol("revise");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1.kind, SymbolKind::Function);
    }

    #[test]
    fn static_fixtures_parse() {
        let root = fixtures_dir();
        assert!(root.is_dir(), "missing fixture dir: {}", root.display());

        let indexer = AstIndexer::new(&root);
        indexer.index_all().unwrap();

        assert!(
            indexer
                .plain_files()
                .iter()
                .any(|p| p == Path::new("README.md")),
            "README.md should be indexed as plain text"
        );
        assert!(
            !indexer.symbol_map().contains_key(Path::new("ignored.rs")),
            "gitignored files must not be indexed"
        );

        let rs = indexer.symbols_for("src/lib.rs");
        assert!(rs
            .iter()
            .any(|s| s.name == "Widget" && s.kind == SymbolKind::Struct));
        assert!(rs
            .iter()
            .any(|s| s.name == "spin" && s.kind == SymbolKind::Function));

        let py = indexer.symbols_for("app.py");
        assert!(py.iter().any(|s| s.name == "greet"));
        assert!(py.iter().any(|s| s.name == "Greeter"));

        let ts = indexer.symbols_for("util.ts");
        assert!(ts.iter().any(|s| s.name == "add"));
        assert!(ts.iter().any(|s| s.name == "Pair"));

        let go = indexer.symbols_for("main.go");
        assert!(go.iter().any(|s| s.name == "Counter"));
        assert!(go.iter().any(|s| s.name == "main"));
    }
}
