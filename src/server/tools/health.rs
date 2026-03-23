use crate::db::Database;
use std::fmt::Write;

pub fn handle_health(db: &Database) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();
    let _ = writeln!(output, "## Index Health\n");

    // Ref quality
    let ref_counts = db.count_refs_by_confidence()?;
    let total_refs: i64 = ref_counts.iter().map(|(_, c)| c).sum();
    let _ = writeln!(output, "### Ref Quality\n");
    let _ = writeln!(output, "- **Total refs:** {total_refs}");
    for (confidence, count) in &ref_counts {
        let pct = if total_refs > 0 {
            count * 100 / total_refs
        } else {
            0
        };
        let _ = writeln!(output, "- **{confidence}:** {count} ({pct}%)");
    }
    let _ = writeln!(output);

    // Signature quality
    let truncated = db.count_truncated_signatures()?;
    let total_fns = db.count_functions()?;
    let _ = writeln!(output, "### Signature Quality\n");
    let _ = writeln!(output, "- **Functions:** {total_fns}");
    let _ = writeln!(output, "- **Truncated signatures:** {truncated}");
    let _ = writeln!(output);

    // Noise sources
    let noisy = db.get_noisy_symbols(5)?;
    if !noisy.is_empty() {
        let _ = writeln!(output, "### Noise Sources (low-confidence hotspots)\n");
        for (name, count) in &noisy {
            let _ = writeln!(output, "- **{name}** — {count} low-confidence refs");
        }
        let _ = writeln!(output);
    }

    // Coverage
    let file_count = db.file_count()?;
    let _ = writeln!(output, "### Coverage\n");
    let _ = writeln!(output, "- **Files indexed:** {file_count}");

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::db::SymbolId;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn make_sym(name: &str, kind: SymbolKind, file: &str) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            visibility: Visibility::Public,
            file_path: file.into(),
            line_start: 1,
            line_end: 10,
            signature: format!("pub fn {name}()"),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }
    }

    fn make_truncated_sym(name: &str, file: &str) -> Symbol {
        Symbol {
            name: name.into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: file.into(),
            line_start: 1,
            line_end: 10,
            signature: format!("pub fn {name}("),
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

    #[test]
    fn test_health_with_data() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "h1").unwrap();
        let f2 = db.insert_file("src/util.rs", "h2").unwrap();

        let symbols = [
            make_sym("foo", SymbolKind::Function, "src/lib.rs"),
            make_sym("bar", SymbolKind::Function, "src/lib.rs"),
            make_sym("baz", SymbolKind::Function, "src/util.rs"),
            make_truncated_sym("broken", "src/util.rs"),
        ];
        store_symbols(&db, f1, &symbols[..2]).unwrap();
        store_symbols(&db, f2, &symbols[2..]).unwrap();

        let foo_id = sym_id(&db, "foo");
        let bar_id = sym_id(&db, "bar");
        let baz_id = sym_id(&db, "baz");

        // High-confidence refs
        db.insert_symbol_ref(bar_id, foo_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(baz_id, foo_id, "call", "high", None)
            .unwrap();

        // Low-confidence ref
        db.insert_symbol_ref(baz_id, bar_id, "call", "low", None)
            .unwrap();

        let result = handle_health(&db).unwrap();

        assert!(result.contains("## Index Health"));
        assert!(result.contains("### Ref Quality"));
        assert!(result.contains("**Total refs:** 3"));
        assert!(result.contains("**high:**"));
        assert!(result.contains("**low:**"));
        assert!(result.contains("### Signature Quality"));
        assert!(result.contains("**Functions:** 4"));
        assert!(result.contains("**Truncated signatures:** 1"));
        assert!(result.contains("### Noise Sources"));
        assert!(result.contains("**bar**"));
        assert!(result.contains("### Coverage"));
        assert!(result.contains("**Files indexed:** 2"));
    }

    #[test]
    fn test_health_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_health(&db).unwrap();

        assert!(result.contains("## Index Health"));
        assert!(result.contains("**Total refs:** 0"));
        assert!(result.contains("**Functions:** 0"));
        assert!(result.contains("**Truncated signatures:** 0"));
        assert!(result.contains("**Files indexed:** 0"));
    }
}
