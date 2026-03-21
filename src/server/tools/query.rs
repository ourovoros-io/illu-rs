use crate::db::Database;
use std::fmt::Write;

#[expect(
    clippy::too_many_arguments,
    reason = "query has many independent filter params"
)]
pub fn handle_query(
    db: &Database,
    query: &str,
    scope: Option<&str>,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    path: Option<&str>,
    limit: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let scope = scope.unwrap_or("symbols");
    let mut output = String::new();

    match scope {
        "symbols" => {
            format_symbols(
                db,
                query,
                kind,
                attribute,
                signature,
                path,
                limit,
                &mut output,
            )?;
        }
        "docs" => format_docs(db, query, &mut output)?,
        "files" => format_files(db, query, path, limit, &mut output)?,
        "all" => {
            format_symbols(
                db,
                query,
                kind,
                attribute,
                signature,
                path,
                limit,
                &mut output,
            )?;
            format_docs(db, query, &mut output)?;
        }
        "doc_comments" => format_doc_comments(db, query, kind, path, limit, &mut output)?,
        "bodies" => format_body_search(db, query, kind, path, limit, &mut output)?,
        other => {
            return Err(format!(
                "Unknown scope: '{other}'. Valid: symbols, docs, files, doc_comments, bodies, all"
            )
            .into());
        }
    }

    if output.is_empty() {
        let _ = write!(output, "No results found for '{query}'.");
    }

    Ok(output)
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal format helper, mirrors handle_query params"
)]
fn format_symbols(
    db: &Database,
    query: &str,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    path: Option<&str>,
    limit: Option<i64>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let is_wildcard = query.is_empty() || query == "*";
    let mut all_symbols = if let Some(attr) = attribute {
        db.search_symbols_by_attribute(attr)?
    } else if !is_wildcard {
        db.search_symbols(query)?
    } else if let Some(p) = path {
        // Path is the most selective seed for wildcard queries;
        // check it before signature to avoid LIMIT truncation
        db.get_symbols_by_path_prefix(p)?
    } else if let Some(sig) = signature {
        db.search_symbols_by_signature(sig)?
    } else if kind.is_some() {
        db.get_symbols_by_path_prefix("")?
    } else {
        return Ok(());
    };
    if attribute.is_some() && !is_wildcard {
        let q = query.to_lowercase();
        all_symbols.retain(|s| s.name.to_lowercase().contains(&q));
    }
    if let Some(sig) = signature {
        let sig_lower = sig.to_lowercase();
        all_symbols.retain(|s| s.signature.to_lowercase().contains(&sig_lower));
    }
    if let Some(p) = path {
        all_symbols.retain(|s| s.file_path.starts_with(p));
    }
    let mut symbols: Vec<_> = if let Some(k) = kind {
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
    // Cap before expensive per-symbol ref count queries,
    // then sort by relevance, then apply user limit
    let max = limit
        .and_then(|l| usize::try_from(l.max(1)).ok())
        .unwrap_or(50);
    symbols.truncate(max.saturating_mul(4).min(200));
    sort_by_ref_count(db, &mut symbols)?;
    symbols.truncate(max);
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

fn sort_by_ref_count(
    db: &Database,
    symbols: &mut Vec<crate::db::StoredSymbol>,
) -> Result<(), Box<dyn std::error::Error>> {
    if symbols.len() <= 1 {
        return Ok(());
    }
    let mut with_counts: Vec<(i64, crate::db::StoredSymbol)> = Vec::with_capacity(symbols.len());
    for sym in symbols.drain(..) {
        let count = db.count_refs_for_symbol(&sym.name, &sym.file_path)?;
        with_counts.push((count, sym));
    }
    with_counts.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    symbols.extend(with_counts.into_iter().map(|(_, s)| s));
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
    limit: Option<i64>,
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
    if let Some(max) = limit {
        let max = usize::try_from(max.max(1)).unwrap_or(50);
        files.truncate(max);
    }

    if !files.is_empty() {
        output.push_str("## Files\n\n");
        for file in &files {
            let _ = writeln!(output, "- {file}");
        }
        output.push('\n');
    }
    Ok(())
}

fn format_doc_comments(
    db: &Database,
    query: &str,
    kind: Option<&str>,
    path: Option<&str>,
    limit: Option<i64>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut symbols = db.search_symbols_by_doc_comment(query)?;
    if let Some(p) = path {
        symbols.retain(|s| s.file_path.starts_with(p));
    }
    let mut symbols: Vec<_> = if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        symbols
            .into_iter()
            .filter(|s| s.kind.to_string().to_lowercase() == k_lower)
            .collect()
    } else {
        symbols
    };
    if let Some(max) = limit {
        let max = usize::try_from(max.max(1)).unwrap_or(50);
        symbols.truncate(max);
    }
    if !symbols.is_empty() {
        output.push_str("## Symbols matching doc comments\n\n");
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

fn format_body_search(
    db: &Database,
    query: &str,
    kind: Option<&str>,
    path: Option<&str>,
    limit: Option<i64>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut symbols = db.search_symbols_by_body(query)?;
    if let Some(p) = path {
        symbols.retain(|s| s.file_path.starts_with(p));
    }
    let mut symbols: Vec<_> = if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        symbols
            .into_iter()
            .filter(|s| s.kind.to_string().to_lowercase() == k_lower)
            .collect()
    } else {
        symbols
    };
    if let Some(max) = limit {
        let max = usize::try_from(max.max(1)).unwrap_or(50);
        symbols.truncate(max);
    }
    if !symbols.is_empty() {
        output.push_str("## Symbols matching body content\n\n");
        let query_lower = query.to_lowercase();
        for sym in &symbols {
            let qname = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qname}** ({}) at {}:{}-{}\n  `{}`",
                sym.kind, sym.file_path, sym.line_start, sym.line_end, sym.signature,
            );
            if let Some(body) = &sym.body {
                for line in body.lines() {
                    if line.to_lowercase().contains(&query_lower) {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            let _ = writeln!(output, "  > `{trimmed}`");
                            break;
                        }
                    }
                }
            }
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

    fn make_fn(name: &str, file: &str, line: usize, sig: &str) -> Symbol {
        Symbol {
            name: name.into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: file.into(),
            line_start: line,
            line_end: line + 5,
            signature: sig.into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }
    }

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

        let result =
            handle_query(&db, "parse", Some("symbols"), None, None, None, None, None).unwrap();
        assert!(!result.is_empty());
        assert!(result.contains("parse_config"));
    }

    #[test]
    fn test_query_docs() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde serialization framework")
            .unwrap();

        let result = handle_query(
            &db,
            "serialization",
            Some("docs"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
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

        let result = handle_query(&db, "serialize", None, None, None, None, None, None).unwrap();
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

        let result = handle_query(
            &db,
            "parse_config",
            Some("symbols"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
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

        let result = handle_query(
            &db,
            "Config",
            Some("symbols"),
            Some("struct"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(result.contains("Config"), "should find Config struct");
        assert!(
            !result.contains("configure"),
            "should not include functions"
        );

        let result = handle_query(
            &db,
            "Config",
            Some("symbols"),
            Some("function"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(!result.contains("Config"), "struct should be filtered out");
    }

    #[test]
    fn test_query_no_results() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_query(&db, "nonexistent", None, None, None, None, None, None).unwrap();
        assert!(result.contains("No results found for 'nonexistent'"));
    }

    #[test]
    fn test_query_path_filter() {
        let db = Database::open_in_memory().unwrap();
        let fa = db.insert_file("src/server/mod.rs", "ha").unwrap();
        let fb = db.insert_file("src/db.rs", "hb").unwrap();
        store_symbols(
            &db,
            fa,
            &[make_fn("serve", "src/server/mod.rs", 1, "pub fn serve()")],
        )
        .unwrap();
        store_symbols(
            &db,
            fb,
            &[make_fn("query_db", "src/db.rs", 1, "pub fn query_db()")],
        )
        .unwrap();

        // No path filter: both found
        let r = handle_query(
            &db,
            "",
            Some("symbols"),
            None,
            None,
            Some("pub fn"),
            None,
            None,
        )
        .unwrap();
        assert!(r.contains("serve"));
        assert!(r.contains("query_db"));

        // Filter to src/server/
        let r = handle_query(
            &db,
            "",
            Some("symbols"),
            None,
            None,
            Some("pub fn"),
            Some("src/server/"),
            None,
        )
        .unwrap();
        assert!(r.contains("serve"));
        assert!(!r.contains("query_db"));

        // Filter to src/db.rs
        let r = handle_query(
            &db,
            "",
            Some("symbols"),
            None,
            None,
            Some("pub fn"),
            Some("src/db.rs"),
            None,
        )
        .unwrap();
        assert!(!r.contains("serve"));
        assert!(r.contains("query_db"));

        // Nonexistent path
        let r = handle_query(
            &db,
            "",
            Some("symbols"),
            None,
            None,
            Some("pub fn"),
            Some("src/other/"),
            None,
        )
        .unwrap();
        assert!(r.contains("No results found"));
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
            &db,
            "",
            Some("symbols"),
            None,
            Some("test"),
            Some("Database"),
            None,
            None,
        )
        .unwrap();
        assert!(
            result.contains("test_fn"),
            "should find fn with both attribute and signature match"
        );
        assert!(
            !result.contains("other_test"),
            "should exclude fn without matching signature"
        );
    }

    #[test]
    fn test_query_doc_comments_scope() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "parse_config".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn parse_config() -> Config".into(),
                    doc_comment: Some(
                        "Parse configuration from TOML files.\nHandles errors gracefully.".into(),
                    ),
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "save_config".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn save_config()".into(),
                    doc_comment: Some("Save configuration to disk.".into()),
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "no_docs".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 22,
                    line_end: 25,
                    signature: "pub fn no_docs()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let result = handle_query(
            &db,
            "TOML",
            Some("doc_comments"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(
            result.contains("parse_config"),
            "should find symbol with TOML in doc"
        );
        assert!(
            !result.contains("save_config"),
            "should not find unrelated doc"
        );
        assert!(
            !result.contains("no_docs"),
            "should not find symbol without docs"
        );
        assert!(result.contains("Symbols matching doc comments"));

        let result = handle_query(
            &db,
            "nonexistent",
            Some("doc_comments"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(result.contains("No results found"));
    }

    #[test]
    fn test_query_bodies_scope() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "spawn_task".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn spawn_task()".into(),
                    doc_comment: None,
                    body: Some("tokio::spawn(async { do_work().await });".into()),
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "sync_work".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 12,
                    line_end: 20,
                    signature: "pub fn sync_work()".into(),
                    doc_comment: None,
                    body: Some("let result = compute(42);".into()),
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let result = handle_query(
            &db,
            "tokio::spawn",
            Some("bodies"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(
            result.contains("spawn_task"),
            "should find symbol with tokio::spawn in body"
        );
        assert!(
            !result.contains("sync_work"),
            "should not find unrelated symbol"
        );
        assert!(result.contains("Symbols matching body content"));
        assert!(
            result.contains("> `"),
            "should show body snippet for matching line"
        );
        assert!(
            result.contains("tokio::spawn"),
            "snippet should contain the matched term"
        );

        let result = handle_query(
            &db,
            "nonexistent_call",
            Some("bodies"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(result.contains("No results found"));
    }

    #[test]
    fn test_query_limit() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        let symbols: Vec<_> = (0..10)
            .map(|i| {
                make_fn(
                    &format!("fn_{i}"),
                    "src/lib.rs",
                    i * 10,
                    &format!("pub fn fn_{i}()"),
                )
            })
            .collect();
        store_symbols(&db, file_id, &symbols).unwrap();

        // Without limit: all 10
        let result =
            handle_query(&db, "fn_", Some("symbols"), None, None, None, None, None).unwrap();
        for i in 0..10 {
            assert!(result.contains(&format!("fn_{i}")));
        }

        // With limit=3: only 3
        let result =
            handle_query(&db, "fn_", Some("symbols"), None, None, None, None, Some(3)).unwrap();
        let count = result.matches("**fn_").count();
        assert_eq!(count, 3, "limit=3 should return 3 results, got {count}");
    }

    #[test]
    fn test_query_default_scope_is_symbols() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "parse".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn parse()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "parse and serialize data")
            .unwrap();

        // Default scope should NOT include docs
        let result = handle_query(&db, "parse", None, None, None, None, None, None).unwrap();
        assert!(result.contains("parse"), "should have symbol results");
        assert!(
            !result.contains("## Documentation"),
            "default scope should not include docs, got: {result}"
        );

        // Explicit "all" scope should include both
        let result = handle_query(&db, "parse", Some("all"), None, None, None, None, None).unwrap();
        assert!(result.contains("parse"));
        assert!(
            result.contains("## Documentation"),
            "explicit 'all' scope should include docs"
        );
    }

    #[test]
    fn test_query_wildcard_with_signature_filter() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "get_users".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn get_users() -> Vec<User>".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "count".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub fn count() -> usize".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // Wildcard query with signature filter
        let result =
            handle_query(&db, "*", None, None, None, Some("Vec<User>"), None, None).unwrap();
        assert!(
            result.contains("get_users"),
            "wildcard + signature should find matching symbols: {result}"
        );
        assert!(
            !result.contains("count"),
            "should not include non-matching signatures"
        );

        // Empty query with path filter
        let result = handle_query(&db, "", None, None, None, None, Some("src/"), None).unwrap();
        assert!(
            result.contains("get_users"),
            "empty query + path should list symbols: {result}"
        );
    }

    #[test]
    fn test_query_wildcard_kind_only() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
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
                },
                make_fn("my_func", "src/lib.rs", 7, "pub fn my_func()"),
            ],
        )
        .unwrap();

        let result = handle_query(
            &db,
            "*",
            Some("symbols"),
            Some("struct"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(
            result.contains("MyStruct"),
            "Should find struct with wildcard+kind: {result}"
        );
        assert!(
            !result.contains("my_func"),
            "Should NOT find function when kind=struct: {result}"
        );
    }

    #[test]
    fn test_query_results_sorted_by_ref_count() {
        use crate::indexer::parser::{RefKind, SymbolRef};

        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                make_fn("parse_alpha", "src/lib.rs", 1, "pub fn parse_alpha()"),
                make_fn("parse_beta", "src/lib.rs", 7, "pub fn parse_beta()"),
                make_fn("caller1", "src/lib.rs", 13, "pub fn caller1()"),
                make_fn("caller2", "src/lib.rs", 19, "pub fn caller2()"),
                make_fn("caller3", "src/lib.rs", 25, "pub fn caller3()"),
            ],
        )
        .unwrap();

        let map = db.build_symbol_id_map().unwrap();
        // parse_beta gets 3 refs, parse_alpha gets 1
        db.store_symbol_refs_fast(
            &[
                SymbolRef {
                    source_name: "caller1".into(),
                    target_name: "parse_beta".into(),
                    source_file: "src/lib.rs".into(),
                    target_file: Some("src/lib.rs".into()),
                    kind: RefKind::Call,
                    target_context: None,
                    ref_line: Some(14),
                },
                SymbolRef {
                    source_name: "caller2".into(),
                    target_name: "parse_beta".into(),
                    source_file: "src/lib.rs".into(),
                    target_file: Some("src/lib.rs".into()),
                    kind: RefKind::Call,
                    target_context: None,
                    ref_line: Some(20),
                },
                SymbolRef {
                    source_name: "caller3".into(),
                    target_name: "parse_beta".into(),
                    source_file: "src/lib.rs".into(),
                    target_file: Some("src/lib.rs".into()),
                    kind: RefKind::Call,
                    target_context: None,
                    ref_line: Some(26),
                },
                SymbolRef {
                    source_name: "caller1".into(),
                    target_name: "parse_alpha".into(),
                    source_file: "src/lib.rs".into(),
                    target_file: Some("src/lib.rs".into()),
                    kind: RefKind::Call,
                    target_context: None,
                    ref_line: Some(15),
                },
            ],
            &map,
        )
        .unwrap();

        let result =
            handle_query(&db, "parse", Some("symbols"), None, None, None, None, None).unwrap();
        // parse_beta (3 refs) should appear before parse_alpha (1 ref)
        let beta_pos = result.find("parse_beta").unwrap();
        let alpha_pos = result.find("parse_alpha").unwrap();
        assert!(
            beta_pos < alpha_pos,
            "parse_beta (3 refs) should appear before parse_alpha (1 ref)\n{result}"
        );
    }

    #[test]
    fn test_query_wildcard_kind_signature_path() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/db.rs", "hash").unwrap();
        let file_id2 = db.insert_file("src/other.rs", "hash2").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                make_fn("open", "src/db.rs", 1, "pub fn open() -> Result<Self>"),
                make_fn("close", "src/db.rs", 5, "pub fn close()"),
            ],
        )
        .unwrap();
        store_symbols(
            &db,
            file_id2,
            &[make_fn(
                "run",
                "src/other.rs",
                1,
                "pub fn run() -> Result<()>",
            )],
        )
        .unwrap();

        // Wildcard + kind + signature + path should find only
        // functions returning Result in src/db.rs
        let result = handle_query(
            &db,
            "*",
            Some("symbols"),
            Some("function"),
            None,
            Some("-> Result"),
            Some("src/db.rs"),
            None,
        )
        .unwrap();
        assert!(
            result.contains("open"),
            "Should find open() which returns Result in db.rs: {result}"
        );
        assert!(
            !result.contains("close"),
            "Should NOT find close() which has no Result return: {result}"
        );
        assert!(
            !result.contains("run"),
            "Should NOT find run() which is in other.rs: {result}"
        );
    }
}
