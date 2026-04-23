use crate::db::Database;
use std::fmt::Write;

pub fn handle_symbols_at(db: &Database, file: &str, line: i64) -> Result<String, crate::IlluError> {
    let symbols = db.get_symbols_at_lines(file, &[(line, line)])?;

    let mut output = String::new();

    if symbols.is_empty() {
        let _ = writeln!(
            output,
            "No symbols found at {file}:{line}.\n\
             The line may be in whitespace, a comment, or between definitions."
        );
        return Ok(output);
    }

    let _ = writeln!(output, "## Symbols at {file}:{line}\n");
    for sym in &symbols {
        let qname = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "- **{qname}** ({}) line {}-{}\n  `{}`",
            sym.kind, sym.line_start, sym.line_end, sym.signature
        );
        if let Some(doc) = &sym.doc_comment
            && let Some(first_line) = doc.lines().next()
        {
            let _ = writeln!(output, "  *{first_line}*");
        }
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    #[test]
    fn test_symbols_at_found() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let symbols = vec![Symbol {
            name: "do_stuff".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 5,
            line_end: 15,
            signature: "pub fn do_stuff() -> bool".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }];
        store_symbols(&db, file_id, &symbols).unwrap();

        let result = handle_symbols_at(&db, "src/lib.rs", 10).unwrap();
        assert!(result.contains("## Symbols at src/lib.rs:10"));
        assert!(result.contains("do_stuff"));
        assert!(result.contains("line 5-15"));
        assert!(result.contains("pub fn do_stuff() -> bool"));
    }

    #[test]
    fn test_symbols_at_not_found() {
        let db = Database::open_in_memory().unwrap();
        let _file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let result = handle_symbols_at(&db, "src/lib.rs", 100).unwrap();
        assert!(result.contains("No symbols found at src/lib.rs:100"));
    }

    #[test]
    fn test_symbols_at_overlapping() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let symbols = vec![
            // Outer impl block
            Symbol {
                name: "MyStruct".into(),
                kind: SymbolKind::Impl,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 50,
                signature: "impl MyStruct".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            // Method inside impl
            Symbol {
                name: "inner_method".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 10,
                line_end: 20,
                signature: "pub fn inner_method(&self)".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("MyStruct".into()),
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();

        let result = handle_symbols_at(&db, "src/lib.rs", 15).unwrap();
        assert!(result.contains("MyStruct"));
        assert!(result.contains("inner_method"));
    }
}
