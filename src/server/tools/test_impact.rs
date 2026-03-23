use crate::db::Database;
use std::fmt::Write;

pub fn handle_test_impact(
    db: &Database,
    symbol_name: &str,
    depth: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = depth.unwrap_or(5);
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(super::symbol_not_found(db, symbol_name));
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Test Impact: {symbol_name}\n");

    let mut all_tests = Vec::new();
    for sym in &symbols {
        let qname = super::qualified_name(sym);
        let tests =
            db.get_related_tests_with_depth(&sym.name, sym.impl_type.as_deref(), max_depth)?;
        if !tests.is_empty() {
            let _ = writeln!(output, "### Tests for `{qname}`\n");
            let test_refs: Vec<&crate::db::TestEntry> = tests.iter().collect();
            super::render_test_list(&mut output, &test_refs);
            for t in &tests {
                if !all_tests
                    .iter()
                    .any(|at: &crate::db::TestEntry| at.name == t.name)
                {
                    all_tests.push(t.clone());
                }
            }
            let _ = writeln!(output);
        }
    }

    if all_tests.is_empty() {
        let _ = writeln!(output, "No tests found that exercise `{symbol_name}`.\n");
        let _ = writeln!(output, "Consider adding test coverage for this symbol.");
        return Ok(output);
    }

    // Suggested cargo test command
    let test_names: Vec<&str> = all_tests.iter().map(|t| t.name.as_str()).collect();
    let suggestion = super::format_cargo_test_suggestion(&test_names);
    let _ = writeln!(output, "---\n### Suggested Command\n");
    let _ = writeln!(output, "```");
    let _ = writeln!(output, "{suggestion}");
    let _ = writeln!(output, "```");

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{RefKind, Symbol, SymbolKind, SymbolRef, Visibility};
    use crate::indexer::store::store_symbols;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        let symbols = vec![
            Symbol {
                name: "helper".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 3,
                signature: "pub fn helper()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "test_helper".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                file_path: "src/lib.rs".into(),
                line_start: 5,
                line_end: 8,
                signature: "fn test_helper()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: Some("test".into()),
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();

        // Create ref: test_helper calls helper
        let symbol_map = db.build_symbol_id_map().unwrap();
        let refs = vec![SymbolRef {
            source_name: "test_helper".into(),
            source_file: "src/lib.rs".into(),
            target_name: "helper".into(),
            kind: RefKind::Call,
            target_file: Some("src/lib.rs".into()),
            target_context: None,
            ref_line: None,
        }];
        db.store_symbol_refs_fast(&refs, &symbol_map).unwrap();
        db
    }

    #[test]
    fn test_test_impact_found() {
        let db = setup_db();
        let result = handle_test_impact(&db, "helper", None).unwrap();
        assert!(result.contains("test_helper"));
        assert!(result.contains("cargo test"));
    }

    #[test]
    fn test_test_impact_not_found() {
        let db = setup_db();
        let result = handle_test_impact(&db, "nonexistent", None).unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_test_impact_no_tests() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        let symbols = vec![Symbol {
            name: "lonely".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 3,
            signature: "pub fn lonely()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }];
        store_symbols(&db, file_id, &symbols).unwrap();
        let result = handle_test_impact(&db, "lonely", None).unwrap();
        assert!(result.contains("No tests found"));
    }
}
