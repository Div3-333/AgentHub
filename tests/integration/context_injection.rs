//! Integration: auto-context injection (Phase 10 / Blueprint §13.2).

use agenthub_core::context::indexer::AstIndexer;
use agenthub_core::context::injector::{inject_context, minify_content, TRUNCATION_SUFFIX};
use tempfile::TempDir;

const MAX_CONTEXT_CHARS: usize = 8000;

fn write_auth_fixture(dir: &std::path::Path) {
    let src = r#"// auth module

pub fn authenticate(user: &str) -> bool {
    !user.is_empty()
}

pub struct AuthConfig {
    pub timeout: u64,
}
"#;
    std::fs::write(dir.join("auth.rs"), src).expect("write auth.rs");
}

#[test]
fn context_injection_filename_reference() {
    let dir = TempDir::new().expect("tempdir");
    write_auth_fixture(dir.path());
    let index = AstIndexer::new(dir.path());
    index.index_all().expect("index");

    let out = inject_context("@mock-1 fix auth.rs", dir.path(), &index).expect("inject");
    assert!(out.contains("[Auto-Context: auth.rs]"));
    assert!(out.contains("pub fn authenticate"));
    assert!(!out.contains("// auth module"));
    assert!(out.contains("[User prompt]: @mock-1 fix auth.rs"));
}

#[test]
fn context_injection_symbol_reference() {
    let dir = TempDir::new().expect("tempdir");
    write_auth_fixture(dir.path());
    let index = AstIndexer::new(dir.path());
    index.index_all().expect("index");

    let out = inject_context("@mock-1 explain authenticate()", dir.path(), &index).expect("inject");
    assert!(out.contains("[Auto-Context: authenticate from auth.rs]"));
    assert!(out.contains("pub fn authenticate"));
    assert!(!out.contains("AuthConfig"));
    assert!(out.contains("[User prompt]:"));
}

#[test]
fn context_injection_nocontext_flag() {
    let dir = TempDir::new().expect("tempdir");
    write_auth_fixture(dir.path());
    let index = AstIndexer::new(dir.path());
    index.index_all().expect("index");

    let out =
        inject_context("--nocontext @mock-1 fix auth.rs", dir.path(), &index).expect("inject");
    assert!(!out.contains("[Auto-Context"));
    assert_eq!(out, "@mock-1 fix auth.rs");
}

#[test]
fn context_injection_minifies_file_content() {
    let dir = TempDir::new().expect("tempdir");
    write_auth_fixture(dir.path());
    let index = AstIndexer::new(dir.path());
    index.index_all().expect("index");

    let out = inject_context("@mock-1 fix auth.rs", dir.path(), &index).expect("inject");
    assert!(!out.contains("// auth module"));
    let minified = minify_content(
        &std::fs::read_to_string(dir.path().join("auth.rs")).expect("read auth.rs"),
        true,
    );
    assert!(out.contains(&minified));
}

#[test]
fn context_injection_truncates_large_files() {
    let dir = TempDir::new().expect("tempdir");
    let body = format!("// header\n\npub fn big() {{\n{}\n}}\n", "x".repeat(9000));
    std::fs::write(dir.path().join("big.rs"), &body).expect("write big.rs");
    let index = AstIndexer::new(dir.path());
    index.index_all().expect("index");

    let out = inject_context("@mock-1 review big.rs", dir.path(), &index).expect("inject");
    assert!(out.contains("[Auto-Context: big.rs]"));
    assert!(out.contains(TRUNCATION_SUFFIX));
    let ctx_start = out.find("[Auto-Context: big.rs]").expect("header");
    let ctx_end = out.find("[User prompt]:").expect("user prompt");
    let injected = &out[ctx_start..ctx_end];
    let body_only = injected
        .strip_prefix("[Auto-Context: big.rs]\n")
        .expect("context body");
    assert!(body_only.chars().count() >= MAX_CONTEXT_CHARS);
    assert!(body_only.contains(TRUNCATION_SUFFIX));
}

#[test]
fn context_injection_no_match_passthrough() {
    let dir = TempDir::new().expect("tempdir");
    let index = AstIndexer::new(dir.path());
    index.index_all().expect("index");
    let prompt = "@mock-1 hello world";
    let out = inject_context(prompt, dir.path(), &index).expect("inject");
    assert_eq!(out, prompt);
}

#[test]
fn context_injection_filename_priority_over_symbol() {
    let dir = TempDir::new().expect("tempdir");
    write_auth_fixture(dir.path());
    let index = AstIndexer::new(dir.path());
    index.index_all().expect("index");

    let out =
        inject_context("compare auth.rs with authenticate", dir.path(), &index).expect("inject");
    assert!(out.contains("[Auto-Context: auth.rs]"));
    assert!(!out.contains("authenticate from"));
}

#[test]
fn context_injection_plain_markdown_file() {
    let dir = TempDir::new().expect("tempdir");
    std::fs::write(dir.path().join("notes.md"), "# Notes\n\nDetails here.\n").expect("write md");
    let index = AstIndexer::new(dir.path());
    index.index_all().expect("index");

    let out = inject_context("@mock-1 summarize notes.md", dir.path(), &index).expect("inject");
    assert!(out.contains("[Auto-Context: notes.md]"));
    assert!(out.contains("# Notes"));
    assert!(out.contains("[User prompt]: @mock-1 summarize notes.md"));
}
