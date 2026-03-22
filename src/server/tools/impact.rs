use crate::db::Database;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

pub fn handle_impact(
    db: &Database,
    symbol_name: &str,
    max_depth: Option<i64>,
    summary: bool,
    exclude_tests: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(super::symbol_not_found(db, symbol_name));
    }

    let (base_name, base_impl) = if let Some((it, method)) = symbol_name.split_once("::") {
        (method, Some(it))
    } else {
        (symbol_name, None)
    };

    let depth = max_depth.unwrap_or(5);
    let mut output = String::new();
    let _ = writeln!(output, "## Impact Analysis: {symbol_name}\n");

    // Crate-level summary (only for workspace projects with >1 crate)
    let crate_count = db.crate_count()?;
    if crate_count > 1 {
        let first_sym = &symbols[0];
        if let Ok(Some(defining_crate)) = db.crate_for_file(&first_sym.file_path) {
            let _ = writeln!(output, "### Affected Crates\n");
            let _ = writeln!(output, "- **{}** (defined here)", defining_crate.name);

            if let Ok(dep_crates) = db.transitive_crate_dependents(defining_crate.id) {
                for c in &dep_crates {
                    let _ = writeln!(output, "- **{}**", c.name);
                }
            }
            output.push('\n');
        }
    }

    let all_dependents = db.impact_dependents_with_depth(base_name, base_impl, depth)?;
    let dependents: Vec<&crate::db::ImpactEntry> = if exclude_tests {
        all_dependents.iter().filter(|d| !d.is_test).collect()
    } else {
        all_dependents.iter().collect()
    };

    let mut current_depth: i64 = -1;
    let mut depth_buf: Vec<&crate::db::ImpactEntry> = Vec::new();
    let mut seen_in_impact: HashSet<&str> = HashSet::new();

    for dep in &dependents {
        if dep.depth != current_depth {
            // Flush previous depth
            if !depth_buf.is_empty() {
                render_depth_entries(&mut output, current_depth, &depth_buf, summary);
                depth_buf.clear();
            }
            current_depth = dep.depth;
        }
        seen_in_impact.insert(&dep.name);
        depth_buf.push(dep);
    }
    // Flush last depth
    if !depth_buf.is_empty() {
        render_depth_entries(&mut output, current_depth, &depth_buf, summary);
    }

    if dependents.is_empty() {
        output.push_str("No dependents found.\n");
        output.push_str(
            "This symbol may be a leaf (not used by other code), \
             or only used in ways the indexer cannot detect \
             (e.g., macro-generated calls, dynamic dispatch).\n",
        );
    }

    // Related tests section — exclude tests already shown in impact
    let all_tests = db.related_tests(base_name, base_impl)?;
    let tests: Vec<_> = all_tests
        .iter()
        .filter(|t| !seen_in_impact.contains(t.name.as_str()))
        .collect();
    if !tests.is_empty() {
        let _ = writeln!(output, "\n### Related Tests\n");
        super::render_test_list(&mut output, &tests);
    }
    if !all_tests.is_empty() {
        let test_names: Vec<&str> = all_tests.iter().map(|t| t.name.as_str()).collect();
        let suggestion = super::format_cargo_test_suggestion(&test_names);
        let _ = writeln!(output, "\nSuggested: `{suggestion}`");
    }

    Ok(output)
}

fn render_depth_entries(
    output: &mut String,
    depth: i64,
    entries: &[&crate::db::ImpactEntry],
    summary: bool,
) {
    const SUMMARY_DEPTH: i64 = 2;
    const SUMMARY_THRESHOLD: usize = 5;

    let should_summarize = summary && depth >= SUMMARY_DEPTH && entries.len() > SUMMARY_THRESHOLD;

    if should_summarize {
        // Group by file
        let mut file_counts: BTreeMap<&str, usize> =
            BTreeMap::new();
        for dep in entries {
            *file_counts.entry(&dep.file_path).or_default() += 1;
        }
        let entry_count = entries.len();
        let file_count = file_counts.len();
        let _ = writeln!(
            output,
            "### Depth {depth} ({entry_count} functions across {file_count} files)\n",
        );
        for (file, count) in &file_counts {
            let _ = writeln!(output, "- **{file}** ({count} symbols)");
        }
        let _ = writeln!(output, "\n*Use `summary: false` to expand all entries.*\n");
    } else {
        let _ = writeln!(output, "### Depth {depth}\n");
        for dep in entries {
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
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::db::SymbolId;
    use crate::indexer::parser::{Confidence, RefKind, Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn sym_id(db: &Database, name: &str, file: &str) -> SymbolId {
        db.symbol_id(name, file).unwrap().unwrap()
    }

    #[test]
    fn test_impact_no_symbol() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_impact(&db, "nonexistent", None, false, false).unwrap();
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

        let result = handle_impact(&db, "lonely_fn", None, false, false).unwrap();
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
        let base_id = db.symbol_id("base_fn", "src/lib.rs").unwrap().unwrap();
        let caller_id = db
            .symbol_id("caller_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(caller_id, base_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        let result = handle_impact(&db, "base_fn", None, false, false).unwrap();
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

        let base_id = db.symbol_id("base_fn", "src/lib.rs").unwrap().unwrap();
        let mid_id = db.symbol_id("mid_fn", "src/lib.rs").unwrap().unwrap();
        let top_id = db.symbol_id("top_fn", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(mid_id, base_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(top_id, mid_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        let result = handle_impact(&db, "base_fn", None, false, false).unwrap();
        assert!(result.contains("mid_fn"), "should show direct dependent");
        assert!(
            result.contains("top_fn"),
            "should show transitive dependent"
        );
        assert!(result.contains("via"), "should show dependency chain");
    }

    #[test]
    fn test_impact_shows_related_tests() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        let test_file_id = db.insert_file("tests/calc.rs", "hash2").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "calculate_tax".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn calculate_tax()".into(),
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
            test_file_id,
            &[
                Symbol {
                    name: "test_tax_basic".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "tests/calc.rs".into(),
                    line_start: 5,
                    line_end: 12,
                    signature: "fn test_tax_basic()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
                Symbol {
                    name: "test_tax_zero".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "tests/calc.rs".into(),
                    line_start: 14,
                    line_end: 20,
                    signature: "fn test_tax_zero()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
                Symbol {
                    name: "unrelated_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "tests/calc.rs".into(),
                    line_start: 22,
                    line_end: 25,
                    signature: "fn unrelated_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // test_tax_basic -> calculate_tax (direct)
        // test_tax_zero  -> calculate_tax (direct)
        // unrelated_fn does NOT call calculate_tax
        let tax_id = sym_id(&db, "calculate_tax", "src/lib.rs");
        let test_basic_id = sym_id(&db, "test_tax_basic", "tests/calc.rs");
        let test_zero_id = sym_id(&db, "test_tax_zero", "tests/calc.rs");
        db.insert_symbol_ref(test_basic_id, tax_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(test_zero_id, tax_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        let result = handle_impact(&db, "calculate_tax", None, false, false).unwrap();
        // Tests that directly call calculate_tax appear in depth entries
        assert!(
            result.contains("test_tax_basic"),
            "should find direct test in depth entries"
        );
        assert!(
            result.contains("test_tax_zero"),
            "should find second test in depth entries"
        );
        assert!(
            !result.contains("unrelated_fn"),
            "should not include unrelated test"
        );
        assert!(
            result.contains("cargo test"),
            "should suggest cargo test command"
        );
        // Since all tests are already in depth entries, Related Tests
        // section should be omitted (deduplication)
        assert!(
            !result.contains("Related Tests"),
            "tests already in depth entries should not be repeated \
             in Related Tests section: {result}"
        );
    }

    #[test]
    fn test_impact_shows_transitive_test() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "inner_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn inner_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "wrapper_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 12,
                    signature: "pub fn wrapper_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "test_via_wrapper".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 14,
                    line_end: 20,
                    signature: "fn test_via_wrapper()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // test_via_wrapper -> wrapper_fn -> inner_fn
        let inner_id = db.symbol_id("inner_fn", "src/lib.rs").unwrap().unwrap();
        let wrapper_id = db
            .symbol_id("wrapper_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        let test_id = db
            .symbol_id("test_via_wrapper", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(wrapper_id, inner_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(test_id, wrapper_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        let result = handle_impact(&db, "inner_fn", None, false, false).unwrap();
        assert!(
            result.contains("test_via_wrapper"),
            "should find transitive test"
        );
    }

    #[test]
    fn test_impact_no_related_tests() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "untested_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn untested_fn()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let result = handle_impact(&db, "untested_fn", None, false, false).unwrap();
        assert!(
            !result.contains("Related Tests"),
            "should not show tests section when none exist"
        );
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
            .symbol_id("SharedType", "shared/src/lib.rs")
            .unwrap()
            .unwrap();
        let app_sym_id = db
            .symbol_id("use_it", "app/src/main.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(app_sym_id, shared_sym_id, RefKind::TypeRef, Confidence::High, None)
            .unwrap();

        let result = handle_impact(&db, "SharedType", None, false, false).unwrap();
        assert!(
            result.contains("Affected Crates"),
            "should have crate summary"
        );
        assert!(result.contains("shared"), "should mention shared crate");
        assert!(result.contains("app"), "should mention app crate");
    }

    #[test]
    fn test_impact_with_impl_type() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "open".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn open() -> Self".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: Some("Database".into()),
                },
                Symbol {
                    name: "caller_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
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

        let open_id = db.symbol_id("open", "src/lib.rs").unwrap().unwrap();
        let caller_id = db
            .symbol_id("caller_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(caller_id, open_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        let result = handle_impact(&db, "Database::open", None, false, false).unwrap();
        assert!(
            result.contains("caller_fn"),
            "impact should find callers for Type::method syntax, got: {result}"
        );
    }

    #[test]
    fn test_impact_related_tests_with_impl_type() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "open".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn open() -> Self".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: Some("Database".into()),
                },
                Symbol {
                    name: "test_open".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "fn test_open()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let open_id = db.symbol_id("open", "src/lib.rs").unwrap().unwrap();
        let test_id = db
            .symbol_id("test_open", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(test_id, open_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        let result = handle_impact(&db, "Database::open", None, false, false).unwrap();
        // test_open directly calls open → appears in depth entries
        assert!(
            result.contains("test_open"),
            "should find test in impact output, got: {result}"
        );
        assert!(
            result.contains("cargo test"),
            "should suggest cargo test command, got: {result}"
        );
    }

    #[test]
    fn test_impact_depth_limit() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "base".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn base()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "mid".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub fn mid()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "top".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 15,
                    signature: "pub fn top()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let base_id = db.symbol_id("base", "src/lib.rs").unwrap().unwrap();
        let mid_id = db.symbol_id("mid", "src/lib.rs").unwrap().unwrap();
        let top_id = db.symbol_id("top", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(mid_id, base_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(top_id, mid_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        // depth=1: only direct callers
        let result = handle_impact(&db, "base", Some(1), false, false).unwrap();
        assert!(result.contains("mid"), "should show direct caller");
        assert!(
            !result.contains("top"),
            "should NOT show transitive caller at depth 1"
        );

        // default depth: shows both
        let result = handle_impact(&db, "base", None, false, false).unwrap();
        assert!(result.contains("mid"));
        assert!(
            result.contains("top"),
            "should show transitive caller at default depth"
        );
    }

    #[test]
    fn test_impact_exclude_tests() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "core_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn core_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "prod_caller".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub fn prod_caller()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "test_core".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 18,
                    signature: "fn test_core()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let core_id = sym_id(&db, "core_fn", "src/lib.rs");
        let prod_id = sym_id(&db, "prod_caller", "src/lib.rs");
        let test_id = sym_id(&db, "test_core", "src/lib.rs");
        db.insert_symbol_ref(prod_id, core_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(test_id, core_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        // Without exclude_tests: both callers appear
        let result = handle_impact(&db, "core_fn", None, false, false).unwrap();
        assert!(result.contains("prod_caller"));
        assert!(result.contains("test_core"));

        // With exclude_tests: test_core excluded from depth entries
        // but still appears in Related Tests section
        let result = handle_impact(&db, "core_fn", None, false, true).unwrap();
        assert!(
            result.contains("prod_caller"),
            "production caller should still appear: {result}"
        );
        // test_core should NOT appear in the Depth section
        let depth_section = result.split("### Related Tests").next().unwrap_or(&result);
        assert!(
            !depth_section.contains("test_core"),
            "test caller should be excluded from depth entries: {result}"
        );
        // But test_core should still appear in Related Tests
        assert!(
            result.contains("Related Tests"),
            "Related Tests section should still be present: {result}"
        );
        assert!(
            result.contains("cargo test"),
            "cargo test suggestion should still appear: {result}"
        );
    }
}
