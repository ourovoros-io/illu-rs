use crate::db::Database;
use crate::indexer::parser::SymbolKind;
use std::fmt::Write;

pub fn handle_orphaned(
    db: &Database,
    path: Option<&str>,
    kind: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    // Get unused symbols (no incoming references)
    let mut unused = db.get_unreferenced_symbols(path, false)?;

    // Filter to meaningful kinds
    unused.retain(|s| {
        s.kind != SymbolKind::EnumVariant
            && s.kind != SymbolKind::Use
            && s.kind != SymbolKind::Mod
            && s.kind != SymbolKind::Impl
    });
    unused.retain(|s| !super::is_entry_point(s));

    if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        unused.retain(|s| s.kind.to_string().to_lowercase() == k_lower);
    }

    // Further filter to symbols with no test coverage
    let mut orphaned = Vec::new();
    for sym in unused {
        let tests = db.get_related_tests(&sym.name)?;
        if tests.is_empty() {
            orphaned.push(sym);
        }
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Orphaned Symbols\n");

    if orphaned.is_empty() {
        let _ = writeln!(
            output,
            "No orphaned symbols found \
             (all symbols have callers or test coverage)."
        );
        return Ok(output);
    }

    let _ = writeln!(
        output,
        "Found {} symbols with **no callers AND no test coverage**:\n",
        orphaned.len()
    );

    let mut current_file = String::new();
    for sym in &orphaned {
        if sym.file_path != current_file {
            current_file.clone_from(&sym.file_path);
            let _ = writeln!(output, "### {current_file}\n");
        }
        let qname = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "- **{qname}** ({}, {}, line {}-{})",
            sym.kind, sym.visibility, sym.line_start, sym.line_end
        );
    }

    let _ = writeln!(
        output,
        "\n*These symbols are safe to remove or should have tests added.*"
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
                name: "orphan".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 3,
                signature: "pub fn orphan()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "test_something".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                file_path: "src/lib.rs".into(),
                line_start: 5,
                line_end: 8,
                signature: "fn test_something()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: Some("#[test]".into()),
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();
        db
    }

    #[test]
    fn test_orphaned_finds_dead_code() {
        let db = setup_db();
        let result = handle_orphaned(&db, None, None).unwrap();
        assert!(result.contains("orphan"));
        assert!(!result.contains("test_something"));
    }

    #[test]
    fn test_orphaned_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_orphaned(&db, None, None).unwrap();
        assert!(result.contains("No orphaned symbols"));
    }
}
