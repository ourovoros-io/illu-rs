use crate::db::Database;
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

    let dependents = db.impact_dependents(symbol_name)?;

    let mut current_depth: i64 = -1;
    for dep in &dependents {
        if dep.depth != current_depth {
            current_depth = dep.depth;
            let _ = writeln!(output, "### Depth {}\n", dep.depth);
        }
        if dep.via.is_empty() {
            let _ = writeln!(output, "- **{}** ({})", dep.name, dep.file_path);
        } else {
            let _ = writeln!(
                output,
                "- **{}** ({}) — via {}",
                dep.name, dep.file_path, dep.via
            );
        }
    }

    if dependents.is_empty() {
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
                attributes: None,
                impl_type: None,
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
                    attributes: None,
                    impl_type: None,
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
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // Create a reference from caller_fn -> base_fn
        let base_id = db.get_symbol_id("base_fn", "src/lib.rs").unwrap().unwrap();
        let caller_id = db
            .get_symbol_id("caller_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(caller_id, base_id, "call").unwrap();

        let result = handle_impact(&db, "base_fn").unwrap();
        assert!(result.contains("caller_fn"));
    }

    #[test]
    fn test_impact_shows_chain() {
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
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "mid_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub fn mid_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "top_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 15,
                    signature: "pub fn top_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let base_id = db.get_symbol_id("base_fn", "src/lib.rs").unwrap().unwrap();
        let mid_id = db.get_symbol_id("mid_fn", "src/lib.rs").unwrap().unwrap();
        let top_id = db.get_symbol_id("top_fn", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(mid_id, base_id, "call").unwrap();
        db.insert_symbol_ref(top_id, mid_id, "call").unwrap();

        let result = handle_impact(&db, "base_fn").unwrap();
        assert!(result.contains("mid_fn"), "should show direct dependent");
        assert!(
            result.contains("top_fn"),
            "should show transitive dependent"
        );
        assert!(result.contains("via"), "should show dependency chain");
    }

    #[test]
    fn test_impact_shows_affected_crates() {
        let db = Database::open_in_memory().unwrap();

        let shared_id = db.insert_crate("shared", "shared").unwrap();
        let app_id = db.insert_crate("app", "app").unwrap();
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
                attributes: None,
                impl_type: None,
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
                attributes: None,
                impl_type: None,
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
