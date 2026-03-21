use crate::db::Database;
use std::fmt::Write;

pub fn handle_hotspots(
    db: &Database,
    path: Option<&str>,
    limit: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let prefix = path.unwrap_or("");
    let max = limit.unwrap_or(10);
    let mut output = String::new();

    let _ = writeln!(output, "## Hotspots\n");

    // Most referenced (fragile to change)
    let most_referenced = db.get_most_referenced_symbols(max, prefix, Some("high"))?;
    if !most_referenced.is_empty() {
        let _ = writeln!(output, "### Most Referenced (fragile to change)\n");
        for (i, (name, file, count)) in most_referenced.iter().enumerate() {
            let _ = writeln!(
                output,
                "{}. **{name}** ({file}) — {count} references",
                i + 1
            );
        }
        let _ = writeln!(output);
    }

    // Most referencing (high complexity)
    let most_referencing = db.get_most_referencing_symbols(max, prefix, Some("high"))?;
    if !most_referencing.is_empty() {
        let _ = writeln!(output, "### Most Referencing (high complexity)\n");
        for (i, (name, file, count)) in most_referencing.iter().enumerate() {
            let _ = writeln!(output, "{}. **{name}** ({file}) — {count} callees", i + 1);
        }
        let _ = writeln!(output);
    }

    // Largest functions (by line span) — targeted SQL query
    let largest = db.get_largest_functions(max, prefix)?;
    if !largest.is_empty() {
        let _ = writeln!(output, "### Largest Functions (by line count)\n");
        for (i, func) in largest.iter().enumerate() {
            let qname = if let Some(it) = &func.impl_type {
                format!("{it}::{}", func.name)
            } else {
                func.name.clone()
            };
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
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
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
        db.insert_symbol_ref(caller_a_id, hub_id, "call", "high")
            .unwrap();
        db.insert_symbol_ref(caller_b_id, hub_id, "call", "high")
            .unwrap();
        // complex -> hub, caller_a, caller_b (3 outgoing)
        db.insert_symbol_ref(complex_id, hub_id, "call", "high")
            .unwrap();
        db.insert_symbol_ref(complex_id, caller_a_id, "call", "high")
            .unwrap();
        db.insert_symbol_ref(complex_id, caller_b_id, "call", "high")
            .unwrap();

        db
    }

    #[test]
    fn test_hotspots_with_data() {
        let db = setup_db_with_refs();
        let result = handle_hotspots(&db, None, None).unwrap();

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
        let result = handle_hotspots(&db, None, None).unwrap();

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
        db.insert_symbol_ref(lib_id, server_id, "call", "high")
            .unwrap();

        let result = handle_hotspots(&db, Some("src/server/"), None).unwrap();
        assert!(result.contains("server_fn"));
        assert!(!result.contains("lib_fn"));
    }
}
