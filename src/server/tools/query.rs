use crate::db::Database;
use std::fmt::Write;

pub fn handle_query(
    db: &Database,
    query: &str,
    scope: Option<&str>,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    path: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let scope = scope.unwrap_or("all");
    let mut output = String::new();

    match scope {
        "symbols" => {
            format_symbols(db, query, kind, attribute, signature, path, &mut output)?;
        }
        "docs" => format_docs(db, query, &mut output)?,
        "files" => format_files(db, query, path, &mut output)?,
        "all" => {
            format_symbols(db, query, kind, attribute, signature, path, &mut output)?;
            format_docs(db, query, &mut output)?;
        }
        other => {
            return Err(
                format!("Unknown scope: '{other}'. Valid: symbols, docs, files, all").into(),
            );
        }
    }

    if output.is_empty() {
        let _ = write!(output, "No results found for '{query}'.");
    }

    Ok(output)
}

fn format_symbols(
    db: &Database,
    query: &str,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    path: Option<&str>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut all_symbols = if let Some(attr) = attribute {
        db.search_symbols_by_attribute(attr)?
    } else if !query.is_empty() {
        db.search_symbols(query)?
    } else if let Some(sig) = signature {
        db.search_symbols_by_signature(sig)?
    } else {
        db.search_symbols(query)?
    };
    if attribute.is_some() && !query.is_empty() {
        let q = query.to_lowercase();
        all_symbols.retain(|s| s.name.to_lowercase().contains(&q));
    }
    if let Some(sig) = signature
        && (attribute.is_some() || !query.is_empty())
    {
        let sig_lower = sig.to_lowercase();
        all_symbols.retain(|s| s.signature.to_lowercase().contains(&sig_lower));
    }
    if let Some(p) = path {
        all_symbols.retain(|s| s.file_path.starts_with(p));
    }
    let symbols: Vec<_> = if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        all_symbols
            .into_iter()
            .filter(|s| s.kind.to_string().to_lowercase() == k_lower)
            .collect()
    } else {
        all_symbols
            .into_iter()
            .filter(|s| {
                s.kind != crate::indexer::parser::SymbolKind::Use
                    && s.kind != crate::indexer::parser::SymbolKind::Mod
                    && s.kind != crate::indexer::parser::SymbolKind::EnumVariant
            })
            .collect()
    };
    if !symbols.is_empty() {
        output.push_str("## Symbols\n\n");
        for sym in &symbols {
            let qname = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qname}** ({}) at {}:{}-{}\n  `{}`",
                sym.kind, sym.file_path, sym.line_start, sym.line_end, sym.signature,
            );
            if let Some(doc) = &sym.doc_comment
                && let Some(first_line) = doc.lines().next()
            {
                let _ = writeln!(output, "  *{first_line}*");
            }
        }
        output.push('\n');
    }
    Ok(())
}

fn format_docs(
    db: &Database,
    query: &str,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let docs = db.search_docs(query)?;
    if !docs.is_empty() {
        output.push_str("## Documentation\n\n");
        for doc in &docs {
            let snippet = super::truncate_snippet(&doc.content, 200);
            let _ = writeln!(
                output,
                "- **{} {}** ({})\n  {}",
                doc.dependency_name, doc.version, doc.source, &snippet,
            );
        }
        output.push('\n');
    }
    Ok(())
}

fn format_files(
    db: &Database,
    query: &str,
    path: Option<&str>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let all_paths = db.get_all_file_paths()?;
    let query_lower = query.to_lowercase();
    let mut files: Vec<&str> = all_paths
        .iter()
        .filter(|p| p.to_lowercase().contains(&query_lower))
        .filter(|p| path.is_none_or(|prefix| p.starts_with(prefix)))
        .map(String::as_str)
        .collect();
    files.sort_unstable();

    if !files.is_empty() {
        output.push_str("## Files\n\n");
        for file in &files {
            let _ = writeln!(output, "- {file}");
        }
        output.push('\n');
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
    fn test_query_symbols() {
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
                signature: "pub fn parse_config() -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let result = handle_query(&db, "parse", Some("symbols"), None, None, None, None).unwrap();
        assert!(!result.is_empty());
        assert!(result.contains("parse_config"));
    }

    #[test]
    fn test_query_docs() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde serialization framework")
            .unwrap();

        let result = handle_query(&db, "serialization", Some("docs"), None, None, None, None).unwrap();
        assert!(result.contains("serialization"));
    }

    #[test]
    fn test_query_all() {
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

        let result = handle_query(&db, "serialize", None, None, None, None, None).unwrap();
        assert!(result.contains("serialize"));
    }

    #[test]
    fn test_query_shows_doc_comment_snippet() {
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
                signature: "pub fn parse_config() -> Config".into(),
                doc_comment: Some("Parse configuration from file.\nSupports TOML and JSON.".into()),
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let result = handle_query(&db, "parse_config", Some("symbols"), None, None, None, None).unwrap();
        assert!(result.contains("*Parse configuration from file.*"));
        assert!(!result.contains("Supports TOML"));
    }

    #[test]
    fn test_query_kind_filter() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
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
                    name: "configure".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub fn configure()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let result = handle_query(&db, "Config", Some("symbols"), Some("struct"), None, None, None).unwrap();
        assert!(result.contains("Config"), "should find Config struct");
        assert!(
            !result.contains("configure"),
            "should not include functions"
        );

        let result = handle_query(&db, "Config", Some("symbols"), Some("function"), None, None, None).unwrap();
        assert!(!result.contains("Config"), "struct should be filtered out");
    }

    #[test]
    fn test_query_no_results() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_query(&db, "nonexistent", None, None, None, None, None).unwrap();
        assert!(result.contains("No results found for 'nonexistent'"));
    }

    #[test]
    fn test_query_path_filter() {
        let db = Database::open_in_memory().unwrap();
        let file_a = db.insert_file("src/server/mod.rs", "hash_a").unwrap();
        let file_b = db.insert_file("src/db.rs", "hash_b").unwrap();
        store_symbols(
            &db,
            file_a,
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
            file_b,
            &[Symbol {
                name: "query_db".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/db.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn query_db()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        // Without path filter: both symbols found via signature search
        let result = handle_query(&db, "", Some("symbols"), None, None, Some("pub fn"), None).unwrap();
        assert!(result.contains("serve"), "should find serve without path filter");
        assert!(result.contains("query_db"), "should find query_db without path filter");

        // With path filter to src/server/: only serve
        let result = handle_query(&db, "", Some("symbols"), None, None, Some("pub fn"), Some("src/server/")).unwrap();
        assert!(result.contains("serve"), "should find serve under src/server/");
        assert!(!result.contains("query_db"), "should not find query_db under src/server/");

        // With path filter to src/db.rs: only query_db
        let result = handle_query(&db, "", Some("symbols"), None, None, Some("pub fn"), Some("src/db.rs")).unwrap();
        assert!(!result.contains("serve"), "should not find serve under src/db.rs");
        assert!(result.contains("query_db"), "should find query_db under src/db.rs");

        // With path filter to nonexistent path: no results
        let result = handle_query(&db, "", Some("symbols"), None, None, Some("pub fn"), Some("src/other/")).unwrap();
        assert!(result.contains("No results found"), "no symbols under src/other/");
    }

    #[test]
    fn test_query_combined_attribute_and_signature() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "test_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn test_fn(db: &Database)".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
                Symbol {
                    name: "other_test".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub fn other_test()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // Combined: attribute=test AND signature contains Database
        let result = handle_query(
            &db, "", Some("symbols"), None,
            Some("test"), Some("Database"), None,
        )
        .unwrap();
        assert!(result.contains("test_fn"), "should find fn with both attribute and signature match");
        assert!(!result.contains("other_test"), "should exclude fn without matching signature");
    }
}
