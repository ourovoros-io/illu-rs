#![expect(clippy::unwrap_used, reason = "integration tests")]

//! Self-indexing tests: index illu-rs's own source and validate results.
//! The ultimate correctness test — if we can't index ourselves accurately,
//! we can't index anything.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use illu_rs::db::Database;
use illu_rs::indexer::parser::{SymbolKind, Visibility};
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{
    context, doc_coverage, graph_export, impact, orphaned, overview, query, references, test_impact,
};

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
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = query::handle_query(
        &db,
        "Database",
        Some("symbols"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
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
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = query::handle_query(
        &db,
        "index_repo",
        Some("symbols"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
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
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = query::handle_query(
        &db,
        "IlluServer",
        Some("symbols"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
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
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = context::handle_context(&db, "Database", false, None, None, None, false).unwrap();
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
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result =
        context::handle_context(&db, "parse_rust_source", false, None, None, None, false).unwrap();
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
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result =
        context::handle_context(&db, "handle_query", false, None, None, None, false).unwrap();
    assert!(
        result.contains("format_symbols") || result.contains("format_docs"),
        "handle_query should show callees like format_symbols or format_docs: {result}"
    );
}

// =========================================================================
// 3. IMPACT ANALYSIS
// =========================================================================

#[test]
fn self_impact_database_is_widely_used() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = impact::handle_impact(&db, "Database", None, false, false).unwrap();
    let file_count = result.matches("src/").count();
    assert!(
        file_count >= 3,
        "Database should be referenced in >=3 file locations, got {file_count}: {result}"
    );
}

#[test]
fn self_impact_symbol_struct_has_dependents() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = impact::handle_impact(&db, "Symbol", None, false, false).unwrap();
    assert!(
        result.contains("store") || result.contains("parser"),
        "Symbol should have dependents in store or parser: {result}"
    );
}

// =========================================================================
// 4. OVERVIEW AND CROSS-TOOL CONSISTENCY
// =========================================================================

#[test]
fn self_overview_lists_known_public_api() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = overview::handle_overview(&db, "src/", false, None).unwrap();
    for name in &["Database", "index_repo", "IlluServer", "parse_rust_source"] {
        assert!(
            result.contains(name),
            "overview of src/ should contain {name}: {result}"
        );
    }
}

#[test]
fn self_overview_db_module() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = overview::handle_overview(&db, "src/db.rs", false, None).unwrap();
    for name in &["open", "search_symbols", "insert_file", "impact_dependents"] {
        assert!(
            result.contains(name),
            "overview of src/db.rs should contain {name}: {result}"
        );
    }
}

#[test]
fn self_query_and_context_agree_on_file_path() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let query_result = query::handle_query(
        &db,
        "extract_refs",
        Some("symbols"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let context_result =
        context::handle_context(&db, "extract_refs", false, None, None, None, false).unwrap();
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
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = overview::handle_overview(&db, "src/", false, None).unwrap();
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
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let start = Instant::now();
    let _ = query::handle_query(&db, "Database", None, None, None, None, None, None).unwrap();
    let query_time = start.elapsed();
    assert!(
        query_time.as_millis() < 100,
        "query took {query_time:?}, should be <100ms"
    );

    let start = Instant::now();
    let _ = context::handle_context(&db, "Database", false, None, None, None, false).unwrap();
    let context_time = start.elapsed();
    assert!(
        context_time.as_millis() < 100,
        "context took {context_time:?}, should be <100ms"
    );

    let start = Instant::now();
    let _ = impact::handle_impact(&db, "Database", None, false, false).unwrap();
    let impact_time = start.elapsed();
    assert!(
        impact_time.as_millis() < 100,
        "impact took {impact_time:?}, should be <100ms"
    );

    let start = Instant::now();
    let _ = overview::handle_overview(&db, "src/", false, None).unwrap();
    let overview_time = start.elapsed();
    assert!(
        overview_time.as_millis() < 100,
        "overview took {overview_time:?}, should be <100ms"
    );
}

// =========================================================================
// 6. PROPERTY-BASED INVARIANTS
// =========================================================================

#[test]
fn self_no_symbol_has_inverted_line_range() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let symbols = db.get_symbols_by_path_prefix("src/").unwrap();
    for sym in &symbols {
        assert!(
            sym.line_start <= sym.line_end,
            "symbol '{}' in {} has inverted line range: {} > {}",
            sym.name,
            sym.file_path,
            sym.line_start,
            sym.line_end,
        );
    }
    assert!(
        symbols.len() > 20,
        "expected >20 symbols to check, got {}",
        symbols.len()
    );
}

#[test]
fn self_no_signature_contains_body() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let symbols = db.get_symbols_by_path_prefix("src/").unwrap();
    for sym in symbols.iter().filter(|s| s.kind == SymbolKind::Function) {
        let trimmed = sym.signature.trim();
        let without_trailing_brace = trimmed.strip_suffix('{').unwrap_or(trimmed);
        assert!(
            !without_trailing_brace.contains('{'),
            "function '{}' in {} has '{{' in signature body: {}",
            sym.name,
            sym.file_path,
            sym.signature,
        );
        assert!(
            !sym.signature.contains("let "),
            "function '{}' in {} has 'let ' in signature: {}",
            sym.name,
            sym.file_path,
            sym.signature,
        );
        assert!(
            !sym.signature.contains("println"),
            "function '{}' in {} has 'println' in signature: {}",
            sym.name,
            sym.file_path,
            sym.signature,
        );
    }
}

#[test]
fn self_all_file_paths_exist_on_disk() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let symbols = db.get_symbols_by_path_prefix("src/").unwrap();
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let unique_paths: std::collections::HashSet<&str> =
        symbols.iter().map(|s| s.file_path.as_str()).collect();
    for path in &unique_paths {
        let full = std::path::Path::new(manifest_dir).join(path);
        assert!(
            full.exists(),
            "indexed file path does not exist on disk: {}",
            full.display(),
        );
    }
}

#[test]
fn self_every_query_result_has_valid_context() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let known_symbols = [
        "Database",
        "index_repo",
        "IlluServer",
        "parse_rust_source",
        "Symbol",
        "handle_query",
        "handle_context",
        "handle_impact",
        "StoredSymbol",
        "truncate_at",
    ];
    for name in &known_symbols {
        let result = context::handle_context(&db, name, false, None, None, None, false).unwrap();
        assert!(
            result.contains("## "),
            "context for '{name}' missing header: {result}"
        );
        assert!(
            result.contains("- **File:**"),
            "context for '{name}' missing file location: {result}"
        );
        assert!(
            result.contains("- **Signature:**"),
            "context for '{name}' missing signature: {result}"
        );
    }
}

#[test]
fn self_overview_covers_all_public_functions() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let symbols = db.get_symbols_by_path_prefix("src/").unwrap();
    let public_fns: Vec<&str> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function && s.visibility == Visibility::Public)
        .map(|s| s.name.as_str())
        .collect();
    let overview_output = overview::handle_overview(&db, "src/", false, None).unwrap();
    for name in &public_fns {
        assert!(
            overview_output.contains(name),
            "public function '{name}' missing from overview output"
        );
    }
    assert!(
        public_fns.len() > 10,
        "expected >10 public functions to check, got {}",
        public_fns.len()
    );
}

// =========================================================================
// 8. NEW TOOL INTEGRATION TESTS
// =========================================================================

#[test]
fn self_references_database_shows_refs() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = references::handle_references(&db, "Database", None).unwrap();
    assert!(
        result.contains("## References: Database"),
        "header: {result}"
    );
    assert!(
        result.contains("### Definition"),
        "definition section: {result}"
    );
    assert!(
        result.contains("### Call Sites"),
        "call sites section: {result}"
    );
}

#[test]
fn self_doc_coverage_finds_undocumented() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = doc_coverage::handle_doc_coverage(&db, Some("src/"), None, false).unwrap();
    assert!(result.contains("## Doc Coverage"), "header: {result}");
    assert!(result.contains("Coverage:"), "coverage stats: {result}");
}

#[test]
fn self_test_impact_database_shows_tests() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = test_impact::handle_test_impact(&db, "Database").unwrap();
    // Database is widely tested
    assert!(result.contains("## Test Impact:"), "header: {result}");
}

#[test]
fn self_orphaned_returns_results() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = orphaned::handle_orphaned(&db, Some("src/"), None).unwrap();
    assert!(result.contains("## Orphaned Symbols"), "header: {result}");
}

#[test]
fn self_graph_export_produces_dot() {
    let db = self_db()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = graph_export::handle_graph_export(&db, Some("Database"), None, Some(1)).unwrap();
    assert!(
        result.contains("digraph"),
        "should produce DOT format: {result}"
    );
}
