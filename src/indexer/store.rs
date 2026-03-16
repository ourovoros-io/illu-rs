use crate::db::Database;
use crate::indexer::dependencies::ResolvedDep;
use crate::indexer::parser::Symbol;
use rusqlite::params;

pub fn store_dependencies(
    db: &Database,
    deps: &[ResolvedDep],
) -> rusqlite::Result<()> {
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
}

pub fn store_symbols(
    db: &Database,
    file_id: i64,
    symbols: &[Symbol],
) -> rusqlite::Result<()> {
    let mut sym_stmt = db.conn.prepare(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut fts_stmt = db.conn.prepare(
        "INSERT INTO symbols_fts (rowid, name, signature) \
         VALUES (?1, ?2, ?3)",
    )?;
    for sym in symbols {
        let line_start = i64::try_from(sym.line_start)
            .unwrap_or(i64::MAX);
        let line_end = i64::try_from(sym.line_end)
            .unwrap_or(i64::MAX);
        sym_stmt.execute(params![
            file_id,
            sym.name,
            sym.kind.to_string(),
            sym.visibility.to_string(),
            line_start,
            line_end,
            sym.signature,
        ])?;
        let rowid = db.conn.last_insert_rowid();
        fts_stmt.execute(params![rowid, sym.name, sym.signature])?;
    }
    Ok(())
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
            repository_url: Some(
                "https://github.com/serde-rs/serde".into(),
            ),
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
            signature: "pub fn parse_config(path: &Path) -> Result<Config>"
                .into(),
        }];
        store_symbols(&db, file_id, &symbols).unwrap();
        let results = db.search_symbols("parse").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "parse_config");
    }

    #[test]
    fn test_store_and_search_docs() {
        let db = Database::open_in_memory().unwrap();
        let dep_id =
            db.insert_dependency("serde", "1.0.210", true, None)
                .unwrap();
        db.store_doc(
            dep_id,
            "docs.rs",
            "Serde is a serialization framework",
        )
        .unwrap();
        let results = db.search_docs("serialization").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("serialization"));
    }

    #[test]
    fn test_get_docs_for_dependency() {
        let db = Database::open_in_memory().unwrap();
        let dep_id =
            db.insert_dependency("tokio", "1.0.0", true, None)
                .unwrap();
        db.store_doc(dep_id, "docs.rs", "Async runtime").unwrap();
        db.store_doc(dep_id, "github_readme", "Tokio README")
            .unwrap();
        let docs = db.get_docs_for_dependency("tokio").unwrap();
        assert_eq!(docs.len(), 2);
    }
}
