#![expect(clippy::unwrap_used, reason = "integration tests")]

//! Error path and edge case tests: verify graceful handling of bad input,
//! malformed source, unicode, and boundary conditions.

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{context, impact, query};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn empty_crate() -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"empty_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"empty_crate\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_dir.join("lib.rs"), "").unwrap();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();
    (dir, db)
}

fn index_source(lib_rs: &str) -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test_crate\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_dir.join("lib.rs"), lib_rs).unwrap();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();
    (dir, db)
}

fn index_multi_file(files: &[(&str, &str)]) -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test_crate\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for (path, content) in files {
        let full = dir.path().join("src").join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();
    (dir, db)
}

// =========================================================================
// 1. EMPTY CRATE — no symbols exist
// =========================================================================

#[test]
fn query_on_empty_crate_returns_no_results() {
    let (_dir, db) = empty_crate();
    let result = query::handle_query(
        &db,
        "anything",
        Some("symbols"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let has_no_symbols = result.contains("No symbols")
        || result.contains("No results")
        || !result.contains("(function)")
            && !result.contains("(struct)")
            && !result.contains("(trait)")
            && !result.contains("(enum)");
    assert!(
        has_no_symbols,
        "empty crate should have no symbol results: {result}"
    );
}

#[test]
fn context_on_nonexistent_symbol_returns_not_found() {
    let (_dir, db) = empty_crate();
    let result =
        context::handle_context(&db, "Nonexistent", false, None, None, None, false).unwrap();
    let indicates_missing = result.contains("not found")
        || result.contains("No symbol")
        || result.contains("no symbol");
    assert!(
        indicates_missing,
        "context for nonexistent symbol should indicate not found: {result}"
    );
}

#[test]
fn impact_on_nonexistent_symbol_does_not_panic() {
    let (_dir, db) = empty_crate();
    let result = impact::handle_impact(&db, "Nonexistent", None, false).unwrap();
    assert!(
        !result.is_empty(),
        "impact on nonexistent symbol should return non-empty result"
    );
}

// =========================================================================
// 2. MALFORMED SOURCE — tree-sitter is error-tolerant
// =========================================================================

#[test]
fn malformed_rust_source_does_not_crash() {
    let (_dir, _db) = index_source("pub fn broken( { }}}}} struct @@@\n");
    // Reaching this point means indexing did not panic
}

// =========================================================================
// 3. UNICODE — doc comments with non-ASCII characters
// =========================================================================

#[test]
fn unicode_in_doc_comments_preserved() {
    let (_dir, db) = index_source(
        "/// H\u{00e9}llo w\u{00f6}rld \u{2014} docs with \u{00fc}\u{00f1}\u{00ed}c\u{00f6}d\u{00e9}\npub fn greet() {}\n",
    );
    let result = context::handle_context(&db, "greet", false, None, None, None, false).unwrap();
    assert!(
        result.contains("H\u{00e9}llo"),
        "unicode 'Héllo' should be preserved: {result}"
    );
    assert!(
        result.contains("w\u{00f6}rld"),
        "unicode 'wörld' should be preserved: {result}"
    );
    assert!(
        result.contains("\u{2014}"),
        "em dash should be preserved: {result}"
    );
}

// =========================================================================
// 4. DEEP NESTING — deeply nested module files
// =========================================================================

#[test]
fn deeply_nested_module_files_indexed() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod a;\n"),
        ("a/mod.rs", "pub mod b;\n"),
        ("a/b/mod.rs", "pub mod c;\n"),
        ("a/b/c/mod.rs", "pub fn deep_fn() {}\n"),
    ]);
    let syms = db.search_symbols("deep_fn").unwrap();
    assert!(!syms.is_empty(), "deeply nested deep_fn should be indexed");
    assert_eq!(syms[0].name, "deep_fn");
}
