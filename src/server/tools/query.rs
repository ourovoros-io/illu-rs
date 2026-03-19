use crate::db::Database;
use std::fmt::Write;

pub fn handle_query(
    db: &Database,
    query: &str,
    scope: Option<&str>,
    kind: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let scope = scope.unwrap_or("all");
    let mut output = String::new();

    match scope {
        "symbols" => format_symbols(db, query, kind, &mut output)?,
        "docs" => format_docs(db, query, &mut output)?,
        "files" => format_files(db, query, &mut output)?,
        "all" => {
            format_symbols(db, query, kind, &mut output)?;
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
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let all_symbols = db.search_symbols(query)?;
    let symbols: Vec<_> = if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        all_symbols
            .into_iter()
            .filter(|s| s.kind.to_string().to_lowercase() == k_lower)
            .collect()
    } else {
        all_symbols
    };
    if !symbols.is_empty() {
        output.push_str("## Symbols\n\n");
        for sym in &symbols {
            let _ = writeln!(
                output,
                "- **{}** ({}) at {}:{}-{}\n  `{}`",
                sym.name, sym.kind, sym.file_path, sym.line_start, sym.line_end, sym.signature,
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
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let symbols = db.search_symbols(query)?;
    let mut files: Vec<&str> = symbols.iter().map(|s| s.file_path.as_str()).collect();
    files.sort_unstable();
    files.dedup();

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

        let result = handle_query(&db, "parse", Some("symbols"), None).unwrap();
        assert!(!result.is_empty());
        assert!(result.contains("parse_config"));
    }

    #[test]
    fn test_query_docs() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde serialization framework")
            .unwrap();

        let result = handle_query(&db, "serialization", Some("docs"), None).unwrap();
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

        let result = handle_query(&db, "serialize", None, None).unwrap();
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

        let result = handle_query(&db, "parse_config", Some("symbols"), None).unwrap();
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

        let result = handle_query(&db, "Config", Some("symbols"), Some("struct")).unwrap();
        assert!(result.contains("Config"), "should find Config struct");
        assert!(
            !result.contains("configure"),
            "should not include functions"
        );

        let result = handle_query(&db, "Config", Some("symbols"), Some("function")).unwrap();
        assert!(!result.contains("Config"), "struct should be filtered out");
    }

    #[test]
    fn test_query_no_results() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_query(&db, "nonexistent", None, None).unwrap();
        assert!(result.contains("No results found for 'nonexistent'"));
    }
}
