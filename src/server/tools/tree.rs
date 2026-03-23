use crate::db::Database;
use std::fmt::Write;

pub fn handle_tree(db: &Database, path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let file_counts = db.get_file_symbol_counts(path)?;

    if file_counts.is_empty() {
        return Ok(format!(
            "No files found under '{path}'. \
             Try 'src/' for standard Rust layout."
        ));
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Module Tree: {path}\n");

    let mut total_symbols = 0i64;
    for fc in &file_counts {
        total_symbols += fc.count;
        let _ = writeln!(output, "- `{}` ({} symbols)", fc.path, fc.count);
    }

    let _ = writeln!(
        output,
        "\n**Total:** {} files, {} public symbols",
        file_counts.len(),
        total_symbols,
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
    fn test_tree_basic() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "h1").unwrap();
        let f2 = db.insert_file("src/server/mod.rs", "h2").unwrap();
        store_symbols(
            &db,
            f1,
            &[Symbol {
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
            }],
        )
        .unwrap();
        store_symbols(
            &db,
            f2,
            &[
                Symbol {
                    name: "serve".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/server/mod.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub fn serve()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "helper".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/server/mod.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "fn helper()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let result = handle_tree(&db, "src/").unwrap();
        assert!(result.contains("src/lib.rs"), "should list lib.rs");
        assert!(
            result.contains("src/server/mod.rs"),
            "should list server/mod.rs"
        );
        assert!(result.contains("2 files"), "should count files");
        assert!(
            result.contains("2 public symbols"),
            "should count only public symbols"
        );
    }

    #[test]
    fn test_tree_no_files() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_tree(&db, "nonexistent/").unwrap();
        assert!(result.contains("No files found"));
    }
}
