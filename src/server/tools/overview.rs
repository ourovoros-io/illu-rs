use crate::db::Database;
use crate::indexer::parser::SymbolKind;
use std::fmt::Write;

pub fn handle_overview(
    db: &Database,
    path: &str,
    include_private: bool,
    limit: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.get_symbols_by_path_prefix_filtered(path, include_private)?;

    if symbols.is_empty() {
        let scope = if include_private { "" } else { "public " };
        return Ok(format!(
            "No {scope}symbols found under '{path}'. \
             Try a broader prefix like 'src/'."
        ));
    }

    let max_symbols = limit.map(|l| usize::try_from(l.max(1)).unwrap_or(usize::MAX));
    let mut output = String::new();
    let mut current_file = "";
    let mut file_count = 0u32;
    let mut kind_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut shown = 0usize;

    for sym in &symbols {
        // Variants are shown in their parent enum's details
        if sym.kind == SymbolKind::EnumVariant {
            continue;
        }
        if let Some(max) = max_symbols
            && shown >= max
        {
            break;
        }
        shown += 1;
        if sym.file_path != current_file {
            current_file = &sym.file_path;
            file_count += 1;
            let _ = writeln!(output, "### {current_file}\n");
        }

        *kind_counts.entry(sym.kind.to_string()).or_default() += 1;

        let _ = write!(
            output,
            "- **{}** ({}) `{}`",
            sym.name, sym.kind, sym.signature
        );

        if let Some(doc) = &sym.doc_comment
            && let Some(first_line) = doc.lines().next()
        {
            let _ = write!(output, " — *{first_line}*");
        }

        let _ = writeln!(output);

        if sym.kind == SymbolKind::Function
            && let Ok(callees) = db.get_callees(&sym.name, &sym.file_path)
        {
            let same_file_calls: Vec<&str> = callees
                .iter()
                .filter(|c| c.file_path == sym.file_path && c.ref_kind == "call")
                .map(|c| c.name.as_str())
                .collect();
            if !same_file_calls.is_empty() {
                let _ = writeln!(output, "    calls: {}", same_file_calls.join(", "));
            }
        }
    }

    let truncated = max_symbols.is_some_and(|max| shown >= max && symbols.len() > shown);
    let _ = writeln!(
        output,
        "\n---\n**Summary:** {} symbols across {} files{}",
        shown,
        file_count,
        if truncated {
            format!(" (limited to {shown}, {} total)", symbols.len())
        } else {
            String::new()
        },
    );
    let mut kinds: Vec<_> = kind_counts.into_iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(&a.1));
    let kind_summary: Vec<String> = kinds.iter().map(|(k, c)| format!("{c} {k}s")).collect();
    let _ = writeln!(output, "{}", kind_summary.join(", "));
    let _ = writeln!(
        output,
        "\n*Calls shown are same-file only. Use `context` for full call graph.*"
    );

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    #[test]
    fn test_overview_groups_by_file() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/server/mod.rs", "h1").unwrap();
        let f2 = db.insert_file("src/server/tools.rs", "h2").unwrap();
        store_symbols(
            &db,
            f1,
            &[Symbol {
                name: "serve".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/server/mod.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn serve()".into(),
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
            f2,
            &[Symbol {
                name: "handle".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/server/tools.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn handle()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let result = handle_overview(&db, "src/server/", false, None).unwrap();
        assert!(result.contains("### src/server/mod.rs"));
        assert!(result.contains("### src/server/tools.rs"));
        assert!(result.contains("**serve**"));
        assert!(result.contains("**handle**"));
    }

    #[test]
    fn test_overview_filters_private() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "h1").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "public_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn public_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "private_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 12,
                    signature: "fn private_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let result = handle_overview(&db, "src/", false, None).unwrap();
        assert!(result.contains("**public_fn**"));
        assert!(!result.contains("private_fn"));
    }

    #[test]
    fn test_overview_no_results() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_overview(&db, "nonexistent/", false, None).unwrap();
        assert!(result.contains("No public symbols found under 'nonexistent/'"));
    }

    #[test]
    fn test_overview_include_private() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "h1").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "public_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn public_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "private_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 12,
                    signature: "fn private_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let result = handle_overview(&db, "src/", true, None).unwrap();
        assert!(result.contains("**public_fn**"));
        assert!(
            result.contains("**private_fn**"),
            "should include private symbols"
        );
    }

    #[test]
    fn test_overview_limit() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "h1").unwrap();
        let symbols: Vec<_> = (0..10)
            .map(|i| Symbol {
                name: format!("fn_{i}"),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: i * 10 + 1,
                line_end: i * 10 + 5,
                signature: format!("pub fn fn_{i}()"),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            })
            .collect();
        store_symbols(&db, file_id, &symbols).unwrap();

        // Without limit: all 10
        let result = handle_overview(&db, "src/", false, None).unwrap();
        assert_eq!(result.matches("**fn_").count(), 10);

        // With limit=3: only 3
        let result = handle_overview(&db, "src/", false, Some(3)).unwrap();
        assert_eq!(result.matches("**fn_").count(), 3);
        assert!(result.contains("limited to 3"));
    }

    #[test]
    fn test_overview_intra_file_calls() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "h1").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "orchestrate".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn orchestrate()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "helper_a".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "fn helper_a()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "helper_b".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 22,
                    line_end: 30,
                    signature: "fn helper_b()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // Create call refs: orchestrate -> helper_a, orchestrate -> helper_b
        let src_id = db
            .get_symbol_id("orchestrate", "src/lib.rs")
            .unwrap()
            .unwrap();
        let a_id = db.get_symbol_id("helper_a", "src/lib.rs").unwrap().unwrap();
        let b_id = db.get_symbol_id("helper_b", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(src_id, a_id, "call", "high").unwrap();
        db.insert_symbol_ref(src_id, b_id, "call", "high").unwrap();

        let result = handle_overview(&db, "src/", true, None).unwrap();
        assert!(
            result.contains("calls: helper_a, helper_b")
                || result.contains("calls: helper_b, helper_a"),
            "should show intra-file callees, got: {result}"
        );
        // helper_a has no callees, so no "calls:" line for it
        let lines: Vec<&str> = result.lines().collect();
        let helper_a_idx = lines
            .iter()
            .position(|l| l.contains("**helper_a**"))
            .unwrap();
        let next_line = lines.get(helper_a_idx + 1).unwrap_or(&"");
        assert!(
            !next_line.starts_with("    calls:"),
            "helper_a should not have a calls line"
        );
    }
}
