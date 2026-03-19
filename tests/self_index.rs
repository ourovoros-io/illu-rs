#![expect(clippy::unwrap_used, reason = "integration tests")]

//! Self-indexing tests: index illu-rs's own source and validate results.
//! The ultimate correctness test — if we can't index ourselves accurately,
//! we can't index anything.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{context, impact, overview, query};

// ---------------------------------------------------------------------------
// Shared setup — index illu-rs once, reuse across all tests
// ---------------------------------------------------------------------------

fn self_db() -> &'static Mutex<Database> {
    static DB: OnceLock<Mutex<Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        };
        index_repo(&db, &config).unwrap();
        Mutex::new(db)
    })
}

// =========================================================================
// 1. SMOKE TESTS — query finds known symbols
// =========================================================================

#[test]
fn self_index_finds_database_struct() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = query::handle_query(&db, "Database", Some("symbols"), None).unwrap();
    assert!(
        result.contains("Database"),
        "query should find Database: {result}"
    );
    assert!(
        result.contains("src/db.rs"),
        "Database should be in src/db.rs: {result}"
    );
}

#[test]
fn self_index_finds_index_repo_function() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = query::handle_query(&db, "index_repo", Some("symbols"), None).unwrap();
    assert!(
        result.contains("index_repo"),
        "query should find index_repo: {result}"
    );
    assert!(
        result.contains("(function)"),
        "index_repo should be a function: {result}"
    );
}

#[test]
fn self_index_finds_illu_server() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = query::handle_query(&db, "IlluServer", Some("symbols"), None).unwrap();
    assert!(
        result.contains("IlluServer"),
        "query should find IlluServer: {result}"
    );
    assert!(
        result.contains("(struct)"),
        "IlluServer should be a struct: {result}"
    );
}

// =========================================================================
// 2. CONTEXT CORRECTNESS
// =========================================================================

#[test]
fn self_context_database_has_source() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = context::handle_context(&db, "Database", false).unwrap();
    assert!(
        result.contains("pub struct Database"),
        "context should show pub struct Database: {result}"
    );
    assert!(
        result.contains("src/db.rs"),
        "context should show src/db.rs: {result}"
    );
}

#[test]
fn self_context_parse_rust_source_has_signature() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = context::handle_context(&db, "parse_rust_source", false).unwrap();
    assert!(
        result.contains("pub fn parse_rust_source"),
        "context should show pub fn parse_rust_source: {result}"
    );
    assert!(
        result.contains("Symbol"),
        "parse_rust_source context should mention Symbol: {result}"
    );
}

#[test]
fn self_context_handle_query_shows_callees() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = context::handle_context(&db, "handle_query", false).unwrap();
    assert!(
        result.contains("format_symbols") || result.contains("format_docs"),
        "handle_query should call format_symbols or format_docs: {result}"
    );
}

// =========================================================================
// 3. IMPACT ANALYSIS
// =========================================================================

#[test]
fn self_impact_database_is_widely_used() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = impact::handle_impact(&db, "Database").unwrap();
    let file_count = result.matches("src/").count();
    assert!(
        file_count >= 3,
        "Database should be referenced in >=3 file locations, got {file_count}: {result}"
    );
}

#[test]
fn self_impact_symbol_struct_has_dependents() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = impact::handle_impact(&db, "Symbol").unwrap();
    assert!(
        result.contains("store") || result.contains("parser") || result.contains("index"),
        "Symbol should have dependents in store or parser: {result}"
    );
}

// =========================================================================
// 4. OVERVIEW AND CROSS-TOOL CONSISTENCY
// =========================================================================

#[test]
fn self_overview_lists_known_public_api() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = overview::handle_overview(&db, "src/").unwrap();
    for name in &["Database", "index_repo", "IlluServer", "parse_rust_source"] {
        assert!(
            result.contains(name),
            "overview of src/ should contain {name}: {result}"
        );
    }
}

#[test]
fn self_overview_db_module() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = overview::handle_overview(&db, "src/db.rs").unwrap();
    for name in &["open", "search_symbols", "insert_file", "impact_dependents"] {
        assert!(
            result.contains(name),
            "overview of src/db.rs should contain {name}: {result}"
        );
    }
}

#[test]
fn self_query_and_context_agree_on_file_path() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let query_result =
        query::handle_query(&db, "extract_refs", Some("symbols"), None).unwrap();
    let context_result = context::handle_context(&db, "extract_refs", false).unwrap();
    assert!(
        query_result.contains("parser.rs"),
        "query should reference parser.rs: {query_result}"
    );
    assert!(
        context_result.contains("parser.rs"),
        "context should reference parser.rs: {context_result}"
    );
}

#[test]
fn self_index_symbol_count_sanity() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = overview::handle_overview(&db, "src/").unwrap();
    let line_count = result.lines().count();
    assert!(
        line_count > 50,
        "overview of src/ should have >50 lines of output, got {line_count}"
    );
}

// =========================================================================
// 5. TIMING GUARDRAILS
// =========================================================================

#[test]
fn self_index_completes_under_5_seconds() {
    let start = Instant::now();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    };
    index_repo(&db, &config).unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "indexing illu-rs should take <5s, took {elapsed:?}"
    );
}

#[test]
fn tool_queries_complete_under_100ms() {
    let db = self_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    let start = Instant::now();
    let _ = query::handle_query(&db, "Database", None, None).unwrap();
    let query_time = start.elapsed();
    assert!(
        query_time.as_millis() < 100,
        "query took {query_time:?}, should be <100ms"
    );

    let start = Instant::now();
    let _ = context::handle_context(&db, "Database", false).unwrap();
    let context_time = start.elapsed();
    assert!(
        context_time.as_millis() < 100,
        "context took {context_time:?}, should be <100ms"
    );

    let start = Instant::now();
    let _ = impact::handle_impact(&db, "Database").unwrap();
    let impact_time = start.elapsed();
    assert!(
        impact_time.as_millis() < 100,
        "impact took {impact_time:?}, should be <100ms"
    );

    let start = Instant::now();
    let _ = overview::handle_overview(&db, "src/").unwrap();
    let overview_time = start.elapsed();
    assert!(
        overview_time.as_millis() < 100,
        "overview took {overview_time:?}, should be <100ms"
    );
}
