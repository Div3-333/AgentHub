//! Auto-context prompt injection before agent PTY writes.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use tracing::trace;

use crate::context::indexer::{AstIndexer, SymbolMatch};
use crate::error::{AgentHubError, Result};

const MAX_CONTEXT_CHARS: usize = 8000;
pub const TRUNCATION_SUFFIX: &str = "[...truncated for context. Full file available on request.]";
const NCONTEXT_FLAG: &str = "--nocontext";

static FILENAME_RE: OnceLock<Regex> = OnceLock::new();
static WORD_RE: OnceLock<Regex> = OnceLock::new();

fn filename_re() -> &'static Regex {
    FILENAME_RE.get_or_init(|| {
        Regex::new(r"\b[\w\-/\.]+\.(?:rs|py|tsx?|jsx?|go|toml|json|sql|md)\b")
            .expect("filename regex is valid")
    })
}

fn word_re() -> &'static Regex {
    WORD_RE.get_or_init(|| Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]*\b").expect("word regex is valid"))
}

/// Intercepts user prompts and prepends workspace context when triggers match.
pub struct ContextInjector<'a> {
    cwd: &'a Path,
    index: &'a AstIndexer,
}

impl<'a> ContextInjector<'a> {
    pub fn new(cwd: &'a Path, index: &'a AstIndexer) -> Self {
        Self { cwd, index }
    }

    /// Apply auto-context rules; returns the prompt to write to the agent PTY.
    pub fn inject(&self, prompt: &str) -> Result<String> {
        inject_context(prompt, self.cwd, self.index)
    }
}

/// Inject file or symbol context into `prompt` when triggers match (Part 13.2).
pub fn inject_context(prompt: &str, cwd: &Path, index: &AstIndexer) -> Result<String> {
    let (disabled, stripped) = strip_nocontext(prompt);
    if disabled {
        trace!(target: "agenthub::context", "auto-context disabled via --nocontext");
        return Ok(stripped);
    }

    if let Some((_token, path)) = find_filename_match(&stripped, cwd, index) {
        let content = read_and_prepare_file(&path)?;
        let file_label = path.strip_prefix(cwd).unwrap_or(&path).to_string_lossy();
        let out = format_file_context(&file_label, &content, &stripped);
        trace!(
            target: "agenthub::context",
            filename = %file_label,
            bytes = content.len(),
            "injected file context"
        );
        return Ok(out);
    }

    if let Some(hit) = find_symbol_match(&stripped, index) {
        let symbol_code = read_symbol_slice(&hit)?;
        let file_label = hit
            .path
            .strip_prefix(cwd)
            .unwrap_or(&hit.path)
            .to_string_lossy();
        let out = format_symbol_context(&hit.name, &file_label, &symbol_code, &stripped);
        trace!(
            target: "agenthub::context",
            symbol = %hit.name,
            file = %file_label,
            bytes = symbol_code.len(),
            "injected symbol context"
        );
        return Ok(out);
    }

    Ok(stripped)
}

fn strip_nocontext(prompt: &str) -> (bool, String) {
    let trimmed = prompt.trim_start();
    if let Some(rest) = trimmed.strip_prefix(NCONTEXT_FLAG) {
        return (true, rest.trim_start().to_string());
    }
    (false, prompt.to_string())
}

fn find_filename_match(
    prompt: &str,
    cwd: &Path,
    index: &AstIndexer,
) -> Option<(String, std::path::PathBuf)> {
    for cap in filename_re().find_iter(prompt) {
        let token = cap.as_str();
        if let Some(path) = index.resolve_file(token, cwd) {
            return Some((token.to_string(), path));
        }
    }
    None
}

fn find_symbol_match(prompt: &str, index: &AstIndexer) -> Option<SymbolMatch> {
    let filename_tokens: std::collections::HashSet<&str> = filename_re()
        .find_iter(prompt)
        .map(|m| m.as_str())
        .collect();

    for cap in word_re().find_iter(prompt) {
        let word = cap.as_str();
        if filename_tokens.contains(word) {
            continue;
        }
        if let Some(hit) = index.lookup_symbol(word) {
            return Some(hit);
        }
    }
    None
}

fn read_and_prepare_file(path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| AgentHubError::Context(format!("failed to read {}: {e}", path.display())))?;
    Ok(truncate_if_needed(minify_content_for_path(path, &raw)))
}

fn read_symbol_slice(hit: &SymbolMatch) -> Result<String> {
    let raw = std::fs::read_to_string(&hit.path).map_err(|e| {
        AgentHubError::Context(format!("failed to read {}: {e}", hit.path.display()))
    })?;
    let end = hit.end_byte.min(raw.len());
    let start = hit.start_byte.min(end);
    let slice = &raw[start..end];
    Ok(truncate_if_needed(minify_content(slice, true)))
}

fn format_file_context(filename: &str, content: &str, user_prompt: &str) -> String {
    format!("[Auto-Context: {filename}]\n{content}\n\n[User prompt]: {user_prompt}")
}

fn format_symbol_context(symbol: &str, filename: &str, code: &str, user_prompt: &str) -> String {
    format!("[Auto-Context: {symbol} from {filename}]\n{code}\n\n[User prompt]: {user_prompt}")
}

/// Whether `#` starts a comment for this file extension (not markdown headings).
fn strip_hash_comments_for_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("py" | "rs" | "go" | "ts" | "tsx" | "js" | "jsx" | "sql" | "toml")
    )
}

fn minify_content_for_path(path: &Path, content: &str) -> String {
    minify_content(content, strip_hash_comments_for_path(path))
}

/// Strip blank lines and single-line comments; collapse consecutive whitespace.
///
/// When `strip_hash` is false (e.g. `.md`), `#` lines are kept so headings survive minify.
pub fn minify_content(content: &str, strip_hash: bool) -> String {
    let mut out = String::new();
    for line in content.lines() {
        let without_comment = strip_line_comment(line, strip_hash);
        let collapsed: String = without_comment
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if collapsed.is_empty() {
            continue;
        }
        out.push_str(&collapsed);
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn strip_line_comment(line: &str, strip_hash: bool) -> &str {
    let mut in_string = false;
    let mut quote = b'"';
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if c == quote {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            in_string = true;
            quote = c;
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            return line[..i].trim_end();
        }
        if strip_hash && c == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return line[..i].trim_end();
        }
        i += 1;
    }
    line
}

fn truncate_if_needed(content: String) -> String {
    if content.chars().count() <= MAX_CONTEXT_CHARS {
        return content;
    }
    let truncated: String = content.chars().take(MAX_CONTEXT_CHARS).collect();
    format!("{truncated}{TRUNCATION_SUFFIX}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::indexer::AstIndexer;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_auth_rs(dir: &Path) -> PathBuf {
        let path = dir.join("auth.rs");
        let src = r#"// auth module

pub fn authenticate(user: &str) -> bool {
    !user.is_empty()
}

pub struct AuthConfig {
    pub timeout: u64,
}
"#;
        std::fs::write(&path, src).expect("write auth.rs");
        path
    }

    fn index_auth(dir: &Path) -> AstIndexer {
        let indexer = AstIndexer::new(dir);
        indexer.index_all().expect("index auth.rs");
        indexer
    }

    #[test]
    fn context_minify_strips_comments_and_blank_lines() {
        let input = "// header\n\nfn foo() {\n    let x = 1; // inline\n}\n";
        let out = minify_content(input, true);
        assert!(!out.contains("//"));
        assert!(!out.contains("inline"));
        assert!(out.contains("fn foo()"));
    }

    #[test]
    fn context_minify_preserves_markdown_headings() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("notes.md");
        std::fs::write(&path, "# Title\n\nBody text.\n").expect("write");
        let out = minify_content_for_path(&path, "# Title\n\nBody text.\n");
        assert!(out.contains("# Title"));
        assert!(out.contains("Body text."));
    }

    #[test]
    fn context_truncate_at_8000_chars() {
        let huge = "x".repeat(9000);
        let out = truncate_if_needed(huge);
        assert!(out.ends_with(TRUNCATION_SUFFIX));
        let body = out.strip_suffix(TRUNCATION_SUFFIX).expect("suffix");
        assert_eq!(body.chars().count(), MAX_CONTEXT_CHARS);
    }

    #[test]
    fn context_tsx_filename_injection() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("component.tsx");
        std::fs::write(&path, "export function render() { return null; }\n").expect("write");
        let index = AstIndexer::new(dir.path());
        index.index_all().expect("index");
        let out = inject_context("@mock-1 fix component.tsx", dir.path(), &index).expect("inject");
        assert!(out.contains("[Auto-Context: component.tsx]"));
        assert!(out.contains("export function render"));
    }

    #[test]
    fn context_filename_injection_prepends_file() {
        let dir = TempDir::new().expect("tempdir");
        write_auth_rs(dir.path());
        let index = index_auth(dir.path());
        let prompt = "@mock-1 fix auth.rs";
        let out = inject_context(prompt, dir.path(), &index).expect("inject");
        assert!(out.contains("[Auto-Context: auth.rs]"));
        assert!(out.contains("pub fn authenticate"));
        assert!(out.contains("[User prompt]: @mock-1 fix auth.rs"));
    }

    #[test]
    fn context_symbol_injection_extracts_definition() {
        let dir = TempDir::new().expect("tempdir");
        write_auth_rs(dir.path());
        let index = index_auth(dir.path());
        let prompt = "@mock-1 explain authenticate()";
        let out = inject_context(prompt, dir.path(), &index).expect("inject");
        assert!(out.contains("[Auto-Context: authenticate from auth.rs]"));
        assert!(out.contains("pub fn authenticate"));
        assert!(!out.contains("AuthConfig"));
        assert!(out.contains("[User prompt]:"));
    }

    #[test]
    fn context_nocontext_bypasses_injection() {
        let dir = TempDir::new().expect("tempdir");
        write_auth_rs(dir.path());
        let index = AstIndexer::new(dir.path());
        let prompt = "--nocontext @mock-1 fix auth.rs";
        let out = inject_context(prompt, dir.path(), &index).expect("inject");
        assert!(!out.contains("[Auto-Context"));
        assert_eq!(out, "@mock-1 fix auth.rs");
    }

    #[test]
    fn context_filename_takes_priority_over_symbol() {
        let dir = TempDir::new().expect("tempdir");
        write_auth_rs(dir.path());
        let index = index_auth(dir.path());
        let prompt = "compare auth.rs with authenticate";
        let out = inject_context(prompt, dir.path(), &index).expect("inject");
        assert!(out.contains("[Auto-Context: auth.rs]"));
        assert!(!out.contains("authenticate from"));
    }

    #[test]
    fn context_no_match_passthrough() {
        let dir = TempDir::new().expect("tempdir");
        let index = AstIndexer::new(dir.path());
        let prompt = "@mock-1 hello world";
        let out = inject_context(prompt, dir.path(), &index).expect("inject");
        assert_eq!(out, prompt);
    }

    #[test]
    fn context_python_hash_comment_stripped() {
        let input = "# comment line\n\ndef foo():\n    pass  # tail\n";
        let out = minify_content(input, true);
        assert!(!out.contains('#'));
        assert!(out.contains("def foo():"));
    }
}
