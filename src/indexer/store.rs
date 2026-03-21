use crate::db::{Database, FileId};
use crate::indexer::dependencies::ResolvedDep;
use crate::indexer::parser::Symbol;
use rusqlite::params;

pub fn store_dependencies(db: &Database, deps: &[ResolvedDep]) -> rusqlite::Result<()> {
    db.with_transaction(|db| {
        let mut stmt = db.conn.prepare(
            "INSERT INTO dependencies \
             (name, version, is_direct, repository_url, features) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for dep in deps {
            let features = dep.features.join(",");
            stmt.execute(params![
                dep.name,
                dep.version,
                dep.is_direct,
                dep.repository_url,
                features,
            ])?;
        }
        Ok(())
    })
}

pub fn store_symbols(db: &Database, file_id: FileId, symbols: &[Symbol]) -> rusqlite::Result<()> {
    db.with_transaction(|db| {
        let mut sym_stmt = db.conn.prepare(
            "INSERT INTO symbols \
             (file_id, name, kind, visibility, \
              line_start, line_end, signature, \
              doc_comment, body, details, attributes, impl_type, is_test) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, \
                     ?8, ?9, ?10, ?11, ?12, ?13)",
        )?;
        let mut fts_stmt = db.conn.prepare(
            "INSERT INTO symbols_fts \
             (rowid, name, signature, doc_comment) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        let mut trigram_stmt = db.conn.prepare(
            "INSERT INTO symbols_trigram (rowid, name) \
             VALUES (?1, ?2)",
        )?;
        for sym in symbols {
            let line_start = i64::try_from(sym.line_start).unwrap_or(i64::MAX);
            let line_end = i64::try_from(sym.line_end).unwrap_or(i64::MAX);
            let is_test = sym
                .attributes
                .as_deref()
                .is_some_and(|a| a.contains("test"));
            sym_stmt.execute(params![
                file_id,
                sym.name,
                sym.kind.to_string(),
                sym.visibility.to_string(),
                line_start,
                line_end,
                sym.signature,
                sym.doc_comment,
                sym.body,
                sym.details,
                sym.attributes,
                sym.impl_type,
                is_test,
            ])?;
            let rowid = db.conn.last_insert_rowid();
            let doc_for_fts = sym.doc_comment.as_deref().unwrap_or("");
            fts_stmt.execute(params![rowid, sym.name, sym.signature, doc_for_fts])?;
            trigram_stmt.execute(params![rowid, sym.name])?;
        }
        Ok(())
    })
}

pub fn store_trait_impls(
    db: &Database,
    file_id: FileId,
    trait_impls: &[crate::indexer::parser::TraitImpl],
) -> rusqlite::Result<()> {
    if trait_impls.is_empty() {
        return Ok(());
    }
    db.with_transaction(|db| {
        let mut stmt = db.conn.prepare(
            "INSERT OR IGNORE INTO trait_impls \
             (type_name, trait_name, file_id, line_start, line_end) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for ti in trait_impls {
            let line_start = i64::try_from(ti.line_start).unwrap_or(i64::MAX);
            let line_end = i64::try_from(ti.line_end).unwrap_or(i64::MAX);
            stmt.execute(params![
                ti.type_name,
                ti.trait_name,
                file_id,
                line_start,
                line_end
            ])?;
        }
        Ok(())
    })
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::dependencies::ResolvedDep;
    use crate::indexer::parser::{SymbolKind, Visibility};

    #[test]
    fn test_store_and_retrieve_dependencies() {
        let db = Database::open_in_memory().unwrap();
        let deps = vec![ResolvedDep {
            name: "serde".into(),
            version: "1.0.210".into(),
            is_direct: true,
            repository_url: Some("https://github.com/serde-rs/serde".into()),
            features: vec!["derive".into()],
        }];
        store_dependencies(&db, &deps).unwrap();
        let stored = db.get_direct_dependencies().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "serde");
        assert_eq!(stored[0].version, "1.0.210");
    }

    #[test]
    fn test_get_dependency_by_name() {
        let db = Database::open_in_memory().unwrap();
        let deps = vec![
            ResolvedDep {
                name: "serde".into(),
                version: "1.0.210".into(),
                is_direct: true,
                repository_url: None,
                features: vec![],
            },
            ResolvedDep {
                name: "tokio".into(),
                version: "1.0.0".into(),
                is_direct: true,
                repository_url: None,
                features: vec![],
            },
        ];
        store_dependencies(&db, &deps).unwrap();
        let dep = db.get_dependency_by_name("serde").unwrap();
        assert!(dep.is_some());
        assert_eq!(dep.unwrap().version, "1.0.210");

        let missing = db.get_dependency_by_name("nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_store_and_search_symbols() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "abc123").unwrap();
        let symbols = vec![Symbol {
            name: "parse_config".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 10,
            line_end: 25,
            signature: "pub fn parse_config(path: &Path) -> Result<Config>".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }];
        store_symbols(&db, file_id, &symbols).unwrap();
        let results = db.search_symbols("parse").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "parse_config");
    }

    #[test]
    fn test_store_and_search_docs() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db
            .insert_dependency("serde", "1.0.210", true, None)
            .unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde is a serialization framework")
            .unwrap();
        let results = db.search_docs("serialization").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("serialization"));
    }

    #[test]
    fn test_get_docs_for_dependency() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("tokio", "1.0.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Async runtime").unwrap();
        db.store_doc(dep_id, "github_readme", "Tokio README")
            .unwrap();
        let docs = db.get_docs_for_dependency("tokio").unwrap();
        assert_eq!(docs.len(), 2);
    }

    #[test]
    fn test_store_symbols_with_new_fields() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "abc123").unwrap();
        let symbols = vec![Symbol {
            name: "Config".into(),
            kind: SymbolKind::Struct,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 5,
            line_end: 15,
            signature: "pub struct Config".into(),
            doc_comment: Some("Configuration for the app.".into()),
            body: Some("pub struct Config { pub port: u16 }".into()),
            details: Some("fields: port: u16".into()),
            attributes: None,
            impl_type: None,
        }];
        store_symbols(&db, file_id, &symbols).unwrap();

        let results = db.search_symbols("Config").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Config");

        // Verify doc_comment is searchable via FTS
        let doc_results = db.search_symbols("Configuration").unwrap();
        assert_eq!(doc_results.len(), 1);
        assert_eq!(doc_results[0].name, "Config");
    }

    #[test]
    fn test_store_and_query_trait_impls() {
        use crate::indexer::parser::TraitImpl;

        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "abc123").unwrap();
        let impls = vec![
            TraitImpl {
                type_name: "Config".into(),
                trait_name: "Display".into(),
                file_path: "src/lib.rs".into(),
                line_start: 20,
                line_end: 30,
            },
            TraitImpl {
                type_name: "Config".into(),
                trait_name: "Debug".into(),
                file_path: "src/lib.rs".into(),
                line_start: 32,
                line_end: 40,
            },
        ];
        store_trait_impls(&db, file_id, &impls).unwrap();

        let stored = db.get_trait_impls_for_type("Config").unwrap();
        assert_eq!(stored.len(), 2);

        let trait_names: Vec<&str> = stored.iter().map(|i| i.trait_name.as_str()).collect();
        assert!(trait_names.contains(&"Display"));
        assert!(trait_names.contains(&"Debug"));

        let by_trait = db.get_trait_impls_for_trait("Display").unwrap();
        assert_eq!(by_trait.len(), 1);
        assert_eq!(by_trait[0].type_name, "Config");
    }
}
