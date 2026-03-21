use crate::db::Database;
use crate::indexer::parser::SymbolKind;
use std::fmt::Write;

pub fn handle_stats(
    db: &Database,
    path: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let prefix = path.unwrap_or("");
    let mut output = String::new();
    let _ = writeln!(
        output,
        "## Codebase Stats{}\n",
        if prefix.is_empty() {
            String::new()
        } else {
            format!(": {prefix}")
        }
    );

    // Symbol breakdown by kind (targeted SQL, no load-all)
    let kind_counts = db.count_symbols_by_kind(prefix)?;
    let total_symbols: i64 = kind_counts.iter().map(|(_, c)| c).sum();

    // File count
    let file_counts = db.get_file_symbol_counts(prefix)?;
    let file_count = file_counts.len();

    // Load only functions for test coverage calculation
    let mut functions = db.get_symbols_by_path_prefix_filtered(prefix, true)?;
    functions.retain(|s| s.kind == SymbolKind::Function);

    // Test count
    let test_count = functions
        .iter()
        .filter(|s| s.attributes.as_deref().is_some_and(|a| a.contains("test")))
        .count();

    // Non-test, non-main function count for coverage calc
    let fn_count = functions
        .iter()
        .filter(|s| !s.attributes.as_deref().is_some_and(|a| a.contains("test")))
        .filter(|s| s.name != "main")
        .count();

    // Untested function count
    let mut untested = 0;
    for sym in &functions {
        if sym
            .attributes
            .as_deref()
            .is_some_and(|a| a.contains("test"))
        {
            continue;
        }
        if sym.name == "main" {
            continue;
        }
        let tests = db.get_related_tests(&sym.name)?;
        if tests.is_empty() {
            untested += 1;
        }
    }

    let _ = writeln!(output, "### Overview\n");
    let _ = writeln!(output, "- **Files:** {file_count}");
    let _ = writeln!(output, "- **Symbols:** {total_symbols}");

    // Kind breakdown inline
    let kinds_str: Vec<String> = kind_counts
        .iter()
        .map(|(k, c)| format!("{c} {k}s"))
        .collect();
    if !kinds_str.is_empty() {
        let _ = writeln!(output, "  ({})", kinds_str.join(", "));
    }

    let _ = writeln!(output, "- **Tests:** {test_count}");
    if fn_count > 0 {
        let tested = fn_count.saturating_sub(untested);
        let pct = (tested * 100) / fn_count;
        let _ = writeln!(
            output,
            "- **Test coverage:** {tested}/{fn_count} functions ({pct}%)"
        );
    }
    let _ = writeln!(output);

    // Most referenced
    let most_ref = db.get_most_referenced_symbols(5, prefix, None)?;
    if !most_ref.is_empty() {
        let _ = writeln!(output, "### Most Referenced\n");
        for (name, file, count) in &most_ref {
            let _ = writeln!(output, "- **{name}** ({file}) — {count} refs");
        }
        let _ = writeln!(output);
    }

    // Largest files
    let mut sorted_files = file_counts;
    sorted_files.sort_by(|a, b| b.count.cmp(&a.count));
    sorted_files.truncate(5);
    if !sorted_files.is_empty() {
        let _ = writeln!(output, "### Largest Files\n");
        for fc in &sorted_files {
            let _ = writeln!(output, "- **{}** — {} symbols", fc.path, fc.count);
        }
        let _ = writeln!(output);
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

    fn make_sym(name: &str, kind: SymbolKind, file: &str, attrs: Option<&str>) -> Symbol {
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
            attributes: attrs.map(String::from),
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
    fn test_stats_with_data() {
        let db = Database::open_in_memory().unwrap();
        let f = db.insert_file("src/lib.rs", "h1").unwrap();

        let symbols = vec![
            make_sym("foo", SymbolKind::Function, "src/lib.rs", None),
            make_sym("bar", SymbolKind::Function, "src/lib.rs", None),
            make_sym(
                "test_foo",
                SymbolKind::Function,
                "src/lib.rs",
                Some("#[test]"),
            ),
            make_sym("MyStruct", SymbolKind::Struct, "src/lib.rs", None),
        ];
        store_symbols(&db, f, &symbols).unwrap();

        // test_foo calls foo (so foo is tested)
        let test_id = sym_id(&db, "test_foo");
        let foo_id = sym_id(&db, "foo");
        db.insert_symbol_ref(test_id, foo_id, "call", "high")
            .unwrap();

        let result = handle_stats(&db, None).unwrap();

        assert!(result.contains("## Codebase Stats"));
        assert!(result.contains("### Overview"));
        assert!(result.contains("**Files:** 1"));
        assert!(result.contains("**Symbols:** 4"));
        assert!(result.contains("**Tests:** 1"));
        // foo is tested, bar is not => 1/2 = 50%
        assert!(result.contains("**Test coverage:** 1/2 functions (50%)"));
    }

    #[test]
    fn test_stats_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_stats(&db, None).unwrap();

        assert!(result.contains("## Codebase Stats"));
        assert!(result.contains("**Files:** 0"));
        assert!(result.contains("**Symbols:** 0"));
        assert!(result.contains("**Tests:** 0"));
    }

    #[test]
    fn test_stats_path_filter() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "h1").unwrap();
        let f2 = db.insert_file("src/server/mod.rs", "h2").unwrap();

        store_symbols(
            &db,
            f1,
            &[make_sym("lib_fn", SymbolKind::Function, "src/lib.rs", None)],
        )
        .unwrap();
        store_symbols(
            &db,
            f2,
            &[
                make_sym("server_fn", SymbolKind::Function, "src/server/mod.rs", None),
                make_sym(
                    "server_fn2",
                    SymbolKind::Function,
                    "src/server/mod.rs",
                    None,
                ),
            ],
        )
        .unwrap();

        let result = handle_stats(&db, Some("src/server/")).unwrap();

        assert!(result.contains(": src/server/"));
        assert!(result.contains("**Files:** 1"));
        assert!(result.contains("**Symbols:** 2"));
        assert!(!result.contains("lib_fn"));
    }
}
