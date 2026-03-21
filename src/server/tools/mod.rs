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

use crate::db::{Database, StoredSymbol};

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

/// Format a symbol's qualified name (e.g. `Database::open` for methods).
pub(crate) fn qualified_name(sym: &StoredSymbol) -> String {
    if let Some(impl_type) = &sym.impl_type {
        format!("{impl_type}::{}", sym.name)
    } else {
        sym.name.clone()
    }
}

pub(crate) const NOISY_CALLEES: &[&str] = &[
    "new",
    "from",
    "into",
    "default",
    "clone",
    "build",
    "init",
    "fmt",
    "write",
    "writeln",
    "push",
    "len",
    "is_empty",
    "to_string",
    "to_owned",
    "as_str",
    "as_ref",
    "iter",
    "collect",
    "map",
    "filter",
];

const MAX_CARGO_TEST_NAMES: usize = 20;

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

/// Check if a symbol is an entry point (main, #[test]) that should
/// be excluded from unused/untested reports.
pub(crate) fn is_entry_point(sym: &StoredSymbol) -> bool {
    if sym.name == "main" {
        return true;
    }
    sym.attributes
        .as_deref()
        .is_some_and(|a| a.contains("test"))
}
