use rusqlite::{Connection, Result as SqlResult, params};

pub struct Database {
    pub(crate) conn: Connection,
}

impl Database {
    pub fn open_in_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open(path: &std::path::Path) -> SqlResult<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> SqlResult<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (
                repo_path TEXT PRIMARY KEY,
                commit_hash TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                content_hash TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL REFERENCES files(id),
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                visibility TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                signature TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS symbol_refs (
                id INTEGER PRIMARY KEY,
                source_symbol_id INTEGER NOT NULL REFERENCES symbols(id),
                target_symbol_id INTEGER NOT NULL REFERENCES symbols(id),
                kind TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS dependencies (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                is_direct INTEGER NOT NULL,
                repository_url TEXT,
                features TEXT
            );

            CREATE TABLE IF NOT EXISTS docs (
                id INTEGER PRIMARY KEY,
                dependency_id INTEGER NOT NULL REFERENCES dependencies(id),
                source TEXT NOT NULL,
                content TEXT NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
                name, signature, content=symbols, content_rowid=id
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
                content, content=docs, content_rowid=id
            );",
        )
    }

    pub fn set_metadata(
        &self,
        repo_path: &str,
        commit_hash: &str,
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (repo_path, commit_hash)
             VALUES (?1, ?2)",
            params![repo_path, commit_hash],
        )?;
        Ok(())
    }

    pub fn get_commit_hash(
        &self,
        repo_path: &str,
    ) -> SqlResult<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT commit_hash FROM metadata WHERE repo_path = ?1",
        )?;
        let mut rows = stmt.query_map(params![repo_path], |row| {
            row.get(0)
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn get_direct_dependencies(
        &self,
    ) -> SqlResult<Vec<StoredDep>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, is_direct, repository_url, features \
             FROM dependencies WHERE is_direct = 1",
        )?;
        let mut deps = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            deps.push(StoredDep {
                name: row.get(0)?,
                version: row.get(1)?,
                is_direct: row.get(2)?,
                repository_url: row.get(3)?,
                features: row.get(4)?,
            });
        }
        Ok(deps)
    }

    pub fn insert_file(
        &self,
        path: &str,
        content_hash: &str,
    ) -> SqlResult<i64> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, content_hash) \
             VALUES (?1, ?2)",
            params![path, content_hash],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn search_symbols(
        &self,
        query: &str,
    ) -> SqlResult<Vec<StoredSymbol>> {
        let fts_query = format!("{query}*");
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature \
             FROM symbols_fts fts \
             JOIN symbols s ON s.id = fts.rowid \
             JOIN files f ON f.id = s.file_id \
             WHERE symbols_fts MATCH ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![fts_query])?;
        while let Some(row) = rows.next()? {
            results.push(StoredSymbol {
                name: row.get(0)?,
                kind: row.get(1)?,
                visibility: row.get(2)?,
                file_path: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
                signature: row.get(6)?,
            });
        }
        Ok(results)
    }

    pub fn get_dependency_by_name(
        &self,
        name: &str,
    ) -> SqlResult<Option<StoredDep>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, is_direct, repository_url, features \
             FROM dependencies WHERE name = ?1",
        )?;
        let mut rows = stmt.query(params![name])?;
        match rows.next()? {
            Some(row) => Ok(Some(StoredDep {
                name: row.get(0)?,
                version: row.get(1)?,
                is_direct: row.get(2)?,
                repository_url: row.get(3)?,
                features: row.get(4)?,
            })),
            None => Ok(None),
        }
    }

    pub fn insert_dependency(
        &self,
        name: &str,
        version: &str,
        is_direct: bool,
        repo_url: Option<&str>,
    ) -> SqlResult<i64> {
        self.conn.execute(
            "INSERT INTO dependencies \
             (name, version, is_direct, repository_url, features) \
             VALUES (?1, ?2, ?3, ?4, '')",
            params![name, version, is_direct, repo_url],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn store_doc(
        &self,
        dep_id: i64,
        source: &str,
        content: &str,
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT INTO docs (dependency_id, source, content) \
             VALUES (?1, ?2, ?3)",
            params![dep_id, source, content],
        )?;
        let rowid = self.conn.last_insert_rowid();
        self.conn.execute(
            "INSERT INTO docs_fts (rowid, content) \
             VALUES (?1, ?2)",
            params![rowid, content],
        )?;
        Ok(())
    }

    pub fn search_docs(
        &self,
        query: &str,
    ) -> SqlResult<Vec<DocResult>> {
        let fts_query = format!("{query}*");
        let mut stmt = self.conn.prepare(
            "SELECT d.content, d.source, dep.name, dep.version \
             FROM docs_fts fts \
             JOIN docs d ON d.id = fts.rowid \
             JOIN dependencies dep ON dep.id = d.dependency_id \
             WHERE docs_fts MATCH ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![fts_query])?;
        while let Some(row) = rows.next()? {
            results.push(DocResult {
                content: row.get(0)?,
                source: row.get(1)?,
                dependency_name: row.get(2)?,
                version: row.get(3)?,
            });
        }
        Ok(results)
    }

    pub fn get_docs_for_dependency(
        &self,
        name: &str,
    ) -> SqlResult<Vec<DocResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.content, d.source, dep.name, dep.version \
             FROM docs d \
             JOIN dependencies dep ON dep.id = d.dependency_id \
             WHERE dep.name = ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![name])?;
        while let Some(row) = rows.next()? {
            results.push(DocResult {
                content: row.get(0)?,
                source: row.get(1)?,
                dependency_name: row.get(2)?,
                version: row.get(3)?,
            });
        }
        Ok(results)
    }
}

#[derive(Debug)]
pub struct DocResult {
    pub content: String,
    pub source: String,
    pub dependency_name: String,
    pub version: String,
}

#[derive(Debug)]
pub struct StoredDep {
    pub name: String,
    pub version: String,
    pub is_direct: bool,
    pub repository_url: Option<String>,
    pub features: Option<String>,
}

#[derive(Debug)]
pub struct StoredSymbol {
    pub name: String,
    pub kind: String,
    pub visibility: String,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub signature: String,
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_creates_schema() {
        let db = Database::open_in_memory().unwrap();
        let tables: Vec<String> = db
            .conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type='table' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        assert!(tables.contains(&"metadata".to_string()));
        assert!(tables.contains(&"files".to_string()));
        assert!(tables.contains(&"symbols".to_string()));
        assert!(tables.contains(&"symbol_refs".to_string()));
        assert!(tables.contains(&"dependencies".to_string()));
        assert!(tables.contains(&"docs".to_string()));
    }

    #[test]
    fn test_fts5_tables_exist() {
        let db = Database::open_in_memory().unwrap();
        let tables: Vec<String> = db
            .conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type='table' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        assert!(tables.contains(&"symbols_fts".to_string()));
        assert!(tables.contains(&"docs_fts".to_string()));
    }

    #[test]
    fn test_metadata_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        db.set_metadata("/tmp/repo", "abc123").unwrap();
        let hash = db.get_commit_hash("/tmp/repo").unwrap();
        assert_eq!(hash, Some("abc123".to_string()));
    }

    #[test]
    fn test_metadata_missing() {
        let db = Database::open_in_memory().unwrap();
        let hash = db.get_commit_hash("/nonexistent").unwrap();
        assert_eq!(hash, None);
    }
}
