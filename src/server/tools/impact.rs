use crate::db::Database;
use rusqlite::params;
use std::fmt::Write;

pub fn handle_impact(
    db: &Database,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.search_symbols(symbol_name)?;
    if symbols.is_empty() {
        return Ok(format!("No symbol found matching '{symbol_name}'."));
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Impact Analysis: {symbol_name}\n");

    // Crate-level summary (only for workspace projects with >1 crate)
    let crate_count = db.get_crate_count()?;
    if crate_count > 1 {
        let first_sym = &symbols[0];
        if let Ok(Some(defining_crate)) = db.get_crate_for_file(&first_sym.file_path) {
            let _ = writeln!(output, "### Affected Crates\n");
            let _ = writeln!(output, "- **{}** (defined here)", defining_crate.name);

            if let Ok(dep_crates) = db.get_transitive_crate_dependents(defining_crate.id) {
                for c in &dep_crates {
                    let _ = writeln!(output, "- **{}**", c.name);
                }
            }
            output.push('\n');
        }
    }

    // Find direct references using symbol_refs
    let mut stmt = db.conn.prepare(
        "WITH RECURSIVE deps(id, name, file_path, depth) AS (
            SELECT s.id, s.name, f.path, 0
            FROM symbols s
            JOIN files f ON f.id = s.file_id
            WHERE s.name = ?1
          UNION
            SELECT s2.id, s2.name, f2.path, deps.depth + 1
            FROM deps
            JOIN symbol_refs sr ON sr.target_symbol_id = deps.id
            JOIN symbols s2 ON s2.id = sr.source_symbol_id
            JOIN files f2 ON f2.id = s2.file_id
            WHERE deps.depth < 5
        )
        SELECT DISTINCT name, file_path, depth FROM deps
        WHERE depth > 0
        ORDER BY depth, name",
    )?;

    let mut has_deps = false;
    let mut rows = stmt.query(params![symbol_name])?;
    let mut current_depth: i64 = -1;

    while let Some(row) = rows.next()? {
        has_deps = true;
        let name: String = row.get(0)?;
        let file_path: String = row.get(1)?;
        let depth: i64 = row.get(2)?;

        if depth != current_depth {
            current_depth = depth;
            let _ = writeln!(output, "### Depth {depth}\n");
        }
        let _ = writeln!(output, "- **{name}** ({file_path})");
    }

    if !has_deps {
        output.push_str("No dependents found.\n");
        output.push_str(
            "Note: Symbol references are populated \
             during indexing.\n",
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

    #[test]
    fn test_impact_no_symbol() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_impact(&db, "nonexistent").unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_impact_no_dependents() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "lonely_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn lonely_fn()".into(),
                doc_comment: None,
                body: None,
                details: None,
            }],
        )
        .unwrap();

        let result = handle_impact(&db, "lonely_fn").unwrap();
        assert!(result.contains("Impact Analysis"));
        assert!(result.contains("No dependents found"));
    }

    #[test]
    fn test_impact_with_refs() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "base_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn base_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                },
                Symbol {
                    name: "caller_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub fn caller_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                },
            ],
        )
        .unwrap();

        // Create a reference from caller_fn -> base_fn
        db.conn
            .execute(
                "INSERT INTO symbol_refs \
                 (source_symbol_id, target_symbol_id, kind) \
                 VALUES (2, 1, 'call')",
                [],
            )
            .unwrap();

        let result = handle_impact(&db, "base_fn").unwrap();
        assert!(result.contains("caller_fn"));
    }

    #[test]
    fn test_impact_shows_affected_crates() {
        let db = Database::open_in_memory().unwrap();

        let shared_id = db.insert_crate("shared", "shared", false).unwrap();
        let app_id = db.insert_crate("app", "app", false).unwrap();
        db.insert_crate_dep(app_id, shared_id).unwrap();

        let shared_file = db
            .insert_file_with_crate("shared/src/lib.rs", "h1", shared_id)
            .unwrap();
        let app_file = db
            .insert_file_with_crate("app/src/main.rs", "h2", app_id)
            .unwrap();

        store_symbols(
            &db,
            shared_file,
            &[Symbol {
                name: "SharedType".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "shared/src/lib.rs".into(),
                line_start: 1,
                line_end: 3,
                signature: "pub struct SharedType".into(),
                doc_comment: None,
                body: None,
                details: None,
            }],
        )
        .unwrap();

        store_symbols(
            &db,
            app_file,
            &[Symbol {
                name: "use_it".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "app/src/main.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn use_it()".into(),
                doc_comment: None,
                body: None,
                details: None,
            }],
        )
        .unwrap();

        let shared_sym_id = db
            .get_symbol_id("SharedType", "shared/src/lib.rs")
            .unwrap()
            .unwrap();
        let app_sym_id = db
            .get_symbol_id("use_it", "app/src/main.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(app_sym_id, shared_sym_id, "type_ref")
            .unwrap();

        let result = handle_impact(&db, "SharedType").unwrap();
        assert!(
            result.contains("Affected Crates"),
            "should have crate summary"
        );
        assert!(result.contains("shared"), "should mention shared crate");
        assert!(result.contains("app"), "should mention app crate");
    }
}
