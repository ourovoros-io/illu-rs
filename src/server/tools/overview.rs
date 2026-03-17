use crate::db::Database;
use std::fmt::Write;

pub fn handle_overview(db: &Database, path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.get_symbols_by_path_prefix(path)?;

    if symbols.is_empty() {
        return Ok(format!("No public symbols found under '{path}'."));
    }

    let mut output = String::new();
    let mut current_file = "";

    for sym in &symbols {
        if sym.file_path != current_file {
            current_file = &sym.file_path;
            let _ = writeln!(output, "### {current_file}\n");
        }

        let _ = write!(
            output,
            "- **{}** ({}) `{}`",
            sym.name, sym.kind, sym.signature
        );

        if let Some(doc) = &sym.doc_comment
            && let Some(first_line) = doc.lines().next()
        {
            let _ = write!(output, " — *{first_line}*");
        }

        let _ = writeln!(output);
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    #[test]
    fn test_overview_groups_by_file() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/server/mod.rs", "h1").unwrap();
        let f2 = db.insert_file("src/server/tools.rs", "h2").unwrap();
        store_symbols(
            &db,
            f1,
            &[Symbol {
                name: "serve".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/server/mod.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn serve()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
            }],
        )
        .unwrap();
        store_symbols(
            &db,
            f2,
            &[Symbol {
                name: "handle".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/server/tools.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn handle()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
            }],
        )
        .unwrap();

        let result = handle_overview(&db, "src/server/").unwrap();
        assert!(result.contains("### src/server/mod.rs"));
        assert!(result.contains("### src/server/tools.rs"));
        assert!(result.contains("**serve**"));
        assert!(result.contains("**handle**"));
    }

    #[test]
    fn test_overview_filters_private() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "h1").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "public_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn public_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                },
                Symbol {
                    name: "private_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 12,
                    signature: "fn private_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                },
            ],
        )
        .unwrap();

        let result = handle_overview(&db, "src/").unwrap();
        assert!(result.contains("**public_fn**"));
        assert!(!result.contains("private_fn"));
    }

    #[test]
    fn test_overview_no_results() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_overview(&db, "nonexistent/").unwrap();
        assert_eq!(result, "No public symbols found under 'nonexistent/'.");
    }
}
