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
                content_hash TEXT NOT NULL,
                crate_id INTEGER REFERENCES crates(id)
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL REFERENCES files(id),
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                visibility TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                signature TEXT NOT NULL,
                doc_comment TEXT,
                body TEXT,
                details TEXT
            );

            CREATE TABLE IF NOT EXISTS trait_impls (
                id INTEGER PRIMARY KEY,
                type_name TEXT NOT NULL,
                trait_name TEXT NOT NULL,
                file_id INTEGER NOT NULL REFERENCES files(id),
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                UNIQUE(type_name, trait_name, file_id)
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

            CREATE TABLE IF NOT EXISTS crates (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                path TEXT NOT NULL,
                is_workspace_root INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS crate_deps (
                source_crate_id INTEGER NOT NULL REFERENCES crates(id),
                target_crate_id INTEGER NOT NULL REFERENCES crates(id),
                PRIMARY KEY (source_crate_id, target_crate_id)
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
                name, signature, doc_comment, content=symbols,
                content_rowid=id
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
                content, content=docs, content_rowid=id
            );",
        )
    }

    /// Detect old FTS schema missing `doc_comment` column and
    /// rebuild if needed. Safe to call on fresh databases too.
    pub fn migrate_fts_schema(&self) -> SqlResult<()> {
        let sql: String = self.conn.query_row(
            "SELECT sql FROM sqlite_master \
             WHERE type='table' AND name='symbols_fts'",
            [],
            |row| row.get(0),
        )?;
        if !sql.contains("doc_comment") {
            self.conn.execute_batch(
                "DROP TABLE IF EXISTS symbols_fts;
                 CREATE VIRTUAL TABLE symbols_fts USING fts5(
                     name, signature, doc_comment,
                     content=symbols, content_rowid=id
                 );
                 INSERT INTO symbols_fts(rowid, name, signature, doc_comment)
                     SELECT id, name, signature, doc_comment FROM symbols;",
            )?;
        }
        Ok(())
    }

    pub fn clear_index(&self) -> SqlResult<()> {
        self.conn.execute_batch(
            "DELETE FROM docs_fts;
             DELETE FROM symbols_fts;
             DELETE FROM docs;
             DELETE FROM symbol_refs;
             DELETE FROM trait_impls;
             DELETE FROM symbols;
             DELETE FROM files;
             DELETE FROM crate_deps;
             DELETE FROM crates;
             DELETE FROM dependencies;
             DELETE FROM metadata;",
        )
    }

    pub fn set_metadata(&self, repo_path: &str, commit_hash: &str) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (repo_path, commit_hash)
             VALUES (?1, ?2)",
            params![repo_path, commit_hash],
        )?;
        Ok(())
    }

    pub fn get_commit_hash(&self, repo_path: &str) -> SqlResult<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT commit_hash FROM metadata WHERE repo_path = ?1")?;
        let mut rows = stmt.query_map(params![repo_path], |row| row.get(0))?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn get_direct_dependencies(&self) -> SqlResult<Vec<StoredDep>> {
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

    /// Get all indexed file paths.
    pub fn get_all_file_paths(&self) -> SqlResult<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files")?;
        let mut paths = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            paths.push(row.get(0)?);
        }
        Ok(paths)
    }

    /// Get all distinct symbol names (for ref extraction matching).
    pub fn get_all_symbol_names(&self) -> SqlResult<std::collections::HashSet<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT name FROM symbols WHERE kind != 'use' AND kind != 'mod'")?;
        let mut names = std::collections::HashSet::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            names.insert(name);
        }
        Ok(names)
    }

    /// Look up a symbol's DB id by name and file path.
    pub fn get_symbol_id(&self, name: &str, file_path: &str) -> SqlResult<Option<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.name = ?1 AND f.path = ?2 \
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![name, file_path])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Look up any symbol's DB id by name (first match).
    pub fn get_symbol_id_by_name(&self, name: &str) -> SqlResult<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM symbols WHERE name = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn insert_symbol_ref(&self, source_id: i64, target_id: i64, kind: &str) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO symbol_refs \
             (source_symbol_id, target_symbol_id, kind) \
             VALUES (?1, ?2, ?3)",
            params![source_id, target_id, kind],
        )?;
        Ok(())
    }

    pub fn insert_crate(&self, name: &str, path: &str, is_workspace_root: bool) -> SqlResult<i64> {
        self.conn.execute(
            "INSERT OR REPLACE INTO crates (name, path, is_workspace_root) \
             VALUES (?1, ?2, ?3)",
            params![name, path, is_workspace_root],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_crate_by_name(&self, name: &str) -> SqlResult<Option<StoredCrate>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, path, is_workspace_root FROM crates WHERE name = ?1")?;
        let mut rows = stmt.query(params![name])?;
        match rows.next()? {
            Some(row) => Ok(Some(StoredCrate {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                is_workspace_root: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    pub fn get_crate_count(&self) -> SqlResult<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM crates", [], |row| row.get(0))
    }

    pub fn insert_crate_dep(&self, source_crate_id: i64, target_crate_id: i64) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO crate_deps (source_crate_id, target_crate_id) \
             VALUES (?1, ?2)",
            params![source_crate_id, target_crate_id],
        )?;
        Ok(())
    }

    pub fn get_crate_dependents(&self, crate_id: i64) -> SqlResult<Vec<StoredCrate>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name, c.path, c.is_workspace_root \
             FROM crate_deps cd \
             JOIN crates c ON c.id = cd.source_crate_id \
             WHERE cd.target_crate_id = ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![crate_id])?;
        while let Some(row) = rows.next()? {
            results.push(StoredCrate {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                is_workspace_root: row.get(3)?,
            });
        }
        Ok(results)
    }

    pub fn get_transitive_crate_dependents(&self, crate_id: i64) -> SqlResult<Vec<StoredCrate>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE deps(id, name, path, is_workspace_root, depth) AS (
                SELECT id, name, path, is_workspace_root, 0
                FROM crates WHERE id = ?1
              UNION
                SELECT c.id, c.name, c.path, c.is_workspace_root, deps.depth + 1
                FROM deps
                JOIN crate_deps cd ON cd.target_crate_id = deps.id
                JOIN crates c ON c.id = cd.source_crate_id
                WHERE deps.depth < 10
            )
            SELECT DISTINCT id, name, path, is_workspace_root
            FROM deps WHERE id != ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![crate_id])?;
        while let Some(row) = rows.next()? {
            results.push(StoredCrate {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                is_workspace_root: row.get(3)?,
            });
        }
        Ok(results)
    }

    pub fn insert_file_with_crate(
        &self,
        path: &str,
        content_hash: &str,
        crate_id: i64,
    ) -> SqlResult<i64> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, content_hash, crate_id) \
             VALUES (?1, ?2, ?3)",
            params![path, content_hash, crate_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_crate_for_file(&self, file_path: &str) -> SqlResult<Option<StoredCrate>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name, c.path, c.is_workspace_root \
             FROM files f \
             JOIN crates c ON c.id = f.crate_id \
             WHERE f.path = ?1",
        )?;
        let mut rows = stmt.query(params![file_path])?;
        match rows.next()? {
            Some(row) => Ok(Some(StoredCrate {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                is_workspace_root: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    pub fn get_dependency_id(&self, name: &str) -> SqlResult<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM dependencies WHERE name = ?1")?;
        let mut rows = stmt.query(params![name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn get_file_hash(&self, path: &str) -> SqlResult<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT content_hash FROM files WHERE path = ?1")?;
        let mut rows = stmt.query(params![path])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn delete_file_data(&self, path: &str) -> SqlResult<()> {
        self.conn.execute(
            "DELETE FROM trait_impls WHERE file_id = \
             (SELECT id FROM files WHERE path = ?1)",
            params![path],
        )?;
        self.conn.execute(
            "DELETE FROM symbol_refs WHERE source_symbol_id IN \
             (SELECT id FROM symbols WHERE file_id = \
              (SELECT id FROM files WHERE path = ?1)) \
             OR target_symbol_id IN \
             (SELECT id FROM symbols WHERE file_id = \
              (SELECT id FROM files WHERE path = ?1))",
            params![path],
        )?;
        self.conn.execute(
            "DELETE FROM symbols_fts WHERE rowid IN \
             (SELECT id FROM symbols WHERE file_id = \
              (SELECT id FROM files WHERE path = ?1))",
            params![path],
        )?;
        self.conn.execute(
            "DELETE FROM symbols WHERE file_id = \
             (SELECT id FROM files WHERE path = ?1)",
            params![path],
        )?;
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn get_all_files_with_hashes(&self) -> SqlResult<Vec<(String, String, Option<i64>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, content_hash, crate_id FROM files")?;
        let mut results = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(results)
    }

    pub fn insert_file(&self, path: &str, content_hash: &str) -> SqlResult<i64> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, content_hash) \
             VALUES (?1, ?2)",
            params![path, content_hash],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn search_symbols(&self, query: &str) -> SqlResult<Vec<StoredSymbol>> {
        let fts_query = format!("{query}*");
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details \
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
                doc_comment: row.get(7)?,
                body: row.get(8)?,
                details: row.get(9)?,
            });
        }
        Ok(results)
    }

    pub fn get_dependency_by_name(&self, name: &str) -> SqlResult<Option<StoredDep>> {
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

    pub fn store_doc(&self, dep_id: i64, source: &str, content: &str) -> SqlResult<()> {
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

    pub fn search_docs(&self, query: &str) -> SqlResult<Vec<DocResult>> {
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

    pub fn insert_trait_impl(
        &self,
        type_name: &str,
        trait_name: &str,
        file_id: i64,
        line_start: i64,
        line_end: i64,
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO trait_impls \
             (type_name, trait_name, file_id, line_start, line_end) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![type_name, trait_name, file_id, line_start, line_end],
        )?;
        Ok(())
    }

    pub fn get_trait_impls_for_type(&self, type_name: &str) -> SqlResult<Vec<StoredTraitImpl>> {
        let mut stmt = self.conn.prepare(
            "SELECT ti.type_name, ti.trait_name, f.path, \
                    ti.line_start, ti.line_end \
             FROM trait_impls ti \
             JOIN files f ON f.id = ti.file_id \
             WHERE ti.type_name = ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![type_name])?;
        while let Some(row) = rows.next()? {
            results.push(StoredTraitImpl {
                type_name: row.get(0)?,
                trait_name: row.get(1)?,
                file_path: row.get(2)?,
                line_start: row.get(3)?,
                line_end: row.get(4)?,
            });
        }
        Ok(results)
    }

    pub fn get_trait_impls_for_trait(&self, trait_name: &str) -> SqlResult<Vec<StoredTraitImpl>> {
        let mut stmt = self.conn.prepare(
            "SELECT ti.type_name, ti.trait_name, f.path, \
                    ti.line_start, ti.line_end \
             FROM trait_impls ti \
             JOIN files f ON f.id = ti.file_id \
             WHERE ti.trait_name = ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![trait_name])?;
        while let Some(row) = rows.next()? {
            results.push(StoredTraitImpl {
                type_name: row.get(0)?,
                trait_name: row.get(1)?,
                file_path: row.get(2)?,
                line_start: row.get(3)?,
                line_end: row.get(4)?,
            });
        }
        Ok(results)
    }

    pub fn get_symbols_by_path_prefix(&self, path_prefix: &str) -> SqlResult<Vec<StoredSymbol>> {
        let pattern = format!("{path_prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE f.path LIKE ?1 \
               AND s.visibility IN ('public', 'pub(crate)') \
             ORDER BY f.path, s.line_start",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push(StoredSymbol {
                name: row.get(0)?,
                kind: row.get(1)?,
                visibility: row.get(2)?,
                file_path: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
                signature: row.get(6)?,
                doc_comment: row.get(7)?,
                body: row.get(8)?,
                details: row.get(9)?,
            });
        }
        Ok(results)
    }

    pub fn get_callees(&self, symbol_name: &str) -> SqlResult<Vec<CalleeInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts.name, ts.kind, f.path, sr.kind \
             FROM symbol_refs sr \
             JOIN symbols ss ON ss.id = sr.source_symbol_id \
             JOIN symbols ts ON ts.id = sr.target_symbol_id \
             JOIN files f ON f.id = ts.file_id \
             WHERE ss.name = ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![symbol_name])?;
        while let Some(row) = rows.next()? {
            results.push(CalleeInfo {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                ref_kind: row.get(3)?,
            });
        }
        Ok(results)
    }

    pub fn get_docs_for_dependency(&self, name: &str) -> SqlResult<Vec<DocResult>> {
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
pub struct StoredCrate {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub is_workspace_root: bool,
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
    pub doc_comment: Option<String>,
    pub body: Option<String>,
    pub details: Option<String>,
}

#[derive(Debug)]
pub struct StoredTraitImpl {
    pub type_name: String,
    pub trait_name: String,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
}

#[derive(Debug)]
pub struct CalleeInfo {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub ref_kind: String,
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
        assert!(tables.contains(&"crates".to_string()));
        assert!(tables.contains(&"crate_deps".to_string()));
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
    fn test_insert_and_get_crate() {
        let db = Database::open_in_memory().unwrap();
        let id = db
            .insert_crate("hcfs-server", "hcfs-server", false)
            .unwrap();
        assert!(id > 0);
        let c = db.get_crate_by_name("hcfs-server").unwrap().unwrap();
        assert_eq!(c.name, "hcfs-server");
        assert_eq!(c.path, "hcfs-server");
    }

    #[test]
    fn test_insert_crate_dep() {
        let db = Database::open_in_memory().unwrap();
        let shared = db.insert_crate("shared", "shared", false).unwrap();
        let server = db.insert_crate("server", "server", false).unwrap();
        db.insert_crate_dep(server, shared).unwrap();
        let deps = db.get_crate_dependents(shared).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "server");
    }

    #[test]
    fn test_transitive_crate_dependents() {
        let db = Database::open_in_memory().unwrap();
        let shared = db.insert_crate("shared", "shared", false).unwrap();
        let client = db.insert_crate("client", "client", false).unwrap();
        let cli = db.insert_crate("cli", "cli", false).unwrap();
        db.insert_crate_dep(client, shared).unwrap();
        db.insert_crate_dep(cli, client).unwrap();
        let deps = db.get_transitive_crate_dependents(shared).unwrap();
        assert_eq!(deps.len(), 2);
        let names: Vec<&str> = deps.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"client"));
        assert!(names.contains(&"cli"));
    }

    #[test]
    fn test_insert_file_with_crate() {
        let db = Database::open_in_memory().unwrap();
        let crate_id = db.insert_crate("mylib", "mylib", false).unwrap();
        let file_id = db
            .insert_file_with_crate("mylib/src/lib.rs", "hash", crate_id)
            .unwrap();
        assert!(file_id > 0);
        let c = db.get_crate_for_file("mylib/src/lib.rs").unwrap().unwrap();
        assert_eq!(c.name, "mylib");
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

    #[test]
    fn test_schema_has_new_columns() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, \
                  doc_comment, body, details) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    file_id,
                    "MyStruct",
                    "struct",
                    "public",
                    1,
                    10,
                    "pub struct MyStruct",
                    "A doc comment",
                    "{ field: u32 }",
                    "field: u32"
                ],
            )
            .unwrap();
        let row: (Option<String>, Option<String>, Option<String>) = db
            .conn
            .query_row(
                "SELECT doc_comment, body, details FROM symbols \
                 WHERE name = 'MyStruct'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, Some("A doc comment".to_string()));
        assert_eq!(row.1, Some("{ field: u32 }".to_string()));
        assert_eq!(row.2, Some("field: u32".to_string()));
    }

    #[test]
    fn test_trait_impls_table_exists() {
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
        assert!(tables.contains(&"trait_impls".to_string()));
    }

    #[test]
    fn test_trait_impls_unique_constraint() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.insert_trait_impl("MyStruct", "Display", file_id, 1, 5)
            .unwrap();
        // Second insert with same key should be ignored (INSERT OR IGNORE)
        db.insert_trait_impl("MyStruct", "Display", file_id, 1, 5)
            .unwrap();
        let impls = db.get_trait_impls_for_type("MyStruct").unwrap();
        assert_eq!(impls.len(), 1);
    }

    #[test]
    fn test_fts_migration_from_old_schema() {
        let conn = Connection::open_in_memory().unwrap();
        // Create old schema without doc_comment in FTS
        conn.execute_batch(
            "CREATE TABLE symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                visibility TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                signature TEXT NOT NULL,
                doc_comment TEXT,
                body TEXT,
                details TEXT
            );
            CREATE TABLE files (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                content_hash TEXT NOT NULL,
                crate_id INTEGER
            );
            CREATE VIRTUAL TABLE symbols_fts USING fts5(
                name, signature, content=symbols, content_rowid=id
            );",
        )
        .unwrap();
        // Insert a symbol
        conn.execute(
            "INSERT INTO files (path, content_hash) VALUES ('a.rs', 'h')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO symbols \
             (file_id, name, kind, visibility, \
              line_start, line_end, signature, doc_comment) \
             VALUES (1, 'foo', 'fn', 'public', 1, 5, \
                     'pub fn foo()', 'Does foo things')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO symbols_fts (rowid, name, signature) \
             VALUES (1, 'foo', 'pub fn foo()')",
            [],
        )
        .unwrap();
        // Wrap in Database and run migration
        let db = Database { conn };
        db.migrate_fts_schema().unwrap();
        // Verify new FTS schema has doc_comment
        let sql: String = db
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master \
                 WHERE type='table' AND name='symbols_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sql.contains("doc_comment"));
    }

    #[test]
    fn test_fts_search_by_doc_comment() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, doc_comment) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    file_id,
                    "process_data",
                    "function",
                    "public",
                    1,
                    10,
                    "pub fn process_data()",
                    "Transforms raw input into structured output"
                ],
            )
            .unwrap();
        let rowid = db.conn.last_insert_rowid();
        db.conn
            .execute(
                "INSERT INTO symbols_fts \
                 (rowid, name, signature, doc_comment) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    rowid,
                    "process_data",
                    "pub fn process_data()",
                    "Transforms raw input into structured output"
                ],
            )
            .unwrap();
        // Search by doc comment content
        let results = db.search_symbols("Transforms").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "process_data");
        assert_eq!(
            results[0].doc_comment,
            Some("Transforms raw input into structured output".to_string())
        );
    }

    #[test]
    fn test_get_trait_impls_for_type() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.insert_trait_impl("MyStruct", "Display", file_id, 10, 20)
            .unwrap();
        db.insert_trait_impl("MyStruct", "Debug", file_id, 22, 30)
            .unwrap();
        db.insert_trait_impl("Other", "Display", file_id, 32, 40)
            .unwrap();
        let impls = db.get_trait_impls_for_type("MyStruct").unwrap();
        assert_eq!(impls.len(), 2);
        let traits: Vec<&str> = impls.iter().map(|i| i.trait_name.as_str()).collect();
        assert!(traits.contains(&"Display"));
        assert!(traits.contains(&"Debug"));
    }

    #[test]
    fn test_get_trait_impls_for_trait() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.insert_trait_impl("MyStruct", "Display", file_id, 10, 20)
            .unwrap();
        db.insert_trait_impl("Other", "Display", file_id, 22, 30)
            .unwrap();
        db.insert_trait_impl("MyStruct", "Debug", file_id, 32, 40)
            .unwrap();
        let impls = db.get_trait_impls_for_trait("Display").unwrap();
        assert_eq!(impls.len(), 2);
        let types: Vec<&str> = impls.iter().map(|i| i.type_name.as_str()).collect();
        assert!(types.contains(&"MyStruct"));
        assert!(types.contains(&"Other"));
    }

    #[test]
    fn test_get_symbols_by_path_prefix() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/server/mod.rs", "h1").unwrap();
        let f2 = db.insert_file("src/server/tools.rs", "h2").unwrap();
        let f3 = db.insert_file("src/db.rs", "h3").unwrap();
        // Insert symbols with different visibilities
        for (fid, name, vis) in [
            (f1, "serve", "public"),
            (f2, "handle", "pub(crate)"),
            (f2, "helper", "private"),
            (f3, "query", "public"),
        ] {
            db.conn
                .execute(
                    "INSERT INTO symbols \
                     (file_id, name, kind, visibility, \
                      line_start, line_end, signature) \
                     VALUES (?1, ?2, 'function', ?3, 1, 10, ?4)",
                    params![fid, name, vis, format!("fn {name}()")],
                )
                .unwrap();
        }
        let results = db.get_symbols_by_path_prefix("src/server/").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "serve");
        assert_eq!(results[1].name, "handle");
        // private helper should be excluded
        assert!(!results.iter().any(|s| s.name == "helper"));
        // db.rs should be excluded (different prefix)
        assert!(!results.iter().any(|s| s.name == "query"));
    }

    #[test]
    fn test_get_callees() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        // Insert source and target symbols
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'caller', 'function', 'public', \
                         1, 10, 'fn caller()')",
                params![file_id],
            )
            .unwrap();
        let caller_id = db.conn.last_insert_rowid();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'callee_a', 'function', 'public', \
                         12, 20, 'fn callee_a()')",
                params![file_id],
            )
            .unwrap();
        let callee_a_id = db.conn.last_insert_rowid();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'callee_b', 'struct', 'public', \
                         22, 30, 'struct callee_b')",
                params![file_id],
            )
            .unwrap();
        let callee_b_id = db.conn.last_insert_rowid();
        // Insert refs
        db.insert_symbol_ref(caller_id, callee_a_id, "call")
            .unwrap();
        db.insert_symbol_ref(caller_id, callee_b_id, "type_ref")
            .unwrap();
        let callees = db.get_callees("caller").unwrap();
        assert_eq!(callees.len(), 2);
        let names: Vec<&str> = callees.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"callee_a"));
        assert!(names.contains(&"callee_b"));
        // Verify ref kinds
        let call_ref = callees.iter().find(|c| c.name == "callee_a").unwrap();
        assert_eq!(call_ref.ref_kind, "call");
        let type_ref = callees.iter().find(|c| c.name == "callee_b").unwrap();
        assert_eq!(type_ref.ref_kind, "type_ref");
    }
}
