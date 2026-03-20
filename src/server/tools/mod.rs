pub mod batch_context;
pub mod callpath;
pub mod context;
pub mod crate_graph;
pub mod diff_impact;
pub mod freshness;
pub mod implements;
pub mod neighborhood;
pub mod docs;
pub mod impact;
pub mod overview;
pub mod query;
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
    if let Some((impl_type, method)) = name.split_once("::") {
        let results = db.search_symbols_by_impl(impl_type, method)?;
        if !results.is_empty() {
            return Ok(results);
        }
    }
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
