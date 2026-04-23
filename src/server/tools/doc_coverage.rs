use crate::db::Database;
use crate::indexer::parser::SymbolKind;
use std::fmt::Write;

pub fn handle_doc_coverage(
    db: &Database,
    path: Option<&str>,
    kind: Option<&str>,
    include_private: bool,
) -> Result<String, crate::IlluError> {
    let prefix = path.unwrap_or("");
    let mut symbols = db.get_symbols_by_path_prefix_filtered(prefix, include_private)?;

    // Filter out non-documentable kinds
    symbols.retain(|s| {
        s.kind != SymbolKind::Use
            && s.kind != SymbolKind::Mod
            && s.kind != SymbolKind::Impl
            && s.kind != SymbolKind::EnumVariant
    });

    super::retain_kind(&mut symbols, kind);

    let total = symbols.len();
    let documented: Vec<_> = symbols.iter().filter(|s| s.doc_comment.is_some()).collect();
    let undocumented: Vec<_> = symbols.iter().filter(|s| s.doc_comment.is_none()).collect();

    let mut output = String::new();
    let _ = writeln!(output, "## Doc Coverage\n");

    let pct = (documented.len() * 100).checked_div(total).unwrap_or(0);
    let _ = writeln!(
        output,
        "**Coverage:** {}/{} symbols documented ({}%)\n",
        documented.len(),
        total,
        pct
    );

    if undocumented.is_empty() {
        let _ = writeln!(output, "All symbols have documentation.");
        return Ok(output);
    }

    let _ = writeln!(output, "### Undocumented Symbols\n");

    let mut current_file = String::new();
    for sym in &undocumented {
        if sym.file_path != current_file {
            current_file.clone_from(&sym.file_path);
            let _ = writeln!(output, "#### {current_file}\n");
        }
        let qname = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "- **{qname}** ({}, line {})",
            sym.kind, sym.line_start
        );
    }

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
                name: "documented_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 3,
                signature: "pub fn documented_fn()".into(),
                doc_comment: Some("This is documented.".into()),
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "undocumented_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 5,
                line_end: 7,
                signature: "pub fn undocumented_fn()".into(),
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
    fn test_doc_coverage_shows_stats() {
        let db = setup_db();
        let result = handle_doc_coverage(&db, None, None, false).unwrap();
        assert!(result.contains("1/2 symbols documented (50%)"));
    }

    #[test]
    fn test_doc_coverage_lists_undocumented() {
        let db = setup_db();
        let result = handle_doc_coverage(&db, None, None, false).unwrap();
        assert!(result.contains("undocumented_fn"));
        // "documented_fn" is a substring of "undocumented_fn", so check
        // that it only appears as part of the undocumented name
        assert!(!result.contains("**documented_fn**"));
    }

    #[test]
    fn test_doc_coverage_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_doc_coverage(&db, None, None, false).unwrap();
        assert!(result.contains("0/0"));
    }
}
