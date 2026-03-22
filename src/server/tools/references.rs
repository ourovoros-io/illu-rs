use crate::db::Database;
use std::collections::HashSet;
use std::fmt::Write;

pub fn handle_references(
    db: &Database,
    symbol_name: &str,
    path: Option<&str>,
    exclude_tests: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(super::symbol_not_found(db, symbol_name));
    }

    let mut output = String::new();
    let _ = writeln!(output, "## References: {symbol_name}\n");

    // Definition(s)
    let _ = writeln!(output, "### Definition\n");
    for sym in &symbols {
        let qname = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "- **{qname}** ({}) at {}:{}-{}",
            sym.kind, sym.file_path, sym.line_start, sym.line_end
        );
    }

    let call_count = render_call_sites(db, &symbols, path, exclude_tests, &mut output)?;
    // For type usage, use the type name (not the method name) for Type::method symbols
    let type_search_name = if let Some((type_name, _method)) = symbol_name.split_once("::") {
        type_name
    } else {
        symbol_name
    };
    let type_count = render_type_usage(db, type_search_name, path, &mut output)?;
    let impl_count = render_trait_impls(db, type_search_name, &mut output)?;

    let _ = writeln!(
        output,
        "\n---\n**Summary:** {call_count} call site(s), \
         {type_count} signature usage(s), {impl_count} trait impl(s)",
    );

    Ok(output)
}

fn render_call_sites(
    db: &Database,
    symbols: &[crate::db::StoredSymbol],
    path: Option<&str>,
    exclude_tests: bool,
    output: &mut String,
) -> Result<usize, Box<dyn std::error::Error>> {
    let _ = writeln!(output, "\n### Call Sites\n");
    let mut seen: HashSet<(String, String, i64)> =
        HashSet::new();
    let mut prod = Vec::new();
    let mut test = Vec::new();
    for sym in symbols {
        for c in db.callers(&sym.name, &sym.file_path, false, Some("high"))? {
            if path.is_some_and(|p| !c.file_path.starts_with(p)) {
                continue;
            }
            let line = c.ref_line.unwrap_or(c.line_start);
            if seen.insert((c.name.clone(), c.file_path.clone(), line)) {
                if c.is_test {
                    if !exclude_tests {
                        test.push((c.name, c.file_path, line));
                    }
                } else {
                    prod.push((c.name, c.file_path, line));
                }
            }
        }
    }
    let count = prod.len() + test.len();
    for (name, file, line) in &prod {
        let _ = writeln!(output, "- {name} ({file}:{line})");
    }
    if !prod.is_empty() && !test.is_empty() {
        output.push('\n');
    }
    for (name, file, line) in &test {
        let _ = writeln!(output, "- {name} ({file}:{line})");
    }
    if count == 0 {
        let _ = writeln!(output, "No call sites found.");
    }
    Ok(count)
}

fn render_type_usage(
    db: &Database,
    base_name: &str,
    path: Option<&str>,
    output: &mut String,
) -> Result<usize, Box<dyn std::error::Error>> {
    let _ = writeln!(output, "\n### Type Usage in Signatures\n");
    let sig_results = db.search_symbols_by_signature(base_name)?;
    let mut entries = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for s in &sig_results {
        if s.name == base_name
            || s.kind == crate::indexer::parser::SymbolKind::Use
            || !super::type_usage::contains_whole_word(&s.signature, base_name)
        {
            continue;
        }
        if path.is_some_and(|p| !s.file_path.starts_with(p)) {
            continue;
        }
        let qname = super::qualified_name(s);
        if seen.insert((qname.clone(), s.file_path.clone())) {
            entries.push((qname, s.signature.clone()));
        }
    }
    for (qname, sig) in &entries {
        let _ = writeln!(
            output,
            "- **{qname}** — `{}`",
            super::truncate_snippet(sig, 120)
        );
    }
    if entries.is_empty() {
        let _ = writeln!(output, "Not used in any signatures.");
    }
    Ok(entries.len())
}

fn render_trait_impls(
    db: &Database,
    base_name: &str,
    output: &mut String,
) -> Result<usize, Box<dyn std::error::Error>> {
    let _ = writeln!(output, "\n### Trait Implementations\n");
    let type_impls = db.trait_impls_for_type(base_name)?;
    let trait_impls = db.trait_impls_for_trait(base_name)?;
    if type_impls.is_empty() && trait_impls.is_empty() {
        let _ = writeln!(output, "No trait implementations found.");
    }
    for ti in type_impls.iter().chain(&trait_impls) {
        let _ = writeln!(
            output,
            "- `{}` implements `{}` ({}:{}-{})",
            ti.type_name, ti.trait_name, ti.file_path, ti.line_start, ti.line_end
        );
    }
    Ok(type_impls.len() + trait_impls.len())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Confidence, RefKind, Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        let symbols = vec![
            Symbol {
                name: "Config".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "load".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn load() -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();
        db
    }

    #[test]
    fn test_references_found() {
        let db = setup_db();
        let result = handle_references(&db, "Config", None, false).unwrap();
        assert!(result.contains("## References: Config"));
        assert!(result.contains("### Definition"));
        assert!(result.contains("Config"));
    }

    #[test]
    fn test_references_not_found() {
        let db = setup_db();
        let result = handle_references(&db, "Nonexistent", None, false).unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_references_type_usage() {
        let db = setup_db();
        let result = handle_references(&db, "Config", None, false).unwrap();
        assert!(result.contains("Type Usage"));
        assert!(result.contains("load"));
    }

    #[test]
    fn test_references_dedup_multi_definition() {
        // Simulate a symbol with multiple definition rows (enum + impl blocks)
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        let symbols = vec![
            Symbol {
                name: "Status".into(),
                kind: SymbolKind::Enum,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub enum Status".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "Status".into(),
                kind: SymbolKind::Impl,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 15,
                signature: "impl Status".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "caller".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 20,
                line_end: 25,
                signature: "pub fn caller(s: Status)".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();

        // caller references both Status definitions
        let enum_id = db.symbol_id("Status", "src/lib.rs").unwrap().unwrap();
        let caller_id = db.symbol_id("caller", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(caller_id, enum_id, RefKind::Call, Confidence::High, Some(22))
            .unwrap();

        let result = handle_references(&db, "Status", None, false).unwrap();
        // caller should appear exactly once despite multiple Status definitions
        let caller_count = result.matches("caller (src/lib.rs:").count();
        assert_eq!(
            caller_count, 1,
            "caller should appear exactly once, got {caller_count}: {result}"
        );
    }

    #[test]
    fn test_references_separates_prod_and_test_callers() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        let symbols = vec![
            Symbol {
                name: "target".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn target()".into(),
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
                name: "test_caller".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 15,
                signature: "fn test_caller()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: Some("test".into()),
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();

        let target_id = db.symbol_id("target", "src/lib.rs").unwrap().unwrap();
        let prod_id = db
            .symbol_id("prod_caller", "src/lib.rs")
            .unwrap()
            .unwrap();
        let test_id = db
            .symbol_id("test_caller", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(prod_id, target_id, RefKind::Call, Confidence::High, Some(8))
            .unwrap();
        db.insert_symbol_ref(test_id, target_id, RefKind::Call, Confidence::High, Some(13))
            .unwrap();

        let result = handle_references(&db, "target", None, false).unwrap();
        // Production callers should appear before test callers
        let prod_pos = result.find("prod_caller").unwrap();
        let test_pos = result.find("test_caller").unwrap();
        assert!(
            prod_pos < test_pos,
            "production callers should appear before test callers: {result}"
        );
    }

    #[test]
    fn test_references_exclude_tests() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        let symbols = vec![
            Symbol {
                name: "target".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn target()".into(),
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
                name: "test_caller".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 15,
                signature: "fn test_caller()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: Some("test".into()),
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();

        let target_id = db.symbol_id("target", "src/lib.rs").unwrap().unwrap();
        let prod_id = db
            .symbol_id("prod_caller", "src/lib.rs")
            .unwrap()
            .unwrap();
        let test_id = db
            .symbol_id("test_caller", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(prod_id, target_id, RefKind::Call, Confidence::High, Some(8))
            .unwrap();
        db.insert_symbol_ref(test_id, target_id, RefKind::Call, Confidence::High, Some(13))
            .unwrap();

        let result = handle_references(&db, "target", None, true).unwrap();
        assert!(
            result.contains("prod_caller"),
            "production caller should appear: {result}"
        );
        assert!(
            !result.contains("test_caller"),
            "test caller should be excluded: {result}"
        );
        assert!(
            result.contains("1 call site"),
            "summary should count only prod callers: {result}"
        );
    }

    #[test]
    fn test_references_excludes_use_from_type_usage() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        let f2 = db.insert_file("src/other.rs", "hash2").unwrap();
        let symbols = vec![
            Symbol {
                name: "Config".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "load".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn load() -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();
        // A `use` import that mentions Config in its signature
        store_symbols(
            &db,
            f2,
            &[Symbol {
                name: "use crate::Config;".into(),
                kind: SymbolKind::Use,
                visibility: Visibility::Private,
                file_path: "src/other.rs".into(),
                line_start: 1,
                line_end: 1,
                signature: "use crate::Config;".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let result = handle_references(&db, "Config", None, false).unwrap();
        // The `use` statement should NOT appear in Type Usage
        assert!(
            !result.contains("use crate::Config"),
            "use imports should be filtered from type usage: {result}"
        );
        // But the real type usage (load) should still appear
        assert!(
            result.contains("load"),
            "real type usage should still appear: {result}"
        );
    }
}
