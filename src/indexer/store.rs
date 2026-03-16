use crate::db::Database;
use crate::indexer::dependencies::ResolvedDep;
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

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::dependencies::ResolvedDep;

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
}
