use crate::db::{CalleeInfo, Database, StoredSymbol};
use crate::indexer::parser::SymbolKind;
use std::fmt::Write;
use std::path::Path;

/// Format a callee/caller name as `ImplType::name` when `impl_type` is present.
fn qualified_callee_name(c: &CalleeInfo) -> String {
    if let Some(it) = &c.impl_type {
        format!("{}::{}", it, c.name)
    } else {
        c.name.clone()
    }
}

pub fn handle_context(
    db: &Database,
    symbol_name: &str,
    full_body: bool,
    file: Option<&str>,
    sections: Option<&[&str]>,
    callers_path: Option<&str>,
    exclude_tests: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = resolve_symbols(db, symbol_name, file)?;
    if symbols.is_empty() {
        return Ok(format!(
            "No symbol found matching '{symbol_name}'.\n\
            Try `Type::method` syntax for methods \
            (e.g. `Database::new`), a partial name, or use `query` to search."
        ));
    }

    let show =
        |name: &str| -> bool { sections.is_none() || sections.is_some_and(|s| s.contains(&name)) };

    let repo_root = db.repo_root();
    let mut output = String::new();

    for sym in &symbols {
        render_symbol_header(&mut output, sym);
        if show("source") {
            render_symbol_details(&mut output, sym, full_body, repo_root);
        }
        if show("traits") {
            render_trait_info(db, &mut output, sym)?;
        }
        if show("callers") {
            render_callers(db, &mut output, sym, callers_path, exclude_tests)?;
        }
        if show("callees") {
            render_callees(db, &mut output, sym, callers_path, exclude_tests)?;
        }
        if show("tested_by") {
            render_tested_by(db, &mut output, sym)?;
        }
        if show("related") {
            render_related(db, &mut output, sym)?;
        }
    }

    if show("docs") {
        let base_name = symbol_name.split_once("::").map_or(symbol_name, |(_, m)| m);
        render_related_docs(db, &mut output, base_name)?;
    }

    Ok(output)
}

/// Resolve symbols supporting `Type::method` syntax and optional file filter.
fn resolve_symbols(
    db: &Database,
    symbol_name: &str,
    file: Option<&str>,
) -> Result<Vec<StoredSymbol>, Box<dyn std::error::Error>> {
    let mut symbols = super::resolve_symbol(db, symbol_name)?;

    if let Some(fp) = file {
        symbols.retain(|s| s.file_path == fp);
    }

    Ok(symbols)
}

fn render_symbol_header(output: &mut String, sym: &StoredSymbol) {
    let _ = writeln!(output, "## {} ({})\n", sym.name, sym.kind);

    if let Some(doc) = &sym.doc_comment {
        for line in doc.lines() {
            let _ = writeln!(output, "> {line}");
        }
        let _ = writeln!(output);
    }

    let _ = writeln!(
        output,
        "- **File:** {}:{}-{}",
        sym.file_path, sym.line_start, sym.line_end
    );
    let _ = writeln!(output, "- **Visibility:** {}", sym.visibility);
    let _ = writeln!(output, "- **Signature:** `{}`", sym.signature);
    if let Some(attrs) = &sym.attributes {
        let _ = writeln!(output, "- **Attributes:** {attrs}");
    }
    if let Some(impl_type) = &sym.impl_type {
        let _ = writeln!(output, "- **Impl:** {impl_type}");
    }
    let _ = writeln!(output);
}

fn render_symbol_details(
    output: &mut String,
    sym: &StoredSymbol,
    full_body: bool,
    repo_root: Option<&Path>,
) {
    if let Some(details) = &sym.details {
        let _ = writeln!(output, "### Fields/Variants\n");
        let _ = writeln!(output, "```rust\n{details}\n```\n");
    }

    if let Some(body) = &sym.body {
        let is_truncated = body.ends_with("// ... truncated");
        if is_truncated
            && full_body
            && let Some(full) =
                read_lines_from_file(repo_root, &sym.file_path, sym.line_start, sym.line_end)
        {
            let _ = writeln!(output, "### Source\n");
            let _ = writeln!(output, "```rust\n{full}\n```\n");
        } else if is_truncated {
            let _ = writeln!(output, "### Source (truncated)\n");
            let _ = writeln!(output, "```rust\n{body}\n```\n");
            let _ = writeln!(
                output,
                "*Full source at {}:{}-{}. Use `full_body: true` to fetch.*\n",
                sym.file_path, sym.line_start, sym.line_end,
            );
        } else {
            let _ = writeln!(output, "### Source\n");
            let _ = writeln!(output, "```rust\n{body}\n```\n");
        }
    }
}

fn read_lines_from_file(
    repo_root: Option<&Path>,
    file_path: &str,
    line_start: i64,
    line_end: i64,
) -> Option<String> {
    let root = repo_root?;
    let full_path = root.join(file_path);
    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("full_body: failed to read {}: {e}", full_path.display());
            return None;
        }
    };
    let start = usize::try_from(line_start.saturating_sub(1)).ok()?;
    let end = usize::try_from(line_end).ok()?;
    let mut result = String::new();
    for (i, line) in content.lines().skip(start).take(end - start).enumerate() {
        if i > 0 {
            result.push('\n');
        }
        result.push_str(line);
    }
    Some(result)
}

fn render_trait_info(
    db: &Database,
    output: &mut String,
    sym: &StoredSymbol,
) -> Result<(), Box<dyn std::error::Error>> {
    if sym.kind == SymbolKind::Struct || sym.kind == SymbolKind::Enum {
        let impls = db.get_trait_impls_for_type(&sym.name)?;
        if !impls.is_empty() {
            let _ = writeln!(output, "### Trait Implementations\n");
            for ti in &impls {
                let _ = writeln!(
                    output,
                    "- **{}** ({}:{}-{})",
                    ti.trait_name, ti.file_path, ti.line_start, ti.line_end
                );
            }
            let _ = writeln!(output);
        }
    }

    if sym.kind == SymbolKind::Trait {
        let implementors = db.get_trait_impls_for_trait(&sym.name)?;
        if !implementors.is_empty() {
            let _ = writeln!(output, "### Implemented By\n");
            for ti in &implementors {
                let _ = writeln!(
                    output,
                    "- **{}** ({}:{}-{})",
                    ti.type_name, ti.file_path, ti.line_start, ti.line_end
                );
            }
            let _ = writeln!(output);
        }
    }

    Ok(())
}

fn render_callers(
    db: &Database,
    output: &mut String,
    sym: &StoredSymbol,
    callers_path: Option<&str>,
    exclude_tests: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    const MAX_CALLERS: usize = 30;

    let mut callers = db.get_callers(&sym.name, &sym.file_path, exclude_tests)?;
    if let Some(p) = callers_path {
        callers.retain(|c| c.file_path.starts_with(p));
    }
    if callers.is_empty() {
        return Ok(());
    }

    callers.sort_by_key(|c| c.is_test);

    let total = callers.len();
    let _ = writeln!(output, "### Called By\n");
    let mut shown_test_separator = false;
    for c in callers.iter().take(MAX_CALLERS) {
        if c.is_test && !shown_test_separator {
            let _ = writeln!(output);
            shown_test_separator = true;
        }
        let display = qualified_callee_name(c);
        let line = c.ref_line.unwrap_or(c.line_start);
        let _ = writeln!(output, "- {} ({}:{})", display, c.file_path, line);
    }
    if total > MAX_CALLERS {
        let _ = writeln!(
            output,
            "\n*({} more — use `references` for the full list)*",
            total - MAX_CALLERS
        );
    }
    let _ = writeln!(output);

    Ok(())
}

fn render_callees(
    db: &Database,
    output: &mut String,
    sym: &StoredSymbol,
    callers_path: Option<&str>,
    exclude_tests: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut callees = db.get_callees(&sym.name, &sym.file_path, exclude_tests)?;
    if let Some(p) = callers_path {
        callees.retain(|c| c.file_path.starts_with(p));
    }
    if callees.is_empty() {
        return Ok(());
    }

    let _ = writeln!(output, "### Callees\n");

    let call_kind = crate::indexer::parser::RefKind::Call.to_string();
    let calls: Vec<_> = callees.iter().filter(|c| c.ref_kind == call_kind).collect();
    let type_refs: Vec<_> = callees.iter().filter(|c| c.ref_kind != call_kind).collect();

    if !calls.is_empty() {
        let _ = writeln!(output, "**Calls:**");
        for c in &calls {
            let display = qualified_callee_name(c);
            let _ = writeln!(output, "- {} ({}:{})", display, c.file_path, c.line_start);
        }
        let _ = writeln!(output);
    }

    if !type_refs.is_empty() {
        let _ = writeln!(output, "**Uses types:**");
        for c in &type_refs {
            let display = qualified_callee_name(c);
            let _ = writeln!(output, "- {} ({}:{})", display, c.file_path, c.line_start);
        }
        let _ = writeln!(output);
    }

    Ok(())
}

fn render_tested_by(
    db: &Database,
    output: &mut String,
    sym: &StoredSymbol,
) -> Result<(), Box<dyn std::error::Error>> {
    const MAX_INLINE: usize = 10;

    let tests = db.get_related_tests(&sym.name, sym.impl_type.as_deref())?;
    if tests.is_empty() {
        return Ok(());
    }

    let _ = writeln!(output, "### Tested By\n");
    if tests.len() <= MAX_INLINE {
        for t in &tests {
            let _ = writeln!(
                output,
                "- **{}** ({}:{})",
                t.name, t.file_path, t.line_start
            );
        }
    } else {
        let mut file_counts: std::collections::BTreeMap<&str, usize> =
            std::collections::BTreeMap::new();
        for t in &tests {
            *file_counts.entry(&t.file_path).or_default() += 1;
        }
        let _ = writeln!(
            output,
            "{} tests across {} files:\n",
            tests.len(),
            file_counts.len()
        );
        for (file, count) in &file_counts {
            let _ = writeln!(output, "- **{file}** ({count} tests)");
        }
    }
    let _ = writeln!(output);

    Ok(())
}

const MAX_RELATED: usize = 10;

fn render_related(
    db: &Database,
    output: &mut String,
    sym: &StoredSymbol,
) -> Result<(), Box<dyn std::error::Error>> {
    let siblings = db.get_symbols_by_path_prefix(&sym.file_path)?;
    let related: Vec<_> = siblings
        .iter()
        .filter(|s| s.name != sym.name || s.line_start != sym.line_start)
        .filter(|s| s.impl_type == sym.impl_type)
        .filter(|s| {
            s.kind != SymbolKind::Use
                && s.kind != SymbolKind::Mod
                && s.kind != SymbolKind::EnumVariant
                && s.kind != SymbolKind::Impl
        })
        .collect();

    if related.is_empty() {
        return Ok(());
    }

    let label = if let Some(it) = &sym.impl_type {
        format!("Related (impl {it})")
    } else {
        "Related (same file)".to_string()
    };
    let _ = writeln!(output, "### {label}\n");
    for s in related.iter().take(MAX_RELATED) {
        let _ = writeln!(
            output,
            "- **{}** ({}, line {}-{})",
            s.name, s.kind, s.line_start, s.line_end
        );
    }
    if related.len() > MAX_RELATED {
        let _ = writeln!(
            output,
            "- *({} more — use `overview` to see all)*",
            related.len() - MAX_RELATED
        );
    }
    let _ = writeln!(output);
    Ok(())
}

fn render_related_docs(
    db: &Database,
    output: &mut String,
    symbol_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let docs = db.search_docs(symbol_name)?;
    if !docs.is_empty() {
        output.push_str("## Related Documentation\n\n");
        for doc in &docs {
            let snippet = super::truncate_snippet(&doc.content, 300);
            let _ = writeln!(
                output,
                "- **{} {}**: {}",
                doc.dependency_name, doc.version, &snippet
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    #[test]
    fn test_context_found() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "parse_config".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn parse_config(path: &Path) -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let result = handle_context(&db, "parse_config", false, None, None, None, false).unwrap();
        assert!(result.contains("parse_config"));
        assert!(result.contains("src/lib.rs"));
        assert!(result.contains("public"));
    }

    #[test]
    fn test_context_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_context(&db, "nonexistent", false, None, None, None, false).unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_context_with_docs() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "serialize".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn serialize()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "serialize and deserialize data")
            .unwrap();

        let result = handle_context(&db, "serialize", false, None, None, None, false).unwrap();
        assert!(result.contains("serialize"));
        assert!(result.contains("Related Documentation"));
    }

    #[test]
    fn test_context_includes_doc_comment() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "Config".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 5,
                line_end: 15,
                signature: "pub struct Config".into(),
                doc_comment: Some("Application configuration.\nHolds all settings.".into()),
                body: Some("pub struct Config { pub port: u16 }".into()),
                details: Some("port: u16".into()),
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let result = handle_context(&db, "Config", false, None, None, None, false).unwrap();
        assert!(result.contains("> Application configuration."));
        assert!(result.contains("> Holds all settings."));
        assert!(result.contains("### Fields/Variants"));
        assert!(result.contains("port: u16"));
        assert!(result.contains("### Source"));
        assert!(result.contains("pub struct Config { pub port: u16 }"));
    }

    #[test]
    fn test_context_includes_callees() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "caller_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn caller_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "callee_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn callee_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let caller_id = db
            .get_symbol_id("caller_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        let callee_id = db
            .get_symbol_id("callee_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(caller_id, callee_id, "call", "high", None)
            .unwrap();

        let result = handle_context(&db, "caller_fn", false, None, None, None, false).unwrap();
        assert!(result.contains("### Callees"));
        assert!(result.contains("**Calls:**"));
        assert!(result.contains("callee_fn"));
    }

    #[test]
    fn test_context_includes_trait_impls() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "MyStruct".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct MyStruct".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        db.insert_trait_impl("MyStruct", "Display", file_id, 10, 20)
            .unwrap();
        db.insert_trait_impl("MyStruct", "Debug", file_id, 22, 30)
            .unwrap();

        let result = handle_context(&db, "MyStruct", false, None, None, None, false).unwrap();
        assert!(result.contains("### Trait Implementations"));
        assert!(result.contains("**Display**"));
        assert!(result.contains("**Debug**"));
    }

    #[test]
    fn test_context_includes_callers() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "target_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn target_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "caller_a".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn caller_a()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let target_id = db
            .get_symbol_id("target_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        let caller_id = db.get_symbol_id("caller_a", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(caller_id, target_id, "call", "high", None)
            .unwrap();

        let result = handle_context(&db, "target_fn", false, None, None, None, false).unwrap();
        assert!(
            result.contains("### Called By"),
            "should show callers section"
        );
        assert!(result.contains("caller_a"), "should list the caller");
    }

    #[test]
    fn test_context_qualified_name() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();

        // Insert two `new` methods with different impl_types via raw SQL
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, impl_type) \
                 VALUES (?1, 'new', 'function', 'public', \
                         1, 5, 'pub fn new() -> Database', 'Database')",
                rusqlite::params![file_id],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, impl_type) \
                 VALUES (?1, 'new', 'function', 'public', \
                         10, 15, 'pub fn new() -> Server', 'Server')",
                rusqlite::params![file_id],
            )
            .unwrap();

        // Qualified query should return only Database::new
        let result = handle_context(&db, "Database::new", false, None, None, None, false).unwrap();
        assert!(result.contains("Database"), "should find Database::new");
        assert!(!result.contains("Server"), "should NOT include Server::new");
    }

    #[test]
    fn test_context_file_filter() {
        let db = Database::open_in_memory().unwrap();
        let file_a = db.insert_file("src/a.rs", "h1").unwrap();
        let file_b = db.insert_file("src/b.rs", "h2").unwrap();
        store_symbols(
            &db,
            file_a,
            &[Symbol {
                name: "Config".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/a.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct Config".into(),
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
            file_b,
            &[Symbol {
                name: "Config".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/b.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        // Without file filter: both appear
        let result = handle_context(&db, "Config", false, None, None, None, false).unwrap();
        assert!(result.contains("src/a.rs"));
        assert!(result.contains("src/b.rs"));

        // With file filter: only one
        let result =
            handle_context(&db, "Config", false, Some("src/a.rs"), None, None, false).unwrap();
        assert!(result.contains("src/a.rs"));
        assert!(!result.contains("src/b.rs"));
    }

    #[test]
    fn test_context_sections_source_only() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "my_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn my_fn()".into(),
                    doc_comment: None,
                    body: Some("pub fn my_fn() { helper() }".into()),
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "helper".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn helper()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let my_fn_id = db.get_symbol_id("my_fn", "src/lib.rs").unwrap().unwrap();
        let helper_id = db.get_symbol_id("helper", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(my_fn_id, helper_id, "call", "high", None)
            .unwrap();

        let sections: &[&str] = &["source"];
        let result =
            handle_context(&db, "my_fn", false, None, Some(sections), None, false).unwrap();
        assert!(result.contains("### Source"), "source section present");
        assert!(!result.contains("### Callees"), "callees section absent");
    }

    #[test]
    fn test_context_sections_callers_only() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "target".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn target()".into(),
                    doc_comment: None,
                    body: Some("pub fn target() {}".into()),
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "invoker".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn invoker()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let invoker_id = db.get_symbol_id("invoker", "src/lib.rs").unwrap().unwrap();
        let target_id = db.get_symbol_id("target", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(invoker_id, target_id, "call", "high", None)
            .unwrap();

        let sections: &[&str] = &["callers"];
        let result =
            handle_context(&db, "target", false, None, Some(sections), None, false).unwrap();
        assert!(result.contains("### Called By"), "callers section present");
        assert!(!result.contains("### Source"), "source section absent");
    }

    #[test]
    fn test_context_sections_none_shows_all() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "all_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn all_fn()".into(),
                    doc_comment: None,
                    body: Some("pub fn all_fn() { dep() }".into()),
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "dep".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn dep()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let all_fn_id = db.get_symbol_id("all_fn", "src/lib.rs").unwrap().unwrap();
        let dep_id = db.get_symbol_id("dep", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(all_fn_id, dep_id, "call", "high", None)
            .unwrap();

        let result = handle_context(&db, "all_fn", false, None, None, None, false).unwrap();
        assert!(result.contains("### Source"), "source present");
        assert!(result.contains("### Callees"), "callees present");
    }

    #[test]
    fn test_context_callers_path_filter() {
        let db = Database::open_in_memory().unwrap();
        let src_file = db.insert_file("src/lib.rs", "h1").unwrap();
        let test_file = db.insert_file("tests/test.rs", "h2").unwrap();
        store_symbols(
            &db,
            src_file,
            &[
                Symbol {
                    name: "target_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn target_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "src_caller".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn src_caller()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();
        store_symbols(
            &db,
            test_file,
            &[Symbol {
                name: "test_caller".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "tests/test.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "fn test_caller()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let target_id = db
            .get_symbol_id("target_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        let src_id = db
            .get_symbol_id("src_caller", "src/lib.rs")
            .unwrap()
            .unwrap();
        let test_id = db
            .get_symbol_id("test_caller", "tests/test.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(src_id, target_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(test_id, target_id, "call", "high", None)
            .unwrap();

        // Without callers_path: both callers shown
        let result = handle_context(&db, "target_fn", false, None, None, None, false).unwrap();
        assert!(result.contains("src_caller"), "src caller present");
        assert!(result.contains("test_caller"), "test caller present");

        // With callers_path="src/": only src caller
        let result =
            handle_context(&db, "target_fn", false, None, None, Some("src/"), false).unwrap();
        assert!(result.contains("src_caller"), "src caller present");
        assert!(
            !result.contains("test_caller"),
            "test caller should be filtered out"
        );
    }

    #[test]
    fn test_context_related_same_impl() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "method_a".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn method_a(&self)".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: Some("MyType".into()),
                },
                Symbol {
                    name: "method_b".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn method_b(&self)".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: Some("MyType".into()),
                },
            ],
        )
        .unwrap();

        let result =
            handle_context(&db, "MyType::method_a", false, None, None, None, false).unwrap();
        assert!(
            result.contains("### Related (impl MyType)"),
            "should show related section with impl label"
        );
        assert!(result.contains("method_b"), "should list sibling method");
    }

    #[test]
    fn test_context_related_top_level() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "func_one".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn func_one()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "func_two".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn func_two()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let result = handle_context(&db, "func_one", false, None, None, None, false).unwrap();
        assert!(
            result.contains("### Related (same file)"),
            "should show related section with file label"
        );
        assert!(result.contains("func_two"), "should list sibling function");
    }

    #[test]
    fn test_context_related_filtered_by_sections() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "alpha".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn alpha()".into(),
                    doc_comment: None,
                    body: Some("pub fn alpha() {}".into()),
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "beta".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn beta()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let sections: &[&str] = &["source"];
        let result =
            handle_context(&db, "alpha", false, None, Some(sections), None, false).unwrap();
        assert!(result.contains("### Source"), "source section present");
        assert!(
            !result.contains("### Related"),
            "related section should be absent when not requested"
        );
    }

    #[test]
    fn test_context_related_capped_at_10() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        let mut syms = Vec::new();
        for i in 0..15 {
            syms.push(Symbol {
                name: format!("method_{i}"),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: i * 10 + 1,
                line_end: i * 10 + 9,
                signature: format!("pub fn method_{i}()"),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("BigType".into()),
            });
        }
        store_symbols(&db, file_id, &syms).unwrap();

        let sections: &[&str] = &["related"];
        let result = handle_context(
            &db,
            "BigType::method_0",
            false,
            None,
            Some(sections),
            None,
            false,
        )
        .unwrap();
        assert!(
            result.contains("4 more"),
            "should show overflow count when >10 related, got: {result}"
        );
        assert!(result.contains("overview"), "should suggest overview tool");
    }

    #[test]
    fn test_context_exact_match_preferred() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "index_repo".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn index_repo()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "open_or_index".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn open_or_index()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // Only request source section to avoid "Related" siblings
        let sections: &[&str] = &["source", "callers", "callees"];
        let result =
            handle_context(&db, "index_repo", false, None, Some(sections), None, false).unwrap();
        assert!(result.contains("index_repo"), "should find exact match");
        assert!(
            !result.contains("open_or_index"),
            "should NOT return fuzzy matches when exact match exists"
        );
    }

    #[test]
    fn test_callers_show_ref_line_not_definition_line() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "caller_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 10,
                    line_end: 50,
                    signature: "pub fn caller_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "target_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 60,
                    line_end: 70,
                    signature: "pub fn target_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let caller_id = db
            .get_symbol_id("caller_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        let target_id = db
            .get_symbol_id("target_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        // ref_line 42 = the line where caller_fn actually calls target_fn
        db.insert_symbol_ref(caller_id, target_id, "call", "high", Some(42))
            .unwrap();

        let result = handle_context(&db, "target_fn", false, None, None, None, false).unwrap();
        // Should show ref_line (42), not caller's definition line (10)
        assert!(
            result.contains("src/lib.rs:42"),
            "callers should show call-site line (42), got:\n{result}"
        );
        assert!(
            !result.contains("src/lib.rs:10"),
            "callers should NOT show definition line (10), got:\n{result}"
        );
    }

    #[test]
    fn test_context_callees_exclude_low_confidence() {
        let db = Database::open_in_memory().unwrap();
        let fid_a = db.insert_file("src/a.rs", "hash_a").unwrap();
        let fid_b = db.insert_file("src/b.rs", "hash_b").unwrap();
        store_symbols(
            &db,
            fid_a,
            &[
                Symbol {
                    name: "caller_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/a.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn caller_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "good_callee".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/a.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub fn good_callee()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();
        store_symbols(
            &db,
            fid_b,
            &[Symbol {
                name: "bad_callee".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/b.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn bad_callee()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("Unrelated".into()),
            }],
        )
        .unwrap();

        let caller_id = db.get_symbol_id("caller_fn", "src/a.rs").unwrap().unwrap();
        let good_id = db
            .get_symbol_id("good_callee", "src/a.rs")
            .unwrap()
            .unwrap();
        let bad_id = db.get_symbol_id("bad_callee", "src/b.rs").unwrap().unwrap();

        // High confidence ref
        db.insert_symbol_ref(caller_id, good_id, "call", "high", None)
            .unwrap();
        // Low confidence ref (name-only fallback)
        db.insert_symbol_ref(caller_id, bad_id, "call", "low", None)
            .unwrap();

        let result = handle_context(
            &db,
            "caller_fn",
            false,
            None,
            Some(&["callees"]),
            None,
            false,
        )
        .unwrap();
        assert!(
            result.contains("good_callee"),
            "should contain high-confidence callee, got:\n{result}"
        );
        assert!(
            !result.contains("bad_callee"),
            "should NOT contain low-confidence callee, got:\n{result}"
        );
        assert!(
            !result.contains("Unrelated"),
            "should NOT show impl_type of low-confidence callee, got:\n{result}"
        );
    }

    #[test]
    fn test_callers_production_before_tests() {
        let db = Database::open_in_memory().unwrap();
        let fid_lib = db.insert_file("src/lib.rs", "hash").unwrap();
        let fid_test = db.insert_file("src/tests.rs", "hash").unwrap();

        store_symbols(
            &db,
            fid_lib,
            &[
                Symbol {
                    name: "target_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "fn target_fn()".into(),
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
                    signature: "fn prod_caller()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        store_symbols(
            &db,
            fid_test,
            &[Symbol {
                name: "test_caller".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/tests.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "fn test_caller()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: Some("test".into()),
                impl_type: None,
            }],
        )
        .unwrap();

        let target_id = db
            .get_symbol_id("target_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        let prod_id = db
            .get_symbol_id("prod_caller", "src/lib.rs")
            .unwrap()
            .unwrap();
        let test_id = db
            .get_symbol_id("test_caller", "src/tests.rs")
            .unwrap()
            .unwrap();

        db.insert_symbol_ref(prod_id, target_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(test_id, target_id, "call", "high", None)
            .unwrap();

        let result = handle_context(
            &db,
            "target_fn",
            false,
            None,
            Some(&["callers"]),
            None,
            false,
        )
        .unwrap();
        let prod_pos = result.find("prod_caller").unwrap();
        let test_pos = result.find("test_caller").unwrap();
        assert!(
            prod_pos < test_pos,
            "production callers should appear before test callers\nOutput:\n{result}"
        );
    }
}
