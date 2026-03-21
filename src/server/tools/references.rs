use crate::db::Database;
use std::fmt::Write;

pub fn handle_references(
    db: &Database,
    symbol_name: &str,
    path: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(format!(
            "No symbol found matching '{symbol_name}'.\n\
            Try `Type::method` syntax for methods, a partial name, or use `query` to search."
        ));
    }

    let mut output = String::new();
    let _ = writeln!(output, "## References: {symbol_name}\n");

    // Definition(s)
    let _ = writeln!(output, "### Definition\n");
    for sym in &symbols {
        let qname = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "- **{qname}** ({}) at {}:{}-{}",
            sym.kind, sym.file_path, sym.line_start, sym.line_end
        );
    }

    // Call sites (callers)
    let _ = writeln!(output, "\n### Call Sites\n");
    let mut call_count = 0usize;
    for sym in &symbols {
        let callers = db.get_callers(&sym.name, &sym.file_path, false)?;
        for c in &callers {
            if path.is_none() || c.file_path.starts_with(path.unwrap_or("")) {
                let line = c.ref_line.unwrap_or(c.line_start);
                let _ = writeln!(output, "- {} ({}:{})", c.name, c.file_path, line);
                call_count += 1;
            }
        }
    }
    if call_count == 0 {
        let _ = writeln!(output, "No call sites found.");
    }

    // Type usage in signatures
    let _ = writeln!(output, "\n### Type Usage in Signatures\n");
    let base_name = symbol_name.rsplit("::").next().unwrap_or(symbol_name);
    let sig_results = db.search_symbols_by_signature(base_name)?;
    let mut type_count = 0usize;
    for s in &sig_results {
        if s.name != base_name && (path.is_none() || s.file_path.starts_with(path.unwrap_or(""))) {
            let qname = super::qualified_name(s);
            let _ = writeln!(
                output,
                "- **{qname}** — `{}`",
                super::truncate_snippet(&s.signature, 120)
            );
            type_count += 1;
        }
    }
    if type_count == 0 {
        let _ = writeln!(output, "Not used in any signatures.");
    }

    // Trait implementations
    let _ = writeln!(output, "\n### Trait Implementations\n");
    let type_impls = db.get_trait_impls_for_type(base_name)?;
    let trait_impls = db.get_trait_impls_for_trait(base_name)?;
    if type_impls.is_empty() && trait_impls.is_empty() {
        let _ = writeln!(output, "No trait implementations found.");
    }
    for ti in &type_impls {
        let _ = writeln!(
            output,
            "- `{}` implements `{}` ({}:{}-{})",
            ti.type_name, ti.trait_name, ti.file_path, ti.line_start, ti.line_end
        );
    }
    for ti in &trait_impls {
        let _ = writeln!(
            output,
            "- `{}` implements `{}` ({}:{}-{})",
            ti.type_name, ti.trait_name, ti.file_path, ti.line_start, ti.line_end
        );
    }

    // Summary
    let _ = writeln!(
        output,
        "\n---\n**Summary:** {} call site(s), {} signature usage(s), {} trait impl(s)",
        call_count,
        type_count,
        type_impls.len() + trait_impls.len()
    );

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        let symbols = vec![
            Symbol {
                name: "Config".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "load".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn load() -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();
        db
    }

    #[test]
    fn test_references_found() {
        let db = setup_db();
        let result = handle_references(&db, "Config", None).unwrap();
        assert!(result.contains("## References: Config"));
        assert!(result.contains("### Definition"));
        assert!(result.contains("Config"));
    }

    #[test]
    fn test_references_not_found() {
        let db = setup_db();
        let result = handle_references(&db, "Nonexistent", None).unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_references_type_usage() {
        let db = setup_db();
        let result = handle_references(&db, "Config", None).unwrap();
        assert!(result.contains("Type Usage"));
        assert!(result.contains("load"));
    }
}
