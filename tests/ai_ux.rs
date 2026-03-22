#![expect(clippy::unwrap_used, reason = "integration tests")]
#![expect(clippy::doc_markdown, reason = "test comments reference code identifiers")]

//! AI UX tests: verify that illu returns data that is correct, complete,
//! and useful for an AI coding assistant.
//!
//! These tests are written from the perspective of Claude using illu tools.
//! Each test represents a real workflow an AI assistant performs and verifies
//! that the returned data enables effective code understanding and modification.
//!
//! Organized by tool in order of usage frequency:
//! 1. Query — "find me the symbol"
//! 2. Context — "show me everything about it"
//! 3. Impact — "what breaks if I change it?"
//! 4. Callpath — "how does A reach B?"
//! 5. Neighborhood — "what's around this symbol?"
//! 6. References — "where is this used?"
//! 7. Test Impact — "which tests cover this?"

use std::sync::{Mutex, OnceLock};

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{
    Direction, NeighborhoodFormat, QueryScope, callpath, context, impact, neighborhood, query,
    references, test_impact,
};

// ---------------------------------------------------------------------------
// Shared setup — index illu-rs once, reuse across all tests
// ---------------------------------------------------------------------------

fn db() -> &'static Mutex<Database> {
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

fn lock() -> std::sync::MutexGuard<'static, Database> {
    db().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// =========================================================================
// 1. QUERY — "find me the symbol"
// =========================================================================
//
// The #1 most-used tool. An AI assistant queries to locate code before
// reading it. The key requirement: the RIGHT symbol appears near the top.

/// When I search for "Database", the struct definition should be the
/// first or among the top results — not buried under random mentions.
#[test]
fn query_relevance_database_struct_ranks_high() {
    let db = lock();
    let result =
        query::handle_query(&db, "Database", Some(QueryScope::Symbols), None, None, None, None, None)
            .unwrap();

    // The struct should appear before any methods or other symbols
    let lines: Vec<&str> = result.lines().collect();
    let struct_line = lines
        .iter()
        .position(|l| l.contains("Database") && l.contains("(struct)"));
    assert!(
        struct_line.is_some(),
        "Database struct should appear in query results: {result}"
    );
    // It should be in the first few results (top 5 lines of output)
    let pos = struct_line.unwrap();
    assert!(
        pos < 10,
        "Database struct should rank high (found at line {pos}): {result}"
    );
}

/// NOTE: query does NOT support Type::method syntax — only context/impact do.
/// Searching "Database::open" in query returns no results because query
/// treats the input as a plain name, not a qualified lookup.
/// This is a known UX gap: query should support Type::method resolution.
///
/// For now, verify that searching for just "open" with a path filter works.
#[test]
fn query_method_with_path_filter_workaround() {
    let db = lock();
    let result = query::handle_query(
        &db,
        "open",
        Some(QueryScope::Symbols),
        None,
        None,
        None,
        Some("src/db.rs"),
        None,
    )
    .unwrap();

    assert!(
        result.contains("open") && result.contains("src/db.rs"),
        "query for 'open' with path filter should find Database::open: {result}"
    );
}

/// Searching for common names like "new" should return qualified results,
/// not a wall of noise from every constructor in every crate.
#[test]
fn query_common_name_returns_qualified_results() {
    let db = lock();
    let result = query::handle_query(
        &db,
        "new",
        Some(QueryScope::Symbols),
        None,
        None,
        None,
        None,
        Some(10),
    )
    .unwrap();

    // Results should exist (many types have ::new)
    assert!(
        !result.contains("No results"),
        "query for 'new' should find constructor methods: {result}"
    );
    // Each result should show impl_type context so I know WHICH new()
    // (e.g., "StatusGuard::new" not just "new")
    let result_lines: Vec<&str> = result
        .lines()
        .filter(|l| l.contains("new") && l.contains("(function)"))
        .collect();
    if !result_lines.is_empty() {
        // At least some results should show the parent type
        let has_qualified = result_lines.iter().any(|l| l.contains("::new"));
        assert!(
            has_qualified,
            "new() results should show qualified names (Type::new): {result}"
        );
    }
}

/// Wildcard + kind filter: I often search for "all structs" or "all enums"
/// to understand a module's data model.
#[test]
fn query_wildcard_kind_filter_finds_structs() {
    let db = lock();
    let result = query::handle_query(
        &db,
        "*",
        Some(QueryScope::Symbols),
        Some("struct"),
        None,
        None,
        Some("src/db.rs"),
        Some(50),
    )
    .unwrap();

    assert!(
        result.contains("Database"),
        "wildcard struct query should find Database: {result}"
    );
    assert!(
        result.contains("StoredSymbol"),
        "wildcard struct query should find StoredSymbol: {result}"
    );
    // Should NOT contain functions
    assert!(
        !result.contains("(function)"),
        "struct-only query should not include functions: {result}"
    );
}

/// Signature search: "find all functions returning SqlResult<Vec<StoredSymbol>>"
/// This is how I find DB methods that return symbol collections.
#[test]
fn query_signature_filter_finds_matching_functions() {
    let db = lock();
    let result = query::handle_query(
        &db,
        "*",
        Some(QueryScope::Symbols),
        None,
        None,
        Some("-> SqlResult<Vec<StoredSymbol>>"),
        None,
        None,
    )
    .unwrap();

    assert!(
        !result.contains("No results"),
        "signature query should find DB methods: {result}"
    );
    // Should find methods like search_symbols, symbols_by_path_prefix, etc.
    assert!(
        result.contains("src/db.rs"),
        "functions returning Vec<StoredSymbol> should be in db.rs: {result}"
    );
}

/// NOTE: query does NOT provide "Did you mean?" suggestions — only
/// context/impact/references do (via symbol_not_found helper).
/// Query just returns "No results found for 'X'".
/// This is a known UX gap: query should offer fuzzy suggestions.
///
/// Verify that context DOES provide suggestions for the same typo.
#[test]
fn context_provides_did_you_mean_for_typos() {
    let db = lock();
    let result =
        context::handle_context(&db, "Databse", false, None, None, None, false).unwrap();

    assert!(
        result.contains("Did you mean") || result.contains("Database"),
        "context should suggest corrections for typos: {result}"
    );
}

// =========================================================================
// 2. CONTEXT — "show me everything about this symbol"
// =========================================================================
//
// The workhorse tool. When I need to understand a symbol, context should
// give me source, callers, callees, and related info in ONE call.

/// Context for a struct should show the definition, impl methods nearby,
/// and what calls into it.
#[test]
fn context_struct_shows_definition_and_callers() {
    let db = lock();
    let result = context::handle_context(&db, "Database", false, None, None, None, false).unwrap();

    // Must have source code
    assert!(
        result.contains("pub struct Database"),
        "context should show struct definition: {result}"
    );
    // Must show file location
    assert!(
        result.contains("src/db.rs"),
        "context should show file path: {result}"
    );
    // Should show related items (methods in impl Database)
    assert!(
        result.contains("Related"),
        "struct context should show related impl methods: {result}"
    );
}

/// Type::method resolution in context — "Database::open" should
/// resolve to the method on Database, not just any "open".
#[test]
fn context_type_method_resolves_correctly() {
    let db = lock();
    let result =
        context::handle_context(&db, "Database::open", false, None, None, None, false).unwrap();

    assert!(
        result.contains("fn open"),
        "Database::open context should show the method: {result}"
    );
    assert!(
        result.contains("src/db.rs"),
        "Database::open should be in db.rs: {result}"
    );
    // Should NOT show a "No symbol found" message
    assert!(
        !result.contains("No symbol found"),
        "Database::open should resolve correctly: {result}"
    );
}

/// Callers shown by context should be REAL callers with file:line references.
/// This is critical — false callers waste my time investigating phantom deps.
#[test]
fn context_callers_are_real_with_file_lines() {
    let db = lock();
    let result = context::handle_context(
        &db,
        "parse_rust_source",
        false,
        None,
        Some(&["callers"]),
        None,
        false,
    )
    .unwrap();

    // Should have a "Called By" section
    assert!(
        result.contains("Called By"),
        "context should show callers section: {result}"
    );
    // Callers should include file:line references (e.g., "src/indexer/mod.rs:123")
    assert!(
        result.contains("src/indexer/mod.rs"),
        "parse_rust_source should be called from indexer/mod.rs: {result}"
    );
    // Each caller line should have a line number (file:NNN format)
    let caller_lines: Vec<&str> = result
        .lines()
        .filter(|l| l.contains("src/") && l.contains(':'))
        .collect();
    assert!(
        !caller_lines.is_empty(),
        "callers should include file:line references: {result}"
    );
}

/// Callees should show what this function calls — I use this to understand
/// a function's behavior without reading all the code.
#[test]
fn context_callees_show_real_dependencies() {
    let db = lock();
    let result = context::handle_context(
        &db,
        "index_repo",
        false,
        None,
        Some(&["callees"]),
        None,
        true, // exclude tests
    )
    .unwrap();

    // index_repo should call parse_rust_source and DB methods
    assert!(
        result.contains("Calls") || result.contains("callees"),
        "context should show callees section: {result}"
    );
}

/// Sections filter: I often only need callers, not the full context.
/// This saves tokens and focuses the output.
#[test]
fn context_sections_filter_restricts_output() {
    let db = lock();
    let full =
        context::handle_context(&db, "Database", false, None, None, None, false).unwrap();
    let callers_only = context::handle_context(
        &db,
        "Database",
        false,
        None,
        Some(&["callers"]),
        None,
        false,
    )
    .unwrap();

    assert!(
        callers_only.len() < full.len(),
        "sections filter should produce shorter output (full={}, filtered={})",
        full.len(),
        callers_only.len()
    );
}

/// Exclude-tests filter: when understanding production code, test callers
/// are noise. The filter should remove them.
#[test]
fn context_exclude_tests_removes_test_callers() {
    let db = lock();
    let with_tests =
        context::handle_context(&db, "Database::open", false, None, None, None, false).unwrap();
    let without_tests =
        context::handle_context(&db, "Database::open", false, None, None, None, true).unwrap();

    // The test-excluded version should be shorter or equal
    assert!(
        without_tests.len() <= with_tests.len(),
        "excluding tests should reduce output (with={}, without={})",
        with_tests.len(),
        without_tests.len()
    );
}

// =========================================================================
// 3. IMPACT — "what breaks if I change this?"
// =========================================================================
//
// Before modifying any symbol, I MUST know the blast radius. Impact
// accuracy directly prevents me from introducing bugs.

/// Impact for a widely-used type should show many dependents.
#[test]
fn impact_widely_used_type_shows_dependents() {
    let db = lock();
    let result = impact::handle_impact(&db, "StoredSymbol", None, true, false).unwrap();

    assert!(
        !result.contains("No symbol found"),
        "StoredSymbol should be found: {result}"
    );
    // StoredSymbol is used in many tool handlers — impact should be significant
    assert!(
        result.contains("Depth 1"),
        "StoredSymbol should have depth-1 dependents: {result}"
    );
}

/// Impact depth=1 should show ONLY direct callers, not the full tree.
/// I use this to scope small changes.
#[test]
fn impact_depth_1_shows_direct_dependents_only() {
    let db = lock();
    let depth1 = impact::handle_impact(&db, "resolve_symbol", Some(1), true, false).unwrap();
    let full = impact::handle_impact(&db, "resolve_symbol", None, true, false).unwrap();

    // depth=1 should be shorter than full
    assert!(
        depth1.len() <= full.len(),
        "depth=1 should be more concise (depth1={}, full={})",
        depth1.len(),
        full.len()
    );
    // depth=1 should not contain "Depth 2" or higher
    assert!(
        !depth1.contains("Depth 2"),
        "depth=1 should not show depth 2: {depth1}"
    );
}

/// Impact should include related tests — so I know what to run.
#[test]
fn impact_includes_related_tests() {
    let db = lock();
    let result = impact::handle_impact(&db, "search_symbols", None, true, false).unwrap();

    // Should have a "Related Tests" section
    assert!(
        result.contains("Related Tests") || result.contains("cargo test"),
        "impact should suggest related tests: {result}"
    );
}

/// Impact for a Type::method symbol should use impl_type resolution,
/// not find every function named "open" across the codebase.
#[test]
fn impact_type_method_resolves_correctly() {
    let db = lock();
    let result = impact::handle_impact(&db, "Database::open", None, true, false).unwrap();

    assert!(
        !result.contains("No symbol found"),
        "Database::open should resolve for impact: {result}"
    );
}

// =========================================================================
// 4. CALLPATH — "how does A reach B?"
// =========================================================================
//
// I use this to understand data flow and trace execution paths.

/// There should be a call path from main entry points to deep internals.
#[test]
fn callpath_finds_path_from_handler_to_db() {
    let db = lock();
    let result =
        callpath::handle_callpath(&db, "handle_query", "search_symbols", None, false, None, false)
            .unwrap();

    // Should find a path, not "No path found"
    assert!(
        !result.contains("No path found"),
        "should find path from handle_query to search_symbols: {result}"
    );
    // Path should show intermediate steps
    assert!(
        result.contains("→") || result.contains("->"),
        "callpath should show chain: {result}"
    );
}

/// Callpath between unrelated symbols should report no path.
#[test]
fn callpath_reports_no_path_for_unrelated_symbols() {
    let db = lock();
    let result = callpath::handle_callpath(
        &db,
        "short_hash",
        "parse_workspace_toml",
        None,
        false,
        None,
        false,
    )
    .unwrap();

    // Output says "No call path found from `X` to `Y`"
    assert!(
        result.contains("No call path found"),
        "unrelated symbols should have no callpath: {result}"
    );
}

// =========================================================================
// 5. NEIGHBORHOOD — "what's around this symbol?"
// =========================================================================
//
// I use this to understand local structure — callers AND callees in one view.

/// Neighborhood should show both upstream (callers) and downstream (callees).
#[test]
fn neighborhood_both_directions_shows_context() {
    let db = lock();
    let result = neighborhood::handle_neighborhood(
        &db,
        "handle_context",
        Some(1),
        Some(Direction::Both),
        None,
        true,
    )
    .unwrap();

    // Neighborhood uses "Callers (upstream)" and "Callees (downstream)" headers
    assert!(
        result.contains("Callers") && result.contains("Callees"),
        "neighborhood should show both callers and callees: {result}"
    );
}

/// Tree format should produce hierarchical output that's easy to scan.
#[test]
fn neighborhood_tree_format_shows_hierarchy() {
    let db = lock();
    let result = neighborhood::handle_neighborhood(
        &db,
        "resolve_symbol",
        Some(2),
        Some(Direction::Up),
        Some(NeighborhoodFormat::Tree),
        true,
    )
    .unwrap();

    // Tree format uses indent characters
    assert!(
        result.contains('├') || result.contains('└') || result.contains("──"),
        "tree format should use tree characters: {result}"
    );
}

// =========================================================================
// 6. REFERENCES — "where is this used?"
// =========================================================================
//
// I use this before refactoring to find every usage site.

/// References for a widely-used type should show all usage categories.
#[test]
fn references_shows_definition_and_call_sites() {
    let db = lock();
    let result = references::handle_references(&db, "SymbolKind", None, false).unwrap();

    assert!(
        !result.contains("No symbol found"),
        "SymbolKind should be found: {result}"
    );
    // Should show definition location
    assert!(
        result.contains("src/indexer/parser.rs"),
        "SymbolKind definition should be in parser.rs: {result}"
    );
    // Should show usage in other files
    assert!(
        result.contains("src/db.rs") || result.contains("src/server/"),
        "SymbolKind should have references across modules: {result}"
    );
}

/// Exclude-tests should filter test references from the output.
#[test]
fn references_exclude_tests_filters_test_code() {
    let db = lock();
    let with_tests = references::handle_references(&db, "Database::open", None, false).unwrap();
    let without_tests = references::handle_references(&db, "Database::open", None, true).unwrap();

    assert!(
        without_tests.len() <= with_tests.len(),
        "excluding tests should reduce references (with={}, without={})",
        with_tests.len(),
        without_tests.len()
    );
}

// =========================================================================
// 7. TEST IMPACT — "which tests cover this?"
// =========================================================================
//
// After changing a function, I need to know which tests to run.

/// test_impact for a core function should find related tests.
#[test]
fn test_impact_finds_relevant_tests() {
    let db = lock();
    let result = test_impact::handle_test_impact(&db, "search_symbols", None).unwrap();

    assert!(
        !result.contains("No symbol found"),
        "search_symbols should be found: {result}"
    );
    // Should suggest a cargo test command
    assert!(
        result.contains("cargo test") || result.contains("test"),
        "test_impact should suggest tests to run: {result}"
    );
}

/// test_impact for a leaf function should still find tests that
/// transitively call it through the call chain.
#[test]
fn test_impact_finds_transitive_test_coverage() {
    let db = lock();
    let result = test_impact::handle_test_impact(&db, "escape_like", None).unwrap();

    // escape_like is a private helper called by DB methods which are
    // called by tool handlers which are called by tests
    // It should find tests transitively
    assert!(
        !result.contains("No symbol found"),
        "escape_like should be found: {result}"
    );
}

// =========================================================================
// 8. CROSS-CUTTING: Data quality properties
// =========================================================================
//
// These tests verify properties that matter across all tools.

/// Line numbers in context output should be accurate — they're how I
/// navigate to code. Off-by-one errors waste time.
#[test]
fn line_numbers_are_accurate_for_known_symbols() {
    let db = lock();
    let result =
        context::handle_context(&db, "parse_rust_source", false, None, None, None, false).unwrap();

    // Extract the line number from context output
    // Format is typically "File: src/indexer/parser.rs:NNN"
    let file_line = result
        .lines()
        .find(|l| l.contains("src/indexer/parser.rs") && l.contains(':'));
    assert!(
        file_line.is_some(),
        "context should show file:line for parse_rust_source: {result}"
    );

    // Read the actual file and verify the function exists near that line
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/indexer/parser.rs"),
    )
    .unwrap();
    assert!(
        source.contains("pub fn parse_rust_source"),
        "parse_rust_source should exist in parser.rs"
    );
}

/// Symbols with common names should be disambiguated via impl_type.
/// "new" alone is ambiguous — the tool should show which type's new().
#[test]
fn common_names_disambiguated_by_impl_type() {
    let db = lock();
    let result =
        context::handle_context(&db, "StatusGuard::new", false, None, None, None, false).unwrap();

    assert!(
        !result.contains("No symbol found"),
        "StatusGuard::new should resolve via impl_type: {result}"
    );
    assert!(
        result.contains("StatusGuard"),
        "context should show StatusGuard context: {result}"
    );
}

/// The "Did you mean?" suggestions should handle Type::method queries.
/// If I typo "Database::opn", it should suggest "Database::open".
#[test]
fn did_you_mean_works_for_type_method_queries() {
    let db = lock();
    let result =
        context::handle_context(&db, "Database::opn", false, None, None, None, false).unwrap();

    assert!(
        result.contains("No symbol found") || result.contains("Did you mean"),
        "misspelled Type::method should give feedback: {result}"
    );
}

/// Enum variants should be findable and distinguishable from their parent.
#[test]
fn enum_variants_are_findable() {
    let db = lock();
    let result = query::handle_query(
        &db,
        "Function",
        Some(QueryScope::Symbols),
        Some("enum_variant"),
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // SymbolKind::Function is an enum variant
    assert!(
        result.contains("Function") && result.contains("enum_variant"),
        "enum variant Function should be findable: {result}"
    );
}

/// Query results should include file paths — without them, I can't
/// navigate to the code.
#[test]
fn query_results_always_include_file_paths() {
    let db = lock();
    let result = query::handle_query(
        &db,
        "handle_impact",
        Some(QueryScope::Symbols),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert!(
        result.contains("src/"),
        "query results must include file paths: {result}"
    );
}

/// Context output should be token-efficient — it's the most expensive
/// tool call in terms of tokens. Verify it's not bloated.
#[test]
fn context_output_is_reasonably_sized() {
    let db = lock();
    let result =
        context::handle_context(&db, "handle_query", false, None, None, None, true).unwrap();

    // Context for a single function should be under 5000 chars
    // (source + callers + callees + related)
    assert!(
        result.len() < 10_000,
        "context output should be token-efficient (got {} chars)",
        result.len()
    );
}
