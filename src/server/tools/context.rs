use crate::db::{Database, StoredSymbol};
use crate::indexer::parser::SymbolKind;
use std::fmt::Write;
use std::path::Path;

pub fn handle_context(
    db: &Database,
    symbol_name: &str,
    full_body: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.search_symbols(symbol_name)?;
    if symbols.is_empty() {
        return Ok(format!("No symbol found matching '{symbol_name}'."));
    }

    let repo_root = db.repo_root();
    let mut output = String::new();

    for sym in &symbols {
        render_symbol_header(&mut output, sym);
        render_symbol_details(&mut output, sym, full_body, repo_root);
        render_trait_info(db, &mut output, sym)?;
        render_callees(db, &mut output, sym)?;
    }

    render_related_docs(db, &mut output, symbol_name)?;

    Ok(output)
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

fn render_callees(
    db: &Database,
    output: &mut String,
    sym: &StoredSymbol,
) -> Result<(), Box<dyn std::error::Error>> {
    let callees = db.get_callees(&sym.name)?;
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
            let _ = writeln!(output, "- {} ({})", c.name, c.file_path);
        }
        let _ = writeln!(output);
    }

    if !type_refs.is_empty() {
        let _ = writeln!(output, "**Uses types:**");
        for c in &type_refs {
            let _ = writeln!(output, "- {} ({})", c.name, c.file_path);
        }
        let _ = writeln!(output);
    }

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
            }],
        )
        .unwrap();

        let result = handle_context(&db, "parse_config", false).unwrap();
        assert!(result.contains("parse_config"));
        assert!(result.contains("src/lib.rs"));
        assert!(result.contains("public"));
    }

    #[test]
    fn test_context_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_context(&db, "nonexistent", false).unwrap();
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
            }],
        )
        .unwrap();

        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "serialize and deserialize data")
            .unwrap();

        let result = handle_context(&db, "serialize", false).unwrap();
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
            }],
        )
        .unwrap();

        let result = handle_context(&db, "Config", false).unwrap();
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
        db.insert_symbol_ref(caller_id, callee_id, "call").unwrap();

        let result = handle_context(&db, "caller_fn", false).unwrap();
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
            }],
        )
        .unwrap();

        db.insert_trait_impl("MyStruct", "Display", file_id, 10, 20)
            .unwrap();
        db.insert_trait_impl("MyStruct", "Debug", file_id, 22, 30)
            .unwrap();

        let result = handle_context(&db, "MyStruct", false).unwrap();
        assert!(result.contains("### Trait Implementations"));
        assert!(result.contains("**Display**"));
        assert!(result.contains("**Debug**"));
    }
}
