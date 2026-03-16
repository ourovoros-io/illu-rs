use crate::db::Database;
use std::fmt::Write;

pub fn handle_context(
    db: &Database,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.search_symbols(symbol_name)?;
    if symbols.is_empty() {
        return Ok(format!(
            "No symbol found matching '{symbol_name}'."
        ));
    }

    let mut output = String::new();

    for sym in &symbols {
        let _ = writeln!(output, "## {} ({})", sym.name, sym.kind);
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "- **File:** {}:{}-{}",
            sym.file_path, sym.line_start, sym.line_end
        );
        let _ = writeln!(
            output,
            "- **Visibility:** {}",
            sym.visibility
        );
        let _ = writeln!(
            output,
            "- **Signature:** `{}`",
            sym.signature
        );
        let _ = writeln!(output);
    }

    // Check if any dependencies have related docs
    let docs = db.search_docs(symbol_name)?;
    if !docs.is_empty() {
        output.push_str("## Related Documentation\n\n");
        for doc in &docs {
            let snippet = if doc.content.len() > 300 {
                format!("{}...", &doc.content[..300])
            } else {
                doc.content.clone()
            };
            let _ = writeln!(
                output,
                "- **{} {}**: {}",
                doc.dependency_name, doc.version, snippet
            );
        }
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
    fn test_context_found() {
        let db = Database::open_in_memory().unwrap();
        let file_id =
            db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "parse_config".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn parse_config(path: &Path) -> Config"
                    .into(),
            }],
        )
        .unwrap();

        let result =
            handle_context(&db, "parse_config").unwrap();
        assert!(result.contains("parse_config"));
        assert!(result.contains("src/lib.rs"));
        assert!(result.contains("public"));
    }

    #[test]
    fn test_context_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result =
            handle_context(&db, "nonexistent").unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_context_with_docs() {
        let db = Database::open_in_memory().unwrap();
        let file_id =
            db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "serialize".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn serialize()".into(),
            }],
        )
        .unwrap();

        let dep_id = db
            .insert_dependency("serde", "1.0", true, None)
            .unwrap();
        db.store_doc(
            dep_id,
            "docs.rs",
            "serialize and deserialize data",
        )
        .unwrap();

        let result =
            handle_context(&db, "serialize").unwrap();
        assert!(result.contains("serialize"));
        assert!(result.contains("Related Documentation"));
    }
}
