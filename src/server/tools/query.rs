use crate::db::Database;
use std::fmt::Write;

pub fn handle_query(
    db: &Database,
    query: &str,
    scope: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let scope = scope.unwrap_or("all");
    let mut output = String::new();

    match scope {
        "symbols" => format_symbols(db, query, &mut output)?,
        "docs" => format_docs(db, query, &mut output)?,
        "files" => format_files(db, query, &mut output)?,
        "all" => {
            format_symbols(db, query, &mut output)?;
            format_docs(db, query, &mut output)?;
        }
        other => {
            return Err(format!("Unknown scope: {other}").into());
        }
    }

    if output.is_empty() {
        output.push_str("No results found.");
    }

    Ok(output)
}

fn format_symbols(
    db: &Database,
    query: &str,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let symbols = db.search_symbols(query)?;
    if !symbols.is_empty() {
        output.push_str("## Symbols\n\n");
        for sym in &symbols {
            let _ = writeln!(
                output,
                "- **{}** ({}) at {}:{}-{}\n  `{}`",
                sym.name, sym.kind, sym.file_path, sym.line_start, sym.line_end, sym.signature,
            );
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
            let snippet = if doc.content.len() > 200 {
                format!("{}...", &doc.content[..200])
            } else {
                doc.content.clone()
            };
            let _ = writeln!(
                output,
                "- **{} {}** ({})\n  {}",
                doc.dependency_name, doc.version, doc.source, snippet,
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
            }],
        )
        .unwrap();

        let result = handle_query(&db, "parse", Some("symbols")).unwrap();
        assert!(!result.is_empty());
        assert!(result.contains("parse_config"));
    }

    #[test]
    fn test_query_docs() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde serialization framework")
            .unwrap();

        let result = handle_query(&db, "serialization", Some("docs")).unwrap();
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
            }],
        )
        .unwrap();

        let result = handle_query(&db, "serialize", None).unwrap();
        assert!(result.contains("serialize"));
    }

    #[test]
    fn test_query_no_results() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_query(&db, "nonexistent", None).unwrap();
        assert_eq!(result, "No results found.");
    }
}
