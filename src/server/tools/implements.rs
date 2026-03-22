use crate::db::Database;
use std::fmt::Write;

pub fn handle_implements(
    db: &Database,
    trait_name: Option<&str>,
    type_name: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();

    match (trait_name, type_name) {
        (Some(t), None) => {
            let impls = db.trait_impls_for_trait(t)?;
            let _ = writeln!(output, "## Types implementing `{t}`\n");
            if impls.is_empty() {
                let _ = writeln!(output, "No implementations found for trait `{t}`.");
            } else {
                for ti in &impls {
                    let _ = writeln!(
                        output,
                        "- **{}** ({}:{}-{})",
                        ti.type_name, ti.file_path, ti.line_start, ti.line_end
                    );
                }
            }
        }
        (None, Some(ty)) => {
            let impls = db.trait_impls_for_type(ty)?;
            let _ = writeln!(output, "## Traits implemented by `{ty}`\n");
            if impls.is_empty() {
                let _ = writeln!(output, "No trait implementations found for type `{ty}`.");
            } else {
                for ti in &impls {
                    let _ = writeln!(
                        output,
                        "- **{}** ({}:{}-{})",
                        ti.trait_name, ti.file_path, ti.line_start, ti.line_end
                    );
                }
            }
        }
        (Some(t), Some(ty)) => {
            let impls = db.trait_impls_for_type(ty)?;
            let filtered: Vec<_> = impls.iter().filter(|i| i.trait_name == t).collect();
            let _ = writeln!(output, "## `{ty}` implementation of `{t}`\n");
            if filtered.is_empty() {
                let _ = writeln!(output, "`{ty}` does not implement `{t}`.");
            } else {
                for ti in &filtered {
                    let _ = writeln!(
                        output,
                        "- {}:{}-{}",
                        ti.file_path, ti.line_start, ti.line_end
                    );
                }
            }
        }
        (None, None) => {
            let _ = writeln!(
                output,
                "Provide at least one of `trait_name` or `type_name`.\n\n\
                Examples:\n\
                - `trait_name: \"Display\"` → find all types implementing Display\n\
                - `type_name: \"MyStruct\"` → find all traits MyStruct implements\n\
                - Both → check if MyStruct implements Display"
            );
        }
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_implements_by_trait() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.insert_trait_impl("MyStruct", "Display", file_id, 10, 20)
            .unwrap();

        let result = handle_implements(&db, Some("Display"), None).unwrap();
        assert!(result.contains("Types implementing `Display`"));
        assert!(result.contains("MyStruct"));
        assert!(result.contains("src/lib.rs:10-20"));
    }

    #[test]
    fn test_implements_by_type() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.insert_trait_impl("MyStruct", "Display", file_id, 10, 20)
            .unwrap();
        db.insert_trait_impl("MyStruct", "Debug", file_id, 22, 30)
            .unwrap();

        let result = handle_implements(&db, None, Some("MyStruct")).unwrap();
        assert!(result.contains("Traits implemented by `MyStruct`"));
        assert!(result.contains("Display"));
        assert!(result.contains("Debug"));
    }

    #[test]
    fn test_implements_derive_traits() {
        use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
        use crate::indexer::store::{store_symbols, store_trait_impls};

        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        // Simulate a struct with derive attributes
        let sym = Symbol {
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
            attributes: Some("derive(Debug, Clone, Serialize)".into()),
            impl_type: None,
        };

        let mut trait_impls = Vec::new();
        crate::indexer::parser::extract_derive_trait_impls(&sym, &mut trait_impls);
        store_symbols(&db, file_id, &[sym]).unwrap();
        store_trait_impls(&db, file_id, &trait_impls).unwrap();

        // Should find Config for Debug
        let result = handle_implements(&db, Some("Debug"), None).unwrap();
        assert!(
            result.contains("Config"),
            "Debug should list Config: {result}"
        );

        // Should find Clone for Config
        let result = handle_implements(&db, None, Some("Config")).unwrap();
        assert!(
            result.contains("Debug"),
            "Config should show Debug: {result}"
        );
        assert!(
            result.contains("Clone"),
            "Config should show Clone: {result}"
        );
        assert!(
            result.contains("Serialize"),
            "Config should show Serialize: {result}"
        );
    }

    #[test]
    fn test_implements_thiserror_generates_display() {
        use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
        use crate::indexer::store::{store_symbols, store_trait_impls};

        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let sym = Symbol {
            name: "MyError".into(),
            kind: SymbolKind::Enum,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub enum MyError".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: Some("derive(thiserror::Error)".into()),
            impl_type: None,
        };

        let mut trait_impls = Vec::new();
        crate::indexer::parser::extract_derive_trait_impls(&sym, &mut trait_impls);
        store_symbols(&db, file_id, &[sym]).unwrap();
        store_trait_impls(&db, file_id, &trait_impls).unwrap();

        let result = handle_implements(&db, Some("Display"), None).unwrap();
        assert!(
            result.contains("MyError"),
            "thiserror::Error should generate Display impl: {result}"
        );
        let result = handle_implements(&db, Some("Error"), None).unwrap();
        assert!(
            result.contains("MyError"),
            "thiserror::Error should generate Error impl: {result}"
        );
    }

    #[test]
    fn test_implements_requires_one_param() {
        let db = Database::open_in_memory().unwrap();

        let result = handle_implements(&db, None, None).unwrap();
        assert!(result.contains("Provide"));
    }
}
