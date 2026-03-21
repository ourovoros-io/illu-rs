use crate::db::Database;
use crate::server::tools::context;

pub fn handle_batch_context(
    db: &Database,
    symbols: &[String],
    full_body: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if symbols.is_empty() {
        return Ok("No symbols provided.".to_string());
    }

    let mut output = String::new();
    for (i, symbol) in symbols.iter().enumerate() {
        if i > 0 {
            output.push_str("\n---\n\n");
        }
        let result = context::handle_context(db, symbol, full_body, None, None, None, false)?;
        output.push_str(&result);
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn make_fn(name: &str, file: &str, line: usize) -> Symbol {
        Symbol {
            name: name.into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: file.into(),
            line_start: line,
            line_end: line + 4,
            signature: format!("pub fn {name}()"),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }
    }

    #[test]
    fn test_batch_context_empty_symbols() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let result = handle_batch_context(&db, &[], false).unwrap();
        assert_eq!(result, "No symbols provided.");
    }

    #[test]
    fn test_batch_context_all_found() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                make_fn("alpha", "src/lib.rs", 1),
                make_fn("beta", "src/lib.rs", 10),
            ],
        )
        .unwrap();

        let symbols = vec!["alpha".to_string(), "beta".to_string()];
        let result = handle_batch_context(&db, &symbols, false).unwrap();

        assert!(result.contains("alpha"), "should contain alpha context");
        assert!(result.contains("beta"), "should contain beta context");
        assert!(result.contains("---"), "results should be separated");
    }

    #[test]
    fn test_batch_context_one_not_found() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        store_symbols(&db, file_id, &[make_fn("existing", "src/lib.rs", 1)]).unwrap();

        let symbols = vec!["existing".to_string(), "missing".to_string()];
        let result = handle_batch_context(&db, &symbols, false).unwrap();

        assert!(result.contains("existing"), "found symbol should appear");
        assert!(
            result.contains("No symbol found matching 'missing'"),
            "missing symbol should produce not-found message"
        );
    }

    #[test]
    fn test_batch_context_single_symbol() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        store_symbols(&db, file_id, &[make_fn("solo", "src/lib.rs", 5)]).unwrap();

        let symbols = vec!["solo".to_string()];
        let result = handle_batch_context(&db, &symbols, false).unwrap();

        assert!(result.contains("solo"));
        // Single result has no separator
        assert!(!result.contains("---"));
    }
}
