use crate::db::Database;
use std::fmt::Write;

pub fn handle_hotspots(
    db: &Database,
    path: Option<&str>,
    limit: Option<i64>,
    exclude_tests: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let prefix = path.unwrap_or("");
    let max = limit.unwrap_or(10);
    let mut output = String::new();

    let _ = writeln!(output, "## Hotspots\n");

    // Most referenced (fragile to change)
    let most_referenced =
        db.most_referenced_symbols_filtered(max, prefix, Some("high"), exclude_tests)?;
    if !most_referenced.is_empty() {
        let _ = writeln!(output, "### Most Referenced (fragile to change)\n");
        for (i, entry) in most_referenced.iter().enumerate() {
            let display = super::format_qualified(&entry.name, entry.impl_type.as_deref());
            let _ = writeln!(
                output,
                "{}. **{display}** ({}) — {} references",
                i + 1,
                entry.file_path,
                entry.count
            );
        }
        let _ = writeln!(output);
    }

    // Most referencing (high complexity)
    let most_referencing =
        db.most_referencing_symbols(max, prefix, Some("high"), exclude_tests)?;
    if !most_referencing.is_empty() {
        let _ = writeln!(output, "### Most Referencing (high complexity)\n");
        for (i, (name, file, count)) in most_referencing.iter().enumerate() {
            let _ = writeln!(output, "{}. **{name}** ({file}) — {count} callees", i + 1);
        }
        let _ = writeln!(output);
    }

    // Largest functions (by line span) — targeted SQL query
    let largest = db.largest_functions(max, prefix, exclude_tests)?;
    if !largest.is_empty() {
        let _ = writeln!(output, "### Largest Functions (by line count)\n");
        for (i, func) in largest.iter().enumerate() {
            let qname = super::format_qualified(&func.name, func.impl_type.as_deref());
            let _ = writeln!(
                output,
                "{}. **{qname}** ({}) — {} lines",
                i + 1,
                func.file_path,
                func.lines
            );
        }
        let _ = writeln!(output);
    }

    if most_referenced.is_empty() && most_referencing.is_empty() && largest.is_empty() {
        let _ = writeln!(output, "No hotspots found.");
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::db::SymbolId;
    use crate::indexer::parser::{Confidence, RefKind, Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn make_fn(name: &str, file: &str, start: usize, end: usize) -> Symbol {
        Symbol {
            name: name.into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: file.into(),
            line_start: start,
            line_end: end,
            signature: format!("pub fn {name}()"),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }
    }

    fn sym_id(db: &Database, name: &str) -> SymbolId {
        db.conn
            .query_row("SELECT id FROM symbols WHERE name = ?1", [name], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn setup_db_with_refs() -> Database {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let symbols = vec![
            make_fn("hub", "src/lib.rs", 1, 50),
            make_fn("caller_a", "src/lib.rs", 52, 60),
            make_fn("caller_b", "src/lib.rs", 62, 70),
            make_fn("complex", "src/lib.rs", 72, 172),
        ];
        store_symbols(&db, file_id, &symbols).unwrap();

        let hub_id = sym_id(&db, "hub");
        let caller_a_id = sym_id(&db, "caller_a");
        let caller_b_id = sym_id(&db, "caller_b");
        let complex_id = sym_id(&db, "complex");

        // caller_a -> hub, caller_b -> hub (hub has 2 incoming)
        db.insert_symbol_ref(caller_a_id, hub_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(caller_b_id, hub_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        // complex -> hub, caller_a, caller_b (3 outgoing)
        db.insert_symbol_ref(complex_id, hub_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(complex_id, caller_a_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(complex_id, caller_b_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        db
    }

    #[test]
    fn test_hotspots_with_data() {
        let db = setup_db_with_refs();
        let result = handle_hotspots(&db, None, None, false).unwrap();

        assert!(result.contains("## Hotspots"));
        assert!(result.contains("### Most Referenced (fragile to change)"));
        assert!(result.contains("**hub**"));
        assert!(result.contains("### Most Referencing (high complexity)"));
        assert!(result.contains("**complex**"));
        assert!(result.contains("### Largest Functions (by line count)"));
        // complex is 101 lines, hub is 50 lines
        // In "Largest Functions" section, complex should appear before hub
        let largest_section = result.find("### Largest Functions").unwrap();
        let complex_in_largest = result[largest_section..].find("**complex**");
        let hub_in_largest = result[largest_section..].find("**hub**");
        assert!(complex_in_largest < hub_in_largest);
        // Verify line counts appear
        assert!(result.contains("101 lines"));
    }

    #[test]
    fn test_hotspots_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_hotspots(&db, None, None, false).unwrap();

        assert!(result.contains("No hotspots found."));
    }

    #[test]
    fn test_hotspots_path_filter() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "h1").unwrap();
        let f2 = db.insert_file("src/server/mod.rs", "h2").unwrap();

        store_symbols(&db, f1, &[make_fn("lib_fn", "src/lib.rs", 1, 100)]).unwrap();
        store_symbols(
            &db,
            f2,
            &[make_fn("server_fn", "src/server/mod.rs", 1, 200)],
        )
        .unwrap();

        let lib_id = sym_id(&db, "lib_fn");
        let server_id = sym_id(&db, "server_fn");
        db.insert_symbol_ref(lib_id, server_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        let result = handle_hotspots(&db, Some("src/server/"), None, false).unwrap();
        assert!(result.contains("server_fn"));
        assert!(!result.contains("lib_fn"));
    }

    #[test]
    fn test_hotspots_exclude_tests_largest_functions() {
        let db = Database::open_in_memory().unwrap();
        let f = db.insert_file("src/lib.rs", "h1").unwrap();

        store_symbols(
            &db,
            f,
            &[
                make_fn("prod_fn", "src/lib.rs", 1, 100),
                Symbol {
                    name: "test_big".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 110,
                    line_end: 300,
                    signature: "fn test_big()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // Without exclude: test_big (191 lines) appears first
        let result = handle_hotspots(&db, None, None, false).unwrap();
        let largest = result.find("### Largest Functions").unwrap();
        assert!(
            result[largest..].contains("test_big"),
            "test function should appear without exclude: {result}"
        );

        // With exclude: only prod_fn appears
        let result = handle_hotspots(&db, None, None, true).unwrap();
        let largest = result.find("### Largest Functions").unwrap();
        assert!(
            !result[largest..].contains("test_big"),
            "test function should be excluded: {result}"
        );
        assert!(
            result[largest..].contains("prod_fn"),
            "production function should still appear: {result}"
        );
    }

    #[test]
    fn test_hotspots_exclude_tests() {
        let db = Database::open_in_memory().unwrap();
        let f = db.insert_file("src/lib.rs", "h1").unwrap();

        store_symbols(
            &db,
            f,
            &[
                make_fn("target_fn", "src/lib.rs", 1, 10),
                make_fn("prod_caller", "src/lib.rs", 12, 20),
                Symbol {
                    name: "test_caller".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 22,
                    line_end: 30,
                    signature: "fn test_caller()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let target_id = sym_id(&db, "target_fn");
        let prod_id = sym_id(&db, "prod_caller");
        let test_id = sym_id(&db, "test_caller");

        // 1 prod ref + 1 test ref
        db.insert_symbol_ref(prod_id, target_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(test_id, target_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        // Without exclude: 2 references
        let result = handle_hotspots(&db, None, None, false).unwrap();
        assert!(result.contains("2 references"));

        // With exclude: only 1 reference (prod only)
        let result = handle_hotspots(&db, None, None, true).unwrap();
        assert!(
            result.contains("1 reference"),
            "should count only prod references: {result}"
        );
    }

    #[test]
    fn test_hotspots_exclude_tests_most_referencing() {
        let db = Database::open_in_memory().unwrap();
        let f = db.insert_file("src/lib.rs", "h1").unwrap();

        store_symbols(
            &db,
            f,
            &[
                make_fn("target_a", "src/lib.rs", 1, 5),
                make_fn("target_b", "src/lib.rs", 7, 12),
                make_fn("prod_complex", "src/lib.rs", 14, 20),
                Symbol {
                    name: "test_complex".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 22,
                    line_end: 30,
                    signature: "fn test_complex()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let target_a = sym_id(&db, "target_a");
        let target_b = sym_id(&db, "target_b");
        let prod_id = sym_id(&db, "prod_complex");
        let test_id = sym_id(&db, "test_complex");

        // test_complex calls both targets (2 outgoing)
        db.insert_symbol_ref(test_id, target_a, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(test_id, target_b, RefKind::Call, Confidence::High, None)
            .unwrap();
        // prod_complex calls one target (1 outgoing)
        db.insert_symbol_ref(prod_id, target_a, RefKind::Call, Confidence::High, None)
            .unwrap();

        // Without exclude: test_complex tops Most Referencing (2 callees)
        let result = handle_hotspots(&db, None, None, false).unwrap();
        let section = result.find("### Most Referencing").unwrap();
        assert!(
            result[section..].contains("test_complex"),
            "test function should appear without exclude: {result}"
        );

        // With exclude: test_complex filtered out
        let result = handle_hotspots(&db, None, None, true).unwrap();
        let section = result.find("### Most Referencing").unwrap();
        assert!(
            !result[section..].contains("test_complex"),
            "test function should be excluded from Most Referencing: {result}"
        );
        assert!(
            result[section..].contains("prod_complex"),
            "prod function should still appear: {result}"
        );
    }
}
