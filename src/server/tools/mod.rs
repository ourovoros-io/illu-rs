pub mod batch_context;
pub mod blame;
pub mod boundary;
pub mod callpath;
pub mod context;
pub mod crate_graph;
pub mod crate_impact;
pub mod cross_callpath;
pub mod cross_deps;
pub mod cross_impact;
pub mod cross_query;
pub mod diff_impact;
pub mod doc_coverage;
pub mod docs;
pub mod file_graph;
pub mod freshness;
pub mod graph_export;
pub mod health;
pub mod history;
pub mod hotspots;
pub mod impact;
pub mod implements;
pub mod neighborhood;
pub mod orphaned;
pub mod overview;
pub mod query;
pub mod references;
pub mod rename_plan;
pub mod repos;
pub mod similar;
pub mod stats;
pub mod symbols_at;
pub mod test_impact;
pub mod tree;
pub mod type_usage;
pub mod unused;

pub(crate) use crate::truncate_at as truncate_snippet;

use crate::db::{Database, StoredSymbol, TestEntry};
use crate::indexer::parser::SymbolKind;
use rmcp::schemars;
use schemars::JsonSchema;
use serde::Deserialize;

/// Direction for graph traversal in neighborhood and `graph_export` tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Both,
    Down,
    Up,
}

/// Display format for neighborhood tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NeighborhoodFormat {
    List,
    Tree,
}

/// Output format for `graph_export` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Dot,
    Edges,
    Summary,
}

/// Scope for query tool search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum QueryScope {
    Symbols,
    All,
    Docs,
    Files,
    Bodies,
    Strings,
    #[serde(rename = "doc_comments")]
    DocComments,
}

/// Resolve a symbol name supporting `Type::method` syntax.
/// Falls back to plain `search_symbols` if `::` lookup yields nothing.
pub(crate) fn resolve_symbol(
    db: &Database,
    name: &str,
) -> Result<Vec<StoredSymbol>, Box<dyn std::error::Error>> {
    // 1. Try Type::method qualified lookup
    if let Some((impl_type, method)) = name.split_once("::") {
        let results = db.search_symbols_by_impl(impl_type, method)?;
        if !results.is_empty() {
            return Ok(results);
        }
    }

    // 2. Try exact name match
    let exact = db.search_symbols_exact(name)?;
    if !exact.is_empty() {
        return Ok(exact);
    }

    // 3. Fall back to FTS/fuzzy
    Ok(db.search_symbols(name)?)
}

/// Simple Levenshtein edit distance (no external dependency).
fn levenshtein(a: &str, b: &str) -> usize {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(!ca.eq_ignore_ascii_case(&cb));
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Fuzzy-match a query against all symbol names using edit distance.
/// Returns top 3 matches within a reasonable distance threshold.
fn levenshtein_suggestions(db: &Database, query: &str) -> Vec<(String, Option<String>)> {
    let all_names = db.get_all_distinct_symbol_names().unwrap_or_default();
    let max_dist = (query.len() * 2 / 5).max(2);
    let mut scored: Vec<(usize, String, Option<String>)> = all_names
        .into_iter()
        .map(|(name, impl_type)| {
            let qname = format_qualified(&name, impl_type.as_deref());
            let dist = levenshtein(query, &qname);
            (dist, name, impl_type)
        })
        .filter(|(dist, _, _)| *dist <= max_dist)
        .collect();
    scored.sort_by_key(|(dist, _, _)| *dist);
    scored.truncate(3);
    scored
        .into_iter()
        .map(|(_, name, impl_type)| (name, impl_type))
        .collect()
}

/// "Symbol not found" message with fuzzy "did you mean?" suggestions.
pub(crate) fn symbol_not_found(db: &Database, name: &str) -> String {
    // Reuse resolve_symbol which already tries Type::method, exact, and FTS
    let mut suggestions = resolve_symbol(db, name).unwrap_or_default();

    // For Type::method names where resolve found nothing, try the method part
    if suggestions.is_empty()
        && let Some((_, method)) = name.split_once("::")
    {
        suggestions = resolve_symbol(db, method).unwrap_or_default();
    }

    // Deduplicate by qualified name, take top 3
    let mut seen = std::collections::HashSet::new();
    let suggestions: Vec<_> = suggestions
        .into_iter()
        .filter(|s| {
            let key = qualified_name(s);
            seen.insert(key)
        })
        .take(3)
        .collect();

    if !suggestions.is_empty() {
        use std::fmt::Write;
        let mut out = format!("No symbol found matching '{name}'.\n\nDid you mean:\n");
        for s in &suggestions {
            let qname = qualified_name(s);
            let _ = writeln!(
                out,
                "- `{qname}` ({} at {}:{})",
                s.kind, s.file_path, s.line_start
            );
        }
        let _ = write!(out, "\nUse `query` to search more broadly.");
        return out;
    }

    // FTS returned nothing — fall back to Levenshtein edit distance
    let lev_matches = levenshtein_suggestions(db, name);
    if !lev_matches.is_empty() {
        use std::fmt::Write;
        let mut out = format!("No symbol found matching '{name}'.\n\nDid you mean:\n",);
        for (sym_name, impl_type) in &lev_matches {
            let qname = format_qualified(sym_name, impl_type.as_deref());
            let _ = writeln!(out, "- `{qname}`");
        }
        let _ = write!(out, "\nUse `query` to search more broadly.");
        return out;
    }

    format!(
        "No symbol found matching '{name}'.\n\
         Try `Type::method` syntax for methods \
         (e.g. `Database::new`), or use `query` to search."
    )
}

/// Extract the base (method) name from a possibly qualified symbol name.
pub(crate) fn base_name(name: &str) -> &str {
    name.split_once("::").map_or(name, |(_, m)| m)
}

/// Format a qualified name from parts.
pub(crate) fn format_qualified(name: &str, impl_type: Option<&str>) -> String {
    match impl_type {
        Some(it) => format!("{it}::{name}"),
        None => name.to_string(),
    }
}

/// Format a symbol's qualified name (e.g. `Database::open` for methods).
pub(crate) fn qualified_name(sym: &StoredSymbol) -> String {
    format_qualified(&sym.name, sym.impl_type.as_deref())
}

/// Abbreviate a full commit hash to 7 characters for display.
#[must_use]
pub(crate) fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(7)]
}

/// Check if a `SymbolKind` matches a user-provided kind filter string.
pub(crate) fn kind_matches(kind: &SymbolKind, filter: &str) -> bool {
    kind.to_string().eq_ignore_ascii_case(filter)
}

pub(crate) const NOISY_CALLEES: &[&str] = &[
    // Constructors / conversions
    "new",
    "from",
    "into",
    "default",
    "clone",
    "build",
    "init",
    // Formatting
    "fmt",
    "write",
    "writeln",
    "display",
    "format",
    // Collection methods
    "push",
    "pop",
    "insert",
    "remove",
    "get",
    "set",
    "clear",
    "contains",
    "extend",
    "with_capacity",
    "capacity",
    "retain",
    // Iterator methods
    "iter",
    "into_iter",
    "collect",
    "map",
    "filter",
    // Size / emptiness
    "len",
    "is_empty",
    // String conversions
    "to_string",
    "to_owned",
    "as_str",
    "as_ref",
    "as_mut",
    // Error handling
    "unwrap",
    "expect",
    "ok",
    "err",
    // Common accessors
    "borrow",
    "borrow_mut",
    "deref",
    "deref_mut",
];

const MAX_CARGO_TEST_NAMES: usize = 20;
const TEST_LIST_GROUP_THRESHOLD: usize = 20;
const TEST_LIST_SUMMARY_THRESHOLD: usize = 50;

pub(crate) fn format_cargo_test_suggestion(test_names: &[&str]) -> String {
    if test_names.len() <= MAX_CARGO_TEST_NAMES {
        format!("cargo test {}", test_names.join(" "))
    } else {
        format!(
            "cargo test  # {} tests affected, run full suite",
            test_names.len()
        )
    }
}

/// Render a test list with smart tiering to control output size.
///
/// - ≤20: list each test individually
/// - 21–50: group by file, show individual names
/// - >50: group by file with counts only
pub(crate) fn render_test_list(output: &mut String, tests: &[&TestEntry]) {
    use std::fmt::Write;

    let count = tests.len();
    if count == 0 {
        return;
    }

    if count <= TEST_LIST_GROUP_THRESHOLD {
        for t in tests {
            let _ = writeln!(
                output,
                "- **{}** ({}:{})",
                t.name, t.file_path, t.line_start
            );
        }
        return;
    }

    // Group tests by file
    let mut by_file: std::collections::BTreeMap<&str, Vec<&TestEntry>> =
        std::collections::BTreeMap::new();
    for t in tests {
        by_file.entry(&t.file_path).or_default().push(t);
    }

    if count <= TEST_LIST_SUMMARY_THRESHOLD {
        for (file, file_tests) in &by_file {
            let _ = writeln!(output, "**{file}** ({} tests)", file_tests.len());
            for t in file_tests {
                let _ = writeln!(output, "- {} (line {})", t.name, t.line_start);
            }
            output.push('\n');
        }
    } else {
        let _ = writeln!(output, "{count} tests across {} files:\n", by_file.len());
        for (file, file_tests) in &by_file {
            let _ = writeln!(output, "- **{file}** — {} tests", file_tests.len());
        }
        let _ = writeln!(
            output,
            "\n*Use `test_impact` with `depth: 1` for a focused list.*"
        );
    }
}

/// Check if a symbol is an entry point (main, #[test]) that should
/// be excluded from unused/untested reports.
pub(crate) fn is_entry_point(sym: &StoredSymbol) -> bool {
    if sym.name == "main" {
        return true;
    }
    sym.attributes
        .as_deref()
        .is_some_and(crate::indexer::store::is_test_attribute)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    fn make_test(name: &str, file: &str, line: i64) -> TestEntry {
        TestEntry {
            name: name.to_string(),
            file_path: file.to_string(),
            line_start: line,
        }
    }

    fn make_tests(count: usize) -> Vec<TestEntry> {
        let files = ["src/db.rs", "src/server.rs", "tests/integration.rs"];
        let mut tests = Vec::new();
        for i in 0..count {
            let file = files[i % files.len()];
            let line = i64::try_from(i).unwrap_or(0) * 10 + 1;
            tests.push(make_test(&format!("test_{i}"), file, line));
        }
        tests
    }

    #[test]
    fn test_render_test_list_small() {
        let entries = make_tests(5);
        let refs: Vec<&TestEntry> = entries.iter().collect();
        let mut output = String::new();
        render_test_list(&mut output, &refs);
        // Individual listing, each with bold name and file:line
        assert_eq!(output.matches("- **test_").count(), 5);
        assert!(output.contains("src/db.rs:"));
    }

    #[test]
    fn test_render_test_list_medium() {
        let entries = make_tests(30);
        let refs: Vec<&TestEntry> = entries.iter().collect();
        let mut output = String::new();
        render_test_list(&mut output, &refs);
        // Grouped by file, individual names still shown
        assert!(output.contains("tests)"));
        assert!(output.contains("test_0"));
        assert!(output.contains("test_29"));
    }

    #[test]
    fn test_render_test_list_large() {
        let entries = make_tests(100);
        let refs: Vec<&TestEntry> = entries.iter().collect();
        let mut output = String::new();
        render_test_list(&mut output, &refs);
        // Summary only: file counts, no individual names
        assert!(output.contains("100 tests across 3 files"));
        assert!(output.contains("tests"));
        // Individual test names should NOT appear
        assert!(!output.contains("test_0 ("));
        assert!(output.contains("depth: 1"));
    }

    #[test]
    fn test_render_test_list_empty() {
        let mut output = String::new();
        render_test_list(&mut output, &[]);
        assert!(output.is_empty());
    }

    #[test]
    fn test_symbol_not_found_suggests_similar() {
        use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
        use crate::indexer::store::store_symbols;

        let db = Database::open_in_memory().unwrap();
        let fid = db.insert_file("src/lib.rs", "hash1").unwrap();
        store_symbols(
            &db,
            fid,
            &[Symbol {
                name: "resolve_symbol".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 10,
                line_end: 20,
                signature: "fn resolve_symbol()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let result = symbol_not_found(&db, "resolve_sym");
        assert!(result.contains("Did you mean"), "got: {result}");
        assert!(result.contains("resolve_symbol"), "got: {result}");
    }

    #[test]
    fn test_symbol_not_found_type_method_fallback() {
        use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
        use crate::indexer::store::store_symbols;

        let db = Database::open_in_memory().unwrap();
        let fid = db.insert_file("src/db.rs", "hash1").unwrap();
        store_symbols(
            &db,
            fid,
            &[Symbol {
                name: "resolve".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/db.rs".into(),
                line_start: 5,
                line_end: 15,
                signature: "fn resolve()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("SymbolIdMap".into()),
            }],
        )
        .unwrap();

        // Full qualified name doesn't match, but method part "resolve" does
        let result = symbol_not_found(&db, "Database::resolve");
        assert!(result.contains("Did you mean"), "got: {result}");
        assert!(result.contains("resolve"), "got: {result}");
    }

    #[test]
    fn test_symbol_not_found_no_suggestions() {
        let db = Database::open_in_memory().unwrap();
        let result = symbol_not_found(&db, "zzz_nonexistent_zzz");
        assert!(result.contains("No symbol found"));
        assert!(!result.contains("Did you mean"));
    }

    #[test]
    fn test_symbol_not_found_typo_suggests_levenshtein() {
        use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
        use crate::indexer::store::store_symbols;
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "h1").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "open_database".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "fn open_database()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();
        let result = symbol_not_found(&db, "open_databse");
        assert!(
            result.contains("open_database"),
            "Should suggest 'open_database' for typo 'open_databse', \
             got: {result}"
        );
    }

    #[test]
    fn test_symbol_not_found_typo_qualified() {
        use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
        use crate::indexer::store::store_symbols;
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/db.rs", "h1").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "open".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/db.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "fn open()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("Database".into()),
            }],
        )
        .unwrap();
        let result = symbol_not_found(&db, "Databse::opn");
        assert!(
            result.contains("Database::open"),
            "Should suggest 'Database::open' for typo 'Databse::opn', \
             got: {result}"
        );
    }
}
