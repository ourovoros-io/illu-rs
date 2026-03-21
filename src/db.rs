use crate::indexer::parser::{SymbolKind, Visibility};
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, Result as SqlResult, params};

macro_rules! newtype_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub(crate) i64);

        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                i64::column_result(value).map(Self)
            }
        }

        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                self.0.to_sql()
            }
        }
    };
}

newtype_id!(FileId);
newtype_id!(SymbolId);
newtype_id!(CrateId);
newtype_id!(DepId);

pub type SymbolRefCount = (String, String, i64, Option<String>);

fn escape_like(s: &str) -> String {
    s.replace('%', r"\%").replace('_', r"\_")
}

fn is_fts_safe(query: &str) -> bool {
    query
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == ' ')
}

fn parse_kind(s: &str) -> rusqlite::Result<SymbolKind> {
    s.parse().map_err(|e: String| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
    })
}

fn parse_visibility(s: &str) -> rusqlite::Result<Visibility> {
    s.parse().map_err(|e: String| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, e.into())
    })
}

fn row_to_stored_symbol(row: &rusqlite::Row) -> SqlResult<StoredSymbol> {
    Ok(StoredSymbol {
        name: row.get(0)?,
        kind: parse_kind(&row.get::<_, String>(1)?)?,
        visibility: parse_visibility(&row.get::<_, String>(2)?)?,
        file_path: row.get(3)?,
        line_start: row.get(4)?,
        line_end: row.get(5)?,
        signature: row.get(6)?,
        doc_comment: row.get(7)?,
        body: row.get(8)?,
        details: row.get(9)?,
        attributes: row.get(10)?,
        impl_type: row.get(11)?,
    })
}

fn row_to_stored_crate(row: &rusqlite::Row) -> SqlResult<StoredCrate> {
    Ok(StoredCrate {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
    })
}

fn row_to_stored_trait_impl(row: &rusqlite::Row) -> SqlResult<StoredTraitImpl> {
    Ok(StoredTraitImpl {
        type_name: row.get(0)?,
        trait_name: row.get(1)?,
        file_path: row.get(2)?,
        line_start: row.get(3)?,
        line_end: row.get(4)?,
    })
}

fn row_to_doc_result(row: &rusqlite::Row) -> SqlResult<DocResult> {
    Ok(DocResult {
        content: row.get(0)?,
        source: row.get(1)?,
        dependency_name: row.get(2)?,
        version: row.get(3)?,
        module: row.get(4)?,
    })
}

pub struct Database {
    pub(crate) conn: Connection,
    repo_root: Option<std::path::PathBuf>,
}

impl Database {
    pub fn open_in_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn,
            repo_root: None,
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn open(path: &std::path::Path) -> SqlResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -8000;
             PRAGMA mmap_size = 67108864;
             PRAGMA temp_store = MEMORY;
             PRAGMA foreign_keys = ON;",
        )?;
        // DB path is {repo}/.illu/index.db — derive repo root
        let repo_root = path
            .parent()
            .filter(|p| p.file_name().is_some_and(|n| n == ".illu"))
            .and_then(|p| p.parent())
            .map(std::path::Path::to_path_buf);
        let db = Self { conn, repo_root };
        db.migrate()?;
        Ok(db)
    }

    pub fn repo_root(&self) -> Option<&std::path::Path> {
        self.repo_root.as_deref()
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
                details TEXT,
                attributes TEXT,
                impl_type TEXT,
                is_test INTEGER NOT NULL DEFAULT 0
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
                kind TEXT NOT NULL,
                confidence TEXT NOT NULL DEFAULT 'high'
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
                content TEXT NOT NULL,
                module TEXT NOT NULL DEFAULT ''
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

            CREATE VIRTUAL TABLE IF NOT EXISTS symbols_trigram USING fts5(
                name, tokenize='trigram', content=symbols,
                content_rowid=id
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
                content, content=docs, content_rowid=id
            );

            CREATE TABLE IF NOT EXISTS schema_info (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        self.create_indexes()?;
        self.migrate_fts_schema()?;
        self.migrate_docs_module_column()?;
        self.migrate_symbols_impl_type_column()?;
        self.migrate_symbol_refs_confidence_column()?;
        self.migrate_symbols_is_test_column()?;
        self.check_schema_version()
    }

    /// Bump to force full re-index after parser/schema changes.
    const SCHEMA_VERSION: &str = "4";

    fn check_schema_version(&self) -> SqlResult<()> {
        let current: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM schema_info WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .ok();

        if current.as_deref() != Some(Self::SCHEMA_VERSION) {
            tracing::info!(
                old = ?current,
                new = Self::SCHEMA_VERSION,
                "Schema version changed, clearing code index for full re-index"
            );
            self.clear_code_index()?;
            self.conn.execute(
                "INSERT OR REPLACE INTO schema_info (key, value) \
                 VALUES ('schema_version', ?1)",
                params![Self::SCHEMA_VERSION],
            )?;
        }
        Ok(())
    }

    fn create_indexes(&self) -> SqlResult<()> {
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_symbols_name
                ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_file_id
                ON symbols(file_id);
            CREATE INDEX IF NOT EXISTS idx_symbol_refs_target
                ON symbol_refs(target_symbol_id);
            CREATE INDEX IF NOT EXISTS idx_symbol_refs_source
                ON symbol_refs(source_symbol_id);
            CREATE INDEX IF NOT EXISTS idx_trait_impls_type
                ON trait_impls(type_name);
            CREATE INDEX IF NOT EXISTS idx_trait_impls_trait
                ON trait_impls(trait_name);
            CREATE INDEX IF NOT EXISTS idx_trait_impls_file_id
                ON trait_impls(file_id);
            CREATE INDEX IF NOT EXISTS idx_deps_name
                ON dependencies(name);
            CREATE INDEX IF NOT EXISTS idx_docs_dep_id
                ON docs(dependency_id);
            CREATE INDEX IF NOT EXISTS idx_symbol_refs_confidence
                ON symbol_refs(confidence);
            CREATE INDEX IF NOT EXISTS idx_files_crate_id
                ON files(crate_id);
            CREATE INDEX IF NOT EXISTS idx_symbols_name_impl
                ON symbols(name, impl_type);",
        )
    }

    /// Detect old FTS schema and rebuild if needed.
    /// Handles: missing `doc_comment` column, missing trigram table.
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

        let has_trigram: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master \
             WHERE type='table' AND name='symbols_trigram'",
            [],
            |row| row.get(0),
        )?;
        if !has_trigram {
            self.conn.execute_batch(
                "CREATE VIRTUAL TABLE symbols_trigram USING fts5(
                     name, tokenize='trigram',
                     content=symbols, content_rowid=id
                 );
                 INSERT INTO symbols_trigram(rowid, name)
                     SELECT id, name FROM symbols;",
            )?;
        }
        Ok(())
    }

    /// Add `module` column to docs table if missing (existing DBs).
    fn migrate_docs_module_column(&self) -> SqlResult<()> {
        let sql: String = self.conn.query_row(
            "SELECT sql FROM sqlite_master \
             WHERE type='table' AND name='docs'",
            [],
            |row| row.get(0),
        )?;
        if !sql.contains("module") {
            self.conn
                .execute_batch("ALTER TABLE docs ADD COLUMN module TEXT NOT NULL DEFAULT ''")?;
        }
        Ok(())
    }

    /// Add `impl_type` column to symbols table if missing (existing DBs).
    fn migrate_symbols_impl_type_column(&self) -> SqlResult<()> {
        let sql: String = self.conn.query_row(
            "SELECT sql FROM sqlite_master \
             WHERE type='table' AND name='symbols'",
            [],
            |row| row.get(0),
        )?;
        if !sql.contains("impl_type") {
            self.conn
                .execute_batch("ALTER TABLE symbols ADD COLUMN impl_type TEXT")?;
        }
        Ok(())
    }

    /// Add `confidence` column to `symbol_refs` table if missing (existing DBs).
    fn migrate_symbol_refs_confidence_column(&self) -> SqlResult<()> {
        let has_confidence = self
            .conn
            .prepare("SELECT confidence FROM symbol_refs LIMIT 0")
            .is_ok();
        if !has_confidence {
            self.conn.execute_batch(
                "ALTER TABLE symbol_refs ADD COLUMN confidence TEXT NOT NULL DEFAULT 'high'",
            )?;
        }
        Ok(())
    }

    /// Add `is_test` column to `symbols` table if missing, then backfill from attributes.
    fn migrate_symbols_is_test_column(&self) -> SqlResult<()> {
        let has_is_test = self
            .conn
            .prepare("SELECT is_test FROM symbols LIMIT 0")
            .is_ok();
        if !has_is_test {
            self.conn.execute_batch(
                "ALTER TABLE symbols ADD COLUMN is_test INTEGER NOT NULL DEFAULT 0",
            )?;
            self.conn
                .execute_batch("UPDATE symbols SET is_test = 1 WHERE attributes LIKE '%test%'")?;
        }
        Ok(())
    }

    /// Clear code index data while preserving cached docs and dependencies.
    /// Used during re-indexing to avoid re-fetching documentation.
    pub fn clear_code_index(&self) -> SqlResult<()> {
        self.conn.execute_batch(
            "INSERT INTO symbols_fts(symbols_fts) VALUES('delete-all');
             INSERT INTO symbols_trigram(symbols_trigram) VALUES('delete-all');
             DELETE FROM symbol_refs;
             DELETE FROM trait_impls;
             DELETE FROM symbols;
             DELETE FROM files;
             DELETE FROM crate_deps;
             DELETE FROM crates;
             DELETE FROM metadata;",
        )
    }

    /// Completely reset the index, including all cached documentation.
    pub fn clear_all(&self) -> SqlResult<()> {
        self.clear_code_index()?;
        self.conn.execute_batch(
            "INSERT INTO docs_fts(docs_fts) VALUES('delete-all');
             DELETE FROM docs;
             DELETE FROM dependencies;",
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

    pub fn get_file_symbol_counts(&self, prefix: &str) -> SqlResult<Vec<FileSymbolCount>> {
        let pattern = format!("{}%", escape_like(prefix));
        let mut stmt = self.conn.prepare(
            "SELECT f.path, COUNT(s.id) \
             FROM files f \
             LEFT JOIN symbols s ON s.file_id = f.id AND s.visibility = 'public' \
             WHERE f.path LIKE ?1 ESCAPE '\\' \
             GROUP BY f.path \
             ORDER BY f.path",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push(FileSymbolCount {
                path: row.get(0)?,
                count: row.get(1)?,
            });
        }
        Ok(results)
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
    pub fn get_symbol_id(&self, name: &str, file_path: &str) -> SqlResult<Option<SymbolId>> {
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

    /// Look up a symbol's DB id by name and impl type.
    pub fn get_symbol_id_in_impl(
        &self,
        name: &str,
        impl_type: &str,
    ) -> SqlResult<Option<SymbolId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM symbols WHERE name = ?1 AND impl_type = ?2 LIMIT 1")?;
        let mut rows = stmt.query(params![name, impl_type])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Look up any symbol's DB id by name (first match).
    pub fn get_symbol_id_by_name(&self, name: &str) -> SqlResult<Option<SymbolId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM symbols WHERE name = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn insert_symbol_ref(
        &self,
        source_id: SymbolId,
        target_id: SymbolId,
        kind: &str,
        confidence: &str,
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO symbol_refs \
             (source_symbol_id, target_symbol_id, kind, confidence) \
             VALUES (?1, ?2, ?3, ?4)",
            params![source_id, target_id, kind, confidence],
        )?;
        Ok(())
    }

    pub fn insert_crate(&self, name: &str, path: &str) -> SqlResult<CrateId> {
        self.conn.execute(
            "INSERT OR REPLACE INTO crates (name, path, is_workspace_root) \
             VALUES (?1, ?2, 0)",
            params![name, path],
        )?;
        Ok(CrateId(self.conn.last_insert_rowid()))
    }

    pub fn get_crate_by_name(&self, name: &str) -> SqlResult<Option<StoredCrate>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, path FROM crates WHERE name = ?1")?;
        let mut rows = stmt.query(params![name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_stored_crate(row)?)),
            None => Ok(None),
        }
    }

    pub fn get_all_crates(&self) -> SqlResult<Vec<StoredCrate>> {
        let mut stmt = self.conn.prepare("SELECT id, name, path FROM crates")?;
        let mut results = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_crate(row)?);
        }
        Ok(results)
    }

    pub fn get_crate_count(&self) -> SqlResult<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM crates", [], |row| row.get(0))
    }

    pub fn get_all_crate_deps(&self) -> SqlResult<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT sc.name, tc.name \
             FROM crate_deps cd \
             JOIN crates sc ON sc.id = cd.source_crate_id \
             JOIN crates tc ON tc.id = cd.target_crate_id \
             ORDER BY sc.name, tc.name",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }
        Ok(results)
    }

    pub fn insert_crate_dep(
        &self,
        source_crate_id: CrateId,
        target_crate_id: CrateId,
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO crate_deps (source_crate_id, target_crate_id) \
             VALUES (?1, ?2)",
            params![source_crate_id, target_crate_id],
        )?;
        Ok(())
    }

    pub fn get_crate_dependents(&self, crate_id: CrateId) -> SqlResult<Vec<StoredCrate>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name, c.path \
             FROM crate_deps cd \
             JOIN crates c ON c.id = cd.source_crate_id \
             WHERE cd.target_crate_id = ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![crate_id])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_crate(row)?);
        }
        Ok(results)
    }

    pub fn get_transitive_crate_dependents(
        &self,
        crate_id: CrateId,
    ) -> SqlResult<Vec<StoredCrate>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE deps(id, name, path, depth) AS (
                SELECT id, name, path, 0
                FROM crates WHERE id = ?1
              UNION
                SELECT c.id, c.name, c.path, deps.depth + 1
                FROM deps
                JOIN crate_deps cd ON cd.target_crate_id = deps.id
                JOIN crates c ON c.id = cd.source_crate_id
                WHERE deps.depth < 10
            )
            SELECT DISTINCT id, name, path
            FROM deps WHERE id != ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![crate_id])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_crate(row)?);
        }
        Ok(results)
    }

    pub fn insert_file_with_crate(
        &self,
        path: &str,
        content_hash: &str,
        crate_id: CrateId,
    ) -> SqlResult<FileId> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, content_hash, crate_id) \
             VALUES (?1, ?2, ?3)",
            params![path, content_hash, crate_id],
        )?;
        Ok(FileId(self.conn.last_insert_rowid()))
    }

    pub fn get_crate_for_file(&self, file_path: &str) -> SqlResult<Option<StoredCrate>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name, c.path \
             FROM files f \
             JOIN crates c ON c.id = f.crate_id \
             WHERE f.path = ?1",
        )?;
        let mut rows = stmt.query(params![file_path])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_stored_crate(row)?)),
            None => Ok(None),
        }
    }

    pub fn get_dependency_id(&self, name: &str) -> SqlResult<Option<DepId>> {
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
        let file_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .ok();
        let Some(fid) = file_id else {
            return Ok(());
        };
        self.conn
            .execute("DELETE FROM trait_impls WHERE file_id = ?1", params![fid])?;
        self.conn.execute(
            "DELETE FROM symbol_refs WHERE source_symbol_id IN \
             (SELECT id FROM symbols WHERE file_id = ?1) \
             OR target_symbol_id IN \
             (SELECT id FROM symbols WHERE file_id = ?1)",
            params![fid],
        )?;
        self.conn.execute(
            "DELETE FROM symbols_fts WHERE rowid IN \
             (SELECT id FROM symbols WHERE file_id = ?1)",
            params![fid],
        )?;
        self.conn.execute(
            "DELETE FROM symbols_trigram WHERE rowid IN \
             (SELECT id FROM symbols WHERE file_id = ?1)",
            params![fid],
        )?;
        self.conn
            .execute("DELETE FROM symbols WHERE file_id = ?1", params![fid])?;
        self.conn
            .execute("DELETE FROM files WHERE id = ?1", params![fid])?;
        Ok(())
    }

    /// Delete `symbol_refs` where source or target symbol no longer exists.
    pub fn delete_stale_refs(&self) -> SqlResult<u64> {
        let deleted = self.conn.execute(
            "DELETE FROM symbol_refs \
             WHERE source_symbol_id NOT IN (SELECT id FROM symbols) \
                OR target_symbol_id NOT IN (SELECT id FROM symbols)",
            [],
        )?;
        Ok(u64::try_from(deleted).unwrap_or(0))
    }

    pub fn file_count(&self) -> SqlResult<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
    }

    /// Count symbol refs grouped by confidence level.
    pub fn count_refs_by_confidence(&self) -> SqlResult<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT confidence, COUNT(*) FROM symbol_refs \
             GROUP BY confidence ORDER BY confidence",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }
        Ok(results)
    }

    /// Count function symbols whose signature appears truncated
    /// (ends with just an open paren).
    pub fn count_truncated_signatures(&self) -> SqlResult<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM symbols \
             WHERE signature LIKE '%(' AND kind = 'function'",
            [],
            |row| row.get(0),
        )
    }

    /// Count all function symbols.
    pub fn count_functions(&self) -> SqlResult<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE kind = 'function'",
            [],
            |row| row.get(0),
        )
    }

    /// Get low-confidence refs with highest fan-in (most likely noise sources).
    pub fn get_noisy_symbols(&self, limit: i64) -> SqlResult<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts.name, COUNT(*) as cnt \
             FROM symbol_refs sr \
             JOIN symbols ts ON ts.id = sr.target_symbol_id \
             WHERE sr.confidence = 'low' \
             GROUP BY ts.name \
             ORDER BY cnt DESC \
             LIMIT ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![limit])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }
        Ok(results)
    }

    pub fn impact_dependents(
        &self,
        symbol_name: &str,
        impl_type: Option<&str>,
    ) -> SqlResult<Vec<ImpactEntry>> {
        self.impact_dependents_with_depth(symbol_name, impl_type, 5)
    }

    pub fn impact_dependents_with_depth(
        &self,
        symbol_name: &str,
        impl_type: Option<&str>,
        max_depth: i64,
    ) -> SqlResult<Vec<ImpactEntry>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE deps(id, name, file_path, depth, via) AS (
                SELECT s.id, s.name, f.path, 0, ''
                FROM symbols s
                JOIN files f ON f.id = s.file_id
                WHERE s.name = ?1 AND (?3 IS NULL OR s.impl_type = ?3)
              UNION
                SELECT s2.id, s2.name, f2.path, deps.depth + 1,
                       CASE WHEN deps.via = '' THEN deps.name
                            ELSE deps.via || ' -> ' || deps.name
                       END
                FROM deps
                JOIN symbol_refs sr ON sr.target_symbol_id = deps.id
                JOIN symbols s2 ON s2.id = sr.source_symbol_id
                JOIN files f2 ON f2.id = s2.file_id
                WHERE deps.depth < ?2
            )
            SELECT DISTINCT name, file_path, depth, via FROM deps
            WHERE depth > 0
            ORDER BY depth, name
            LIMIT 100",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![symbol_name, max_depth, impl_type])?;
        while let Some(row) = rows.next()? {
            results.push(ImpactEntry {
                name: row.get(0)?,
                file_path: row.get(1)?,
                depth: row.get(2)?,
                via: row.get(3)?,
            });
        }
        Ok(results)
    }

    /// Find test functions that directly or transitively call the given symbol.
    /// Walks the call graph upward (who calls me?) filtering for `#[test]` attributes.
    pub fn get_related_tests(
        &self,
        symbol_name: &str,
        impl_type: Option<&str>,
    ) -> SqlResult<Vec<TestEntry>> {
        let mut stmt = self.conn.prepare_cached(
            "WITH RECURSIVE callers(id, name, file_path, line_start, depth, is_test) AS (
                SELECT s.id, s.name, f.path, s.line_start, 0, s.is_test
                FROM symbols s
                JOIN files f ON f.id = s.file_id
                WHERE s.name = ?1 AND (?2 IS NULL OR s.impl_type = ?2)
              UNION
                SELECT s2.id, s2.name, f2.path, s2.line_start, callers.depth + 1, s2.is_test
                FROM callers
                JOIN symbol_refs sr ON sr.target_symbol_id = callers.id
                JOIN symbols s2 ON s2.id = sr.source_symbol_id
                JOIN files f2 ON f2.id = s2.file_id
                WHERE callers.depth < 5
            )
            SELECT DISTINCT name, file_path, line_start
            FROM callers
            WHERE depth > 0
              AND is_test = 1
            ORDER BY file_path, name",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![symbol_name, impl_type])?;
        while let Some(row) = rows.next()? {
            results.push(TestEntry {
                name: row.get(0)?,
                file_path: row.get(1)?,
                line_start: row.get(2)?,
            });
        }
        Ok(results)
    }

    pub fn get_all_files_with_hashes(&self) -> SqlResult<Vec<FileRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, content_hash, crate_id FROM files")?;
        let mut results = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            results.push(FileRecord {
                path: row.get(0)?,
                content_hash: row.get(1)?,
                crate_id: row.get(2)?,
            });
        }
        Ok(results)
    }

    pub fn insert_file(&self, path: &str, content_hash: &str) -> SqlResult<FileId> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, content_hash) \
             VALUES (?1, ?2)",
            params![path, content_hash],
        )?;
        Ok(FileId(self.conn.last_insert_rowid()))
    }

    pub fn search_symbols(&self, query: &str) -> SqlResult<Vec<StoredSymbol>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let fts_safe = is_fts_safe(query);
        let use_trigram = query.len() >= 3;

        if fts_safe {
            let fts_query = format!("\"{query}\"*");
            let (sql, substr_param) = if use_trigram {
                (
                    "WITH fts_results AS ( \
                        SELECT fts.rowid AS sid FROM symbols_fts fts \
                        WHERE symbols_fts MATCH ?1 \
                    ), \
                    fallback AS ( \
                        SELECT tri.rowid AS sid FROM symbols_trigram tri \
                        WHERE symbols_trigram MATCH ?3 \
                          AND tri.rowid NOT IN (SELECT sid FROM fts_results) \
                    ), \
                    combined AS ( \
                        SELECT sid, 0 AS source FROM fts_results \
                        UNION ALL \
                        SELECT sid, 1 AS source FROM fallback \
                    ) \
                    SELECT s.name, s.kind, s.visibility, f.path, \
                           s.line_start, s.line_end, s.signature, \
                           s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
                    FROM combined c \
                    JOIN symbols s ON s.id = c.sid \
                    JOIN files f ON f.id = s.file_id \
                    ORDER BY CASE WHEN s.name = ?2 THEN 0 ELSE 1 END, \
                             c.source, s.name \
                    LIMIT 50",
                    format!("\"{query}\""),
                )
            } else {
                let escaped = escape_like(query);
                (
                    "WITH fts_results AS ( \
                        SELECT fts.rowid AS sid FROM symbols_fts fts \
                        WHERE symbols_fts MATCH ?1 \
                    ), \
                    fallback AS ( \
                        SELECT s.id AS sid FROM symbols s \
                        WHERE s.name LIKE ?3 ESCAPE '\\' \
                          AND s.id NOT IN (SELECT sid FROM fts_results) \
                    ), \
                    combined AS ( \
                        SELECT sid, 0 AS source FROM fts_results \
                        UNION ALL \
                        SELECT sid, 1 AS source FROM fallback \
                    ) \
                    SELECT s.name, s.kind, s.visibility, f.path, \
                           s.line_start, s.line_end, s.signature, \
                           s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
                    FROM combined c \
                    JOIN symbols s ON s.id = c.sid \
                    JOIN files f ON f.id = s.file_id \
                    ORDER BY CASE WHEN s.name = ?2 THEN 0 ELSE 1 END, \
                             c.source, s.name \
                    LIMIT 50",
                    format!("%{escaped}%"),
                )
            };
            let mut stmt = self.conn.prepare_cached(sql)?;
            let mut results = Vec::new();
            let mut rows = stmt.query(params![fts_query, query, substr_param])?;
            while let Some(row) = rows.next()? {
                results.push(row_to_stored_symbol(row)?);
            }
            Ok(results)
        } else {
            // Unsafe for FTS — use LIKE only (always safe)
            let escaped = escape_like(query);
            let like_pattern = format!("%{escaped}%");
            let mut stmt = self.conn.prepare_cached(
                "SELECT s.name, s.kind, s.visibility, f.path, \
                       s.line_start, s.line_end, s.signature, \
                       s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
                FROM symbols s \
                JOIN files f ON f.id = s.file_id \
                WHERE s.name LIKE ?1 ESCAPE '\\' \
                ORDER BY CASE WHEN s.name = ?2 THEN 0 ELSE 1 END, s.name \
                LIMIT 50",
            )?;
            let mut results = Vec::new();
            let mut rows = stmt.query(params![like_pattern, query])?;
            while let Some(row) = rows.next()? {
                results.push(row_to_stored_symbol(row)?);
            }
            Ok(results)
        }
    }

    /// Search for symbols matching `impl_type::name` pattern.
    pub fn search_symbols_by_impl(
        &self,
        impl_type: &str,
        method_name: &str,
    ) -> SqlResult<Vec<StoredSymbol>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.name = ?1 AND s.impl_type = ?2 \
             ORDER BY f.path, s.line_start",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![method_name, impl_type])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
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
    ) -> SqlResult<DepId> {
        self.conn.execute(
            "INSERT INTO dependencies \
             (name, version, is_direct, repository_url, features) \
             VALUES (?1, ?2, ?3, ?4, '')",
            params![name, version, is_direct, repo_url],
        )?;
        Ok(DepId(self.conn.last_insert_rowid()))
    }

    pub fn store_doc(&self, dep_id: DepId, source: &str, content: &str) -> SqlResult<()> {
        self.store_doc_with_module(dep_id, source, content, "")
    }

    pub fn store_doc_with_module(
        &self,
        dep_id: DepId,
        source: &str,
        content: &str,
        module: &str,
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT INTO docs (dependency_id, source, content, module) \
             VALUES (?1, ?2, ?3, ?4)",
            params![dep_id, source, content, module],
        )?;
        let rowid = self.conn.last_insert_rowid();
        self.conn.execute(
            "INSERT INTO docs_fts (rowid, content) \
             VALUES (?1, ?2)",
            params![rowid, content],
        )?;
        Ok(())
    }

    pub fn get_doc_by_module(&self, dep_name: &str, module: &str) -> SqlResult<Option<DocResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.content, d.source, dep.name, dep.version, d.module \
             FROM docs d \
             JOIN dependencies dep ON dep.id = d.dependency_id \
             WHERE dep.name = ?1 AND d.module = ?2 \
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![dep_name, module])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_doc_result(row)?)),
            None => Ok(None),
        }
    }

    /// List all module names stored for a dependency.
    pub fn get_doc_modules(&self, dep_name: &str) -> SqlResult<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT d.module \
             FROM docs d \
             JOIN dependencies dep ON dep.id = d.dependency_id \
             WHERE dep.name = ?1 AND d.module != '' \
             ORDER BY d.module",
        )?;
        let mut modules = Vec::new();
        let mut rows = stmt.query(params![dep_name])?;
        while let Some(row) = rows.next()? {
            modules.push(row.get(0)?);
        }
        Ok(modules)
    }

    pub fn search_docs(&self, query: &str) -> SqlResult<Vec<DocResult>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        if !is_fts_safe(query) {
            return Ok(Vec::new());
        }
        let fts_query = format!("\"{query}\"*");
        let mut stmt = self.conn.prepare(
            "SELECT d.content, d.source, dep.name, dep.version, d.module \
             FROM docs_fts fts \
             JOIN docs d ON d.id = fts.rowid \
             JOIN dependencies dep ON dep.id = d.dependency_id \
             WHERE docs_fts MATCH ?1 \
             LIMIT 20",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![fts_query])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_doc_result(row)?);
        }
        Ok(results)
    }

    pub fn insert_trait_impl(
        &self,
        type_name: &str,
        trait_name: &str,
        file_id: FileId,
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
            results.push(row_to_stored_trait_impl(row)?);
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
            results.push(row_to_stored_trait_impl(row)?);
        }
        Ok(results)
    }

    /// Find symbols whose line range overlaps any of the given ranges.
    pub fn get_symbols_at_lines(
        &self,
        file_path: &str,
        line_ranges: &[(i64, i64)],
    ) -> SqlResult<Vec<StoredSymbol>> {
        if line_ranges.is_empty() {
            return Ok(Vec::new());
        }
        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE f.path = ?1 AND s.line_start <= ?3 AND s.line_end >= ?2",
        )?;
        for &(range_start, range_end) in line_ranges {
            let mut rows = stmt.query(params![file_path, range_start, range_end])?;
            while let Some(row) = rows.next()? {
                let sym = row_to_stored_symbol(row)?;
                let key = (sym.name.clone(), sym.line_start);
                if seen.insert(key) {
                    results.push(sym);
                }
            }
        }
        Ok(results)
    }

    pub fn get_symbols_by_path_prefix(&self, path_prefix: &str) -> SqlResult<Vec<StoredSymbol>> {
        self.get_symbols_by_path_prefix_filtered(path_prefix, false)
    }

    pub fn get_symbols_by_path_prefix_filtered(
        &self,
        path_prefix: &str,
        include_private: bool,
    ) -> SqlResult<Vec<StoredSymbol>> {
        let pattern = format!("{}%", escape_like(path_prefix));
        let sql = if include_private {
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE f.path LIKE ?1 ESCAPE '\\' \
             ORDER BY f.path, s.line_start"
        } else {
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE f.path LIKE ?1 ESCAPE '\\' \
               AND s.visibility IN ('public', 'pub(crate)') \
             ORDER BY f.path, s.line_start"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
        }
        Ok(results)
    }

    pub fn search_symbols_by_attribute(&self, attr: &str) -> SqlResult<Vec<StoredSymbol>> {
        let pattern = format!("%{}%", escape_like(attr));
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.attributes LIKE ?1 ESCAPE '\\' \
             ORDER BY s.name",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
        }
        Ok(results)
    }

    pub fn search_symbols_by_doc_comment(&self, query: &str) -> SqlResult<Vec<StoredSymbol>> {
        let pattern = format!("%{}%", escape_like(query));
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.doc_comment LIKE ?1 ESCAPE '\\' \
             ORDER BY s.name \
             LIMIT 50",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
        }
        Ok(results)
    }

    pub fn search_symbols_by_body(&self, query: &str) -> SqlResult<Vec<StoredSymbol>> {
        let pattern = format!("%{}%", escape_like(query));
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.body LIKE ?1 ESCAPE '\\' \
             ORDER BY f.path, s.line_start \
             LIMIT 50",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
        }
        Ok(results)
    }

    pub fn search_symbols_by_signature(&self, pattern: &str) -> SqlResult<Vec<StoredSymbol>> {
        let like_pattern = format!("%{}%", escape_like(pattern));
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, \
                    s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.signature LIKE ?1 ESCAPE '\\' \
             ORDER BY s.name \
             LIMIT 50",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![like_pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
        }
        Ok(results)
    }

    pub fn get_callees(&self, symbol_name: &str, source_file: &str) -> SqlResult<Vec<CalleeInfo>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT ts.name, ts.kind, f.path, sr.kind, ts.line_start, ts.impl_type \
             FROM symbol_refs sr \
             JOIN symbols ss ON ss.id = sr.source_symbol_id \
             JOIN symbols ts ON ts.id = sr.target_symbol_id \
             JOIN files f ON f.id = ts.file_id \
             JOIN files sf ON sf.id = ss.file_id \
             WHERE ss.name = ?1 AND sf.path = ?2",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![symbol_name, source_file])?;
        while let Some(row) = rows.next()? {
            results.push(CalleeInfo {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                ref_kind: row.get(3)?,
                line_start: row.get(4)?,
                impl_type: row.get(5)?,
            });
        }
        Ok(results)
    }

    /// Find symbols that directly reference the given symbol (reverse of `get_callees`).
    pub fn get_callers(&self, symbol_name: &str, target_file: &str) -> SqlResult<Vec<CalleeInfo>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT ss.name, ss.kind, sf.path, sr.kind, ss.line_start, ss.impl_type \
             FROM symbol_refs sr \
             JOIN symbols ss ON ss.id = sr.source_symbol_id \
             JOIN symbols ts ON ts.id = sr.target_symbol_id \
             JOIN files sf ON sf.id = ss.file_id \
             JOIN files tf ON tf.id = ts.file_id \
             WHERE ts.name = ?1 AND tf.path = ?2",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![symbol_name, target_file])?;
        while let Some(row) = rows.next()? {
            results.push(CalleeInfo {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                ref_kind: row.get(3)?,
                line_start: row.get(4)?,
                impl_type: row.get(5)?,
            });
        }
        Ok(results)
    }

    pub fn get_callees_by_name(
        &self,
        symbol_name: &str,
        min_confidence: Option<&str>,
    ) -> SqlResult<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT ts.name, f.path \
             FROM symbol_refs sr \
             JOIN symbols ss ON ss.id = sr.source_symbol_id \
             JOIN symbols ts ON ts.id = sr.target_symbol_id \
             JOIN files f ON f.id = ts.file_id \
             WHERE ss.name = ?1 AND (?2 IS NULL OR sr.confidence = ?2)",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![symbol_name, min_confidence])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }
        Ok(results)
    }

    pub fn get_callers_by_name(
        &self,
        symbol_name: &str,
        min_confidence: Option<&str>,
    ) -> SqlResult<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT ss.name, sf.path \
             FROM symbol_refs sr \
             JOIN symbols ss ON ss.id = sr.source_symbol_id \
             JOIN symbols ts ON ts.id = sr.target_symbol_id \
             JOIN files sf ON sf.id = ss.file_id \
             WHERE ts.name = ?1 AND (?2 IS NULL OR sr.confidence = ?2)",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![symbol_name, min_confidence])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }
        Ok(results)
    }

    pub fn get_docs_for_dependency(&self, name: &str) -> SqlResult<Vec<DocResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.content, d.source, dep.name, dep.version, d.module \
             FROM docs d \
             JOIN dependencies dep ON dep.id = d.dependency_id \
             WHERE dep.name = ?1",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![name])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_doc_result(row)?);
        }
        Ok(results)
    }

    /// Begin an explicit transaction. Caller must call `commit()`
    /// on the returned guard, or it rolls back on drop.
    pub fn begin_transaction(&self) -> SqlResult<()> {
        self.conn.execute_batch("BEGIN")
    }

    pub fn commit(&self) -> SqlResult<()> {
        self.conn.execute_batch("COMMIT")
    }

    pub fn with_transaction<T>(&self, f: impl FnOnce(&Self) -> SqlResult<T>) -> SqlResult<T> {
        self.conn.execute_batch("BEGIN")?;
        let result = f(self);
        if result.is_ok() {
            self.conn.execute_batch("COMMIT")?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        result
    }

    /// Build an in-memory lookup table mapping symbol names to their DB IDs.
    /// Three tiers: (name, file), (name, impl type), and name-only.
    pub fn build_symbol_id_map(&self) -> SqlResult<SymbolIdMap> {
        let mut file_qualified = std::collections::HashMap::new();
        let mut impl_qualified = std::collections::HashMap::new();
        let mut name_only = std::collections::HashMap::new();

        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, f.path, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id: SymbolId = row.get(0)?;
            let name: String = row.get(1)?;
            let path: String = row.get(2)?;
            let impl_type: Option<String> = row.get(3)?;

            file_qualified.entry((name.clone(), path)).or_insert(id);
            if let Some(it) = impl_type {
                impl_qualified.entry((name.clone(), it)).or_insert(id);
            }
            name_only.entry(name).or_insert(id);
        }

        Ok(SymbolIdMap {
            file_qualified,
            impl_qualified,
            name_only,
        })
    }

    /// Insert symbol references using an in-memory lookup map instead of
    /// per-ref SQL queries. Much faster for large ref counts.
    pub fn store_symbol_refs_fast(
        &self,
        refs: &[crate::indexer::parser::SymbolRef],
        map: &SymbolIdMap,
    ) -> SqlResult<u64> {
        let mut count = 0;
        for r in refs {
            let source_id = map.resolve(&r.source_name, Some(&r.source_file), None);
            let target_id = map.resolve(
                &r.target_name,
                r.target_file.as_deref(),
                r.target_context.as_deref(),
            );
            if let (Some((sid, _)), Some((tid, confidence))) = (source_id, target_id) {
                self.insert_symbol_ref(sid, tid, &r.kind.to_string(), confidence)?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Insert symbol references from parsed refs, looking up IDs by name.
    /// Caller should wrap in a transaction for performance.
    pub fn store_symbol_refs(&self, refs: &[crate::indexer::parser::SymbolRef]) -> SqlResult<u64> {
        let mut count = 0;
        for r in refs {
            let source_id = self.get_symbol_id(&r.source_name, &r.source_file)?;
            let (target_id, confidence) = if let Some(ctx) = &r.target_context {
                if let Some(id) = self.get_symbol_id_in_impl(&r.target_name, ctx)? {
                    (Some(id), "high")
                } else if let Some(tf) = &r.target_file {
                    if let Some(id) = self.get_symbol_id(&r.target_name, tf)? {
                        (Some(id), "high")
                    } else {
                        (self.get_symbol_id_by_name(&r.target_name)?, "low")
                    }
                } else {
                    (self.get_symbol_id_by_name(&r.target_name)?, "low")
                }
            } else if let Some(target_file) = &r.target_file {
                if let Some(id) = self.get_symbol_id(&r.target_name, target_file)? {
                    (Some(id), "high")
                } else {
                    (self.get_symbol_id_by_name(&r.target_name)?, "low")
                }
            } else {
                (self.get_symbol_id_by_name(&r.target_name)?, "low")
            };
            if let (Some(sid), Some(tid)) = (source_id, target_id) {
                self.insert_symbol_ref(sid, tid, &r.kind.to_string(), confidence)?;
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn get_unreferenced_symbols(
        &self,
        path_prefix: Option<&str>,
        include_private: bool,
    ) -> SqlResult<Vec<StoredSymbol>> {
        let prefix = path_prefix.unwrap_or("");
        let like_pattern = format!("{}%", escape_like(prefix));
        let sql = if include_private {
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, \
                    s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             LEFT JOIN symbol_refs sr ON sr.target_symbol_id = s.id \
             WHERE sr.id IS NULL \
               AND f.path LIKE ?1 ESCAPE '\\' \
               AND s.kind NOT IN ('use', 'mod', 'impl') \
             ORDER BY f.path, s.line_start"
        } else {
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, \
                    s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             LEFT JOIN symbol_refs sr ON sr.target_symbol_id = s.id \
             WHERE sr.id IS NULL \
               AND f.path LIKE ?1 ESCAPE '\\' \
               AND s.kind NOT IN ('use', 'mod', 'impl') \
               AND s.visibility IN ('public', 'pub(crate)') \
             ORDER BY f.path, s.line_start"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![like_pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
        }
        Ok(results)
    }

    pub fn get_file_dependencies(
        &self,
        path_prefix: &str,
        min_confidence: Option<&str>,
    ) -> SqlResult<Vec<(String, String)>> {
        let pattern = format!("{}%", escape_like(path_prefix));
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT sf.path, tf.path \
             FROM symbol_refs sr \
             JOIN symbols ss ON ss.id = sr.source_symbol_id \
             JOIN symbols ts ON ts.id = sr.target_symbol_id \
             JOIN files sf ON sf.id = ss.file_id \
             JOIN files tf ON tf.id = ts.file_id \
             WHERE sf.path LIKE ?1 ESCAPE '\\' AND sf.path != tf.path \
               AND (?2 IS NULL OR sr.confidence = ?2) \
             ORDER BY sf.path, tf.path",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern, min_confidence])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }
        Ok(results)
    }

    /// Get symbols with the most incoming references (most depended-upon).
    pub fn get_most_referenced_symbols(
        &self,
        limit: i64,
        path_prefix: &str,
        min_confidence: Option<&str>,
    ) -> SqlResult<Vec<SymbolRefCount>> {
        let pattern = format!("{}%", escape_like(path_prefix));
        let mut stmt = self.conn.prepare_cached(
            "SELECT ts.name, f.path, \
                    COUNT(DISTINCT sr.source_symbol_id) as ref_count, \
                    ts.impl_type \
             FROM symbol_refs sr \
             JOIN symbols ts ON ts.id = sr.target_symbol_id \
             JOIN files f ON f.id = ts.file_id \
             WHERE f.path LIKE ?1 ESCAPE '\\' \
               AND (?3 IS NULL OR sr.confidence = ?3) \
             GROUP BY ts.id \
             ORDER BY ref_count DESC \
             LIMIT ?2",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern, limit, min_confidence])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
        }
        Ok(results)
    }

    /// Get symbols with the most outgoing references (most complex).
    pub fn get_most_referencing_symbols(
        &self,
        limit: i64,
        path_prefix: &str,
        min_confidence: Option<&str>,
    ) -> SqlResult<Vec<(String, String, i64)>> {
        let pattern = format!("{}%", escape_like(path_prefix));
        let mut stmt = self.conn.prepare_cached(
            "SELECT ss.name, f.path, \
                    COUNT(DISTINCT sr.target_symbol_id) as ref_count \
             FROM symbol_refs sr \
             JOIN symbols ss ON ss.id = sr.source_symbol_id \
             JOIN files f ON f.id = ss.file_id \
             WHERE f.path LIKE ?1 ESCAPE '\\' \
               AND (?3 IS NULL OR sr.confidence = ?3) \
             GROUP BY ss.id \
             ORDER BY ref_count DESC \
             LIMIT ?2",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern, limit, min_confidence])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(results)
    }

    /// Search for structs whose `details` field contains the given text.
    pub fn search_symbols_by_details(
        &self,
        query: &str,
        path_prefix: &str,
    ) -> SqlResult<Vec<StoredSymbol>> {
        let pattern = format!("%{}%", escape_like(query));
        let path_pattern = format!("{}%", escape_like(path_prefix));
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.details LIKE ?1 ESCAPE '\\' \
               AND s.kind = 'struct' \
               AND f.path LIKE ?2 ESCAPE '\\' \
             ORDER BY s.name \
             LIMIT 50",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern, path_pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
        }
        Ok(results)
    }

    /// Count symbols grouped by kind, scoped to a path prefix.
    pub fn count_symbols_by_kind(&self, path_prefix: &str) -> SqlResult<Vec<(String, i64)>> {
        let pattern = format!("{}%", escape_like(path_prefix));
        let mut stmt = self.conn.prepare(
            "SELECT s.kind, COUNT(*) \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE f.path LIKE ?1 ESCAPE '\\' \
             GROUP BY s.kind \
             ORDER BY s.kind",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }
        Ok(results)
    }

    /// Get the largest functions by line count, scoped to a path prefix.
    pub fn get_largest_functions(
        &self,
        limit: i64,
        path_prefix: &str,
    ) -> SqlResult<Vec<LargestFunction>> {
        let pattern = format!("{}%", escape_like(path_prefix));
        let mut stmt = self.conn.prepare(
            "SELECT s.name, f.path, s.impl_type, \
                    (s.line_end - s.line_start + 1) as lines \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.kind = 'function' AND f.path LIKE ?1 ESCAPE '\\' \
             ORDER BY lines DESC \
             LIMIT ?2",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern, limit])?;
        while let Some(row) = rows.next()? {
            results.push(LargestFunction {
                name: row.get(0)?,
                file_path: row.get(1)?,
                impl_type: row.get(2)?,
                lines: row.get(3)?,
            });
        }
        Ok(results)
    }
}

pub struct SymbolIdMap {
    file_qualified: std::collections::HashMap<(String, String), SymbolId>,
    impl_qualified: std::collections::HashMap<(String, String), SymbolId>,
    name_only: std::collections::HashMap<String, SymbolId>,
}

impl SymbolIdMap {
    /// Resolve a symbol name to its DB ID using a three-tier lookup:
    /// impl-qualified, then file-qualified (with `mod.rs` fallback),
    /// then name-only fallback.
    #[must_use]
    pub fn resolve(
        &self,
        name: &str,
        target_file: Option<&str>,
        target_context: Option<&str>,
    ) -> Option<(SymbolId, &'static str)> {
        if let Some(ctx) = target_context
            && let Some(id) = self
                .impl_qualified
                .get(&(name.to_string(), ctx.to_string()))
        {
            return Some((*id, "high"));
        }
        if let Some(file) = target_file {
            let key = (name.to_string(), file.to_string());
            if let Some(id) = self.file_qualified.get(&key) {
                return Some((*id, "high"));
            }
            // Try mod.rs variant: src/foo/bar.rs → src/foo/bar/mod.rs
            if let Some(alt) = mod_rs_alternative(file) {
                let alt_key = (name.to_string(), alt);
                if let Some(id) = self.file_qualified.get(&alt_key) {
                    return Some((*id, "high"));
                }
            }
        }
        self.name_only.get(name).map(|id| (*id, "low"))
    }
}

/// Generate the alternative path for module resolution:
/// `src/foo/bar.rs` → `src/foo/bar/mod.rs` and vice versa.
fn mod_rs_alternative(path: &str) -> Option<String> {
    path.strip_suffix("/mod.rs")
        .map(|stem| format!("{stem}.rs"))
        .or_else(|| {
            path.strip_suffix(".rs")
                .map(|stem| format!("{stem}/mod.rs"))
        })
}

#[derive(Debug, PartialEq, Eq)]
pub struct FileSymbolCount {
    pub path: String,
    pub count: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FileRecord {
    pub path: String,
    pub content_hash: String,
    pub crate_id: Option<CrateId>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ImpactEntry {
    pub name: String,
    pub file_path: String,
    pub depth: i64,
    pub via: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestEntry {
    pub name: String,
    pub file_path: String,
    pub line_start: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StoredCrate {
    pub id: CrateId,
    pub name: String,
    pub path: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DocResult {
    pub content: String,
    pub source: String,
    pub dependency_name: String,
    pub version: String,
    pub module: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StoredDep {
    pub name: String,
    pub version: String,
    pub is_direct: bool,
    pub repository_url: Option<String>,
    pub features: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StoredSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub body: Option<String>,
    pub details: Option<String>,
    pub attributes: Option<String>,
    pub impl_type: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StoredTraitImpl {
    pub type_name: String,
    pub trait_name: String,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct LargestFunction {
    pub name: String,
    pub file_path: String,
    pub impl_type: Option<String>,
    pub lines: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CalleeInfo {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub ref_kind: String,
    pub line_start: i64,
    pub impl_type: Option<String>,
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::Symbol;
    use crate::indexer::store::store_symbols;

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
        let id = db.insert_crate("hcfs-server", "hcfs-server").unwrap();
        assert!(id.0 > 0);
        let c = db.get_crate_by_name("hcfs-server").unwrap().unwrap();
        assert_eq!(c.name, "hcfs-server");
        assert_eq!(c.path, "hcfs-server");
    }

    #[test]
    fn test_insert_crate_dep() {
        let db = Database::open_in_memory().unwrap();
        let shared = db.insert_crate("shared", "shared").unwrap();
        let server = db.insert_crate("server", "server").unwrap();
        db.insert_crate_dep(server, shared).unwrap();
        let deps = db.get_crate_dependents(shared).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "server");
    }

    #[test]
    fn test_transitive_crate_dependents() {
        let db = Database::open_in_memory().unwrap();
        let shared = db.insert_crate("shared", "shared").unwrap();
        let client = db.insert_crate("client", "client").unwrap();
        let cli = db.insert_crate("cli", "cli").unwrap();
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
        let crate_id = db.insert_crate("mylib", "mylib").unwrap();
        let file_id = db
            .insert_file_with_crate("mylib/src/lib.rs", "hash", crate_id)
            .unwrap();
        assert!(file_id.0 > 0);
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
        let db = Database {
            conn,
            repo_root: None,
        };
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
        let caller_id = SymbolId(db.conn.last_insert_rowid());
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
        let callee_a_id = SymbolId(db.conn.last_insert_rowid());
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
        let callee_b_id = SymbolId(db.conn.last_insert_rowid());
        // Insert refs
        db.insert_symbol_ref(caller_id, callee_a_id, "call", "high")
            .unwrap();
        db.insert_symbol_ref(caller_id, callee_b_id, "type_ref", "high")
            .unwrap();
        let callees = db.get_callees("caller", "src/lib.rs").unwrap();
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

    #[test]
    fn test_delete_file_data() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        // Add a symbol, trait impl, and FTS entry
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'Foo', 'struct', 'public', 1, 5, 'pub struct Foo')",
                params![file_id],
            )
            .unwrap();
        let sym_id = SymbolId(db.conn.last_insert_rowid());
        db.conn
            .execute(
                "INSERT INTO symbols_fts (rowid, name, signature, doc_comment) \
                 VALUES (?1, 'Foo', 'pub struct Foo', '')",
                params![sym_id],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols_trigram (rowid, name) \
                 VALUES (?1, 'Foo')",
                params![sym_id],
            )
            .unwrap();
        db.insert_trait_impl("Foo", "Display", file_id, 10, 20)
            .unwrap();

        // Add a second file with a ref pointing at the first file's symbol
        let file2 = db.insert_file("src/main.rs", "hash2").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'main', 'function', 'public', 1, 10, 'fn main()')",
                params![file2],
            )
            .unwrap();
        let main_id = SymbolId(db.conn.last_insert_rowid());
        db.insert_symbol_ref(main_id, sym_id, "type_ref", "high")
            .unwrap();

        // Delete first file's data
        db.delete_file_data("src/lib.rs").unwrap();

        // Verify: file gone
        assert!(db.get_file_hash("src/lib.rs").unwrap().is_none());
        // Verify: symbol gone
        let syms = db.search_symbols("Foo").unwrap();
        assert!(syms.is_empty());
        // Verify: trait impl gone
        let impls = db.get_trait_impls_for_type("Foo").unwrap();
        assert!(impls.is_empty());
        // Verify: ref to deleted symbol gone
        let callees = db.get_callees("main", "src/main.rs").unwrap();
        assert!(callees.is_empty());
        // Verify: second file still intact
        assert!(db.get_file_hash("src/main.rs").unwrap().is_some());
    }

    #[test]
    fn test_search_symbols_exact_match_first() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        // Insert "ConfigHelper" first so without exact-match priority
        // it would sort before "Config" alphabetically or by rowid
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'ConfigHelper', 'struct', 'public', \
                  10, 20, 'pub struct ConfigHelper')",
                params![file_id],
            )
            .unwrap();
        let id1 = db.conn.last_insert_rowid();
        db.conn
            .execute(
                "INSERT INTO symbols_fts \
                 (rowid, name, signature, doc_comment) \
                 VALUES (?1, 'ConfigHelper', \
                  'pub struct ConfigHelper', '')",
                params![id1],
            )
            .unwrap();

        // Insert "Config" second (higher rowid)
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'Config', 'struct', 'public', \
                  1, 8, 'pub struct Config')",
                params![file_id],
            )
            .unwrap();
        let id2 = db.conn.last_insert_rowid();
        db.conn
            .execute(
                "INSERT INTO symbols_fts \
                 (rowid, name, signature, doc_comment) \
                 VALUES (?1, 'Config', 'pub struct Config', '')",
                params![id2],
            )
            .unwrap();

        let results = db.search_symbols("Config").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "Config");
        assert_eq!(results[1].name, "ConfigHelper");
    }

    #[test]
    fn test_search_symbols_substring_match() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        // Insert symbol with CamelCase name where "Conf" is a
        // mid-word substring that FTS5 prefix match cannot find
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'AppConfigLoader', 'struct', 'public', \
                         1, 10, 'pub struct AppConfigLoader')",
                params![file_id],
            )
            .unwrap();
        let rowid = db.conn.last_insert_rowid();
        db.conn
            .execute(
                "INSERT INTO symbols_fts \
                 (rowid, name, signature, doc_comment) \
                 VALUES (?1, 'AppConfigLoader', \
                         'pub struct AppConfigLoader', '')",
                params![rowid],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols_trigram (rowid, name) \
                 VALUES (?1, 'AppConfigLoader')",
                params![rowid],
            )
            .unwrap();
        // FTS5 prefix "onfig*" won't match "AppConfigLoader"
        // since the tokenizer treats the whole name as one token.
        // The trigram index catches substring matches.
        let results = db.search_symbols("onfig").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "AppConfigLoader");
    }

    #[test]
    fn test_search_symbols_escapes_like_metacharacters() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        // Insert symbol whose name contains a LIKE wildcard character
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'my_func', 'function', 'public', \
                         1, 5, 'pub fn my_func()')",
                params![file_id],
            )
            .unwrap();
        let rowid = db.conn.last_insert_rowid();
        db.conn
            .execute(
                "INSERT INTO symbols_fts (rowid, name, signature, doc_comment) \
                 VALUES (?1, 'my_func', 'pub fn my_func()', '')",
                params![rowid],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols_trigram (rowid, name) VALUES (?1, 'my_func')",
                params![rowid],
            )
            .unwrap();
        // Searching for "y_f" should NOT match via unescaped "_" wildcard
        // matching any character — it should match only because of trigram
        let results = db.search_symbols("y_f").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "my_func");
    }

    #[test]
    fn test_search_by_attribute() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, attributes) \
                 VALUES (?1, 'Config', 'struct', 'public', \
                         1, 5, 'pub struct Config', \
                         'derive(Debug, Clone, Serialize)')",
                params![file_id],
            )
            .unwrap();
        let results = db.search_symbols_by_attribute("Serialize").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Config");
    }

    #[test]
    fn test_search_by_doc_comment() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, doc_comment) \
                 VALUES (?1, 'parse_config', 'function', 'public', \
                         1, 10, 'pub fn parse_config()', \
                         'Parse configuration from TOML files.')",
                params![file_id],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'no_docs', 'function', 'public', \
                         11, 15, 'pub fn no_docs()')",
                params![file_id],
            )
            .unwrap();

        let results = db.search_symbols_by_doc_comment("TOML").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "parse_config");

        let results = db.search_symbols_by_doc_comment("nonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_by_body() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, body) \
                 VALUES (?1, 'risky_fn', 'function', 'public', \
                         1, 10, 'pub fn risky_fn()', \
                         'let val = map.get(key).unwrap();')",
                params![file_id],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, body) \
                 VALUES (?1, 'safe_fn', 'function', 'public', \
                         11, 20, 'pub fn safe_fn()', \
                         'let val = map.get(key).unwrap_or_default();')",
                params![file_id],
            )
            .unwrap();

        let results = db.search_symbols_by_body("unwrap()").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "risky_fn");

        let results = db.search_symbols_by_body("nonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_clear_code_index_preserves_docs() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'Foo', 'struct', 'public', \
                         1, 10, 'pub struct Foo')",
                params![file_id],
            )
            .unwrap();

        let dep_id = db.insert_dependency("serde", "1.0.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde docs content")
            .unwrap();

        db.clear_code_index().unwrap();

        // Symbols and files should be gone
        let syms = db.search_symbols("Foo").unwrap();
        assert!(syms.is_empty());
        let hash = db.get_file_hash("src/lib.rs").unwrap();
        assert!(hash.is_none());

        // Docs and dependencies should remain
        let doc_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM docs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 1);
        let dep_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM dependencies", [], |row| row.get(0))
            .unwrap();
        assert_eq!(dep_count, 1);
    }

    #[test]
    fn test_store_doc_with_module() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("tokio", "1.35.0", true, None).unwrap();

        // Store summary doc (empty module)
        db.store_doc_with_module(dep_id, "cargo_doc", "Tokio summary", "")
            .unwrap();
        // Store module-specific doc
        db.store_doc_with_module(dep_id, "cargo_doc", "Tokio net module", "net")
            .unwrap();
        db.store_doc_with_module(dep_id, "cargo_doc", "Tokio sync module", "sync")
            .unwrap();

        // get_doc_by_module finds summary
        let summary = db.get_doc_by_module("tokio", "").unwrap().unwrap();
        assert_eq!(summary.content, "Tokio summary");
        assert_eq!(summary.module, "");

        // get_doc_by_module finds specific module
        let net = db.get_doc_by_module("tokio", "net").unwrap().unwrap();
        assert_eq!(net.content, "Tokio net module");
        assert_eq!(net.module, "net");

        // get_doc_by_module returns None for unknown module
        assert!(db.get_doc_by_module("tokio", "io").unwrap().is_none());

        // get_doc_modules lists non-empty modules
        let modules = db.get_doc_modules("tokio").unwrap();
        assert_eq!(modules, vec!["net", "sync"]);

        // get_docs_for_dependency returns all docs including module field
        let all = db.get_docs_for_dependency("tokio").unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.iter().any(|d| d.module == "net"));
    }

    #[test]
    fn test_clear_code_index_then_reinsert_deps_preserves_matching_docs() {
        let db = Database::open_in_memory().unwrap();

        let dep_id = db.insert_dependency("tokio", "1.35.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Tokio async runtime docs")
            .unwrap();

        db.clear_code_index().unwrap();

        // Docs should still be accessible via search
        let results = db.search_docs("Tokio").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].dependency_name, "tokio");
        assert!(results[0].content.contains("Tokio async runtime"));

        // Dependency row still present
        let dep_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM dependencies \
                 WHERE name = 'tokio'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dep_count, 1);
    }

    #[test]
    fn test_get_symbols_at_lines() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        // Insert symbols at different line ranges
        for (name, start, end) in [("alpha", 1, 10), ("beta", 15, 25), ("gamma", 30, 40)] {
            db.conn
                .execute(
                    "INSERT INTO symbols \
                     (file_id, name, kind, visibility, \
                      line_start, line_end, signature) \
                     VALUES (?1, ?2, 'function', 'public', ?3, ?4, ?5)",
                    params![file_id, name, start, end, format!("fn {name}()")],
                )
                .unwrap();
        }

        // Range overlapping alpha only
        let results = db.get_symbols_at_lines("src/lib.rs", &[(5, 8)]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "alpha");

        // Range overlapping beta and gamma
        let results = db.get_symbols_at_lines("src/lib.rs", &[(20, 35)]).unwrap();
        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"gamma"));

        // Multiple ranges with deduplication
        let results = db
            .get_symbols_at_lines("src/lib.rs", &[(1, 5), (3, 8)])
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "alpha");

        // No overlap
        let results = db.get_symbols_at_lines("src/lib.rs", &[(11, 14)]).unwrap();
        assert!(results.is_empty());

        // Empty ranges
        let results = db.get_symbols_at_lines("src/lib.rs", &[]).unwrap();
        assert!(results.is_empty());

        // Wrong file
        let results = db.get_symbols_at_lines("src/other.rs", &[(1, 50)]).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_delete_stale_refs() {
        let db = Database::open_in_memory().unwrap();

        // Create two symbols with a valid ref between them
        let file1 = db.insert_file("src/a.rs", "hash1").unwrap();
        let file2 = db.insert_file("src/b.rs", "hash2").unwrap();

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'alpha', 'function', 'public', 1, 3, 'fn alpha()')",
                params![file1],
            )
            .unwrap();
        let alpha_id = SymbolId(db.conn.last_insert_rowid());

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'beta', 'function', 'public', 1, 3, 'fn beta()')",
                params![file2],
            )
            .unwrap();
        let beta_id = SymbolId(db.conn.last_insert_rowid());

        db.insert_symbol_ref(beta_id, alpha_id, "call", "high")
            .unwrap();

        // Verify the ref exists
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Delete alpha's symbol directly (bypassing FK by disabling checks),
        // simulating a scenario where the symbol row is gone but refs remain.
        db.conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        db.conn
            .execute("DELETE FROM symbols WHERE id = ?1", params![alpha_id])
            .unwrap();
        db.conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

        // The ref is now dangling (target_symbol_id points to nothing)
        let deleted = db.delete_stale_refs().unwrap();
        assert_eq!(deleted, 1, "should delete the dangling ref");

        let remaining: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_get_symbol_id_in_impl() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        // Insert a method with impl_type
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, impl_type) \
                 VALUES (?1, 'method', 'function', 'public', \
                         5, 10, 'pub fn method()', 'MyStruct')",
                params![file_id],
            )
            .unwrap();

        // Insert another method with same name, different impl_type
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, impl_type) \
                 VALUES (?1, 'method', 'function', 'public', \
                         15, 20, 'pub fn method()', 'OtherStruct')",
                params![file_id],
            )
            .unwrap();

        let my_id = db.get_symbol_id_in_impl("method", "MyStruct").unwrap();
        assert!(my_id.is_some());

        let other_id = db.get_symbol_id_in_impl("method", "OtherStruct").unwrap();
        assert!(other_id.is_some());

        // Different ids
        assert_ne!(my_id, other_id);

        // Non-existent impl type
        let missing = db.get_symbol_id_in_impl("method", "Nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_get_callers_by_name() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'caller_fn', 'function', 'public', \
                         1, 10, 'fn caller_fn()')",
                params![file_id],
            )
            .unwrap();
        let caller_id = SymbolId(db.conn.last_insert_rowid());
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'target_fn', 'function', 'public', \
                         12, 20, 'fn target_fn()')",
                params![file_id],
            )
            .unwrap();
        let target_id = SymbolId(db.conn.last_insert_rowid());
        db.insert_symbol_ref(caller_id, target_id, "call", "high")
            .unwrap();

        let callers = db.get_callers_by_name("target_fn", None).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].0, "caller_fn");
        assert_eq!(callers[0].1, "src/lib.rs");

        // No callers for caller_fn
        let empty = db.get_callers_by_name("caller_fn", None).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_get_file_dependencies() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/a.rs", "h1").unwrap();
        let f2 = db.insert_file("src/b.rs", "h2").unwrap();

        // Symbol in file a
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'foo', 'function', 'public', 1, 5, 'fn foo()')",
                params![f1],
            )
            .unwrap();
        let src_id = SymbolId(db.conn.last_insert_rowid());

        // Symbol in file b
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'bar', 'function', 'public', 1, 5, 'fn bar()')",
                params![f2],
            )
            .unwrap();
        let tgt_id = SymbolId(db.conn.last_insert_rowid());

        db.insert_symbol_ref(src_id, tgt_id, "call", "high")
            .unwrap();

        let edges = db.get_file_dependencies("src/", None).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0], ("src/a.rs".to_string(), "src/b.rs".to_string()));

        // Wrong prefix returns empty
        let empty = db.get_file_dependencies("other/", None).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_get_file_dependencies_excludes_self() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/a.rs", "h1").unwrap();

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'foo', 'function', 'public', 1, 5, 'fn foo()')",
                params![f1],
            )
            .unwrap();
        let s1 = SymbolId(db.conn.last_insert_rowid());

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'bar', 'function', 'public', 6, 10, 'fn bar()')",
                params![f1],
            )
            .unwrap();
        let s2 = SymbolId(db.conn.last_insert_rowid());

        db.insert_symbol_ref(s1, s2, "call", "high").unwrap();

        let edges = db.get_file_dependencies("src/", None).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn test_confidence_filtering() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/a.rs", "h1").unwrap();
        let f2 = db.insert_file("src/b.rs", "h2").unwrap();

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'caller', 'function', 'public', 1, 5, 'fn caller()')",
                params![f1],
            )
            .unwrap();
        let caller_id = SymbolId(db.conn.last_insert_rowid());

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'target', 'function', 'public', 1, 5, 'fn target()')",
                params![f2],
            )
            .unwrap();
        let target_id = SymbolId(db.conn.last_insert_rowid());

        // Insert a high-confidence ref
        db.insert_symbol_ref(caller_id, target_id, "call", "high")
            .unwrap();

        // Create another pair for a low-confidence ref
        let f3 = db.insert_file("src/c.rs", "h3").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'noise', 'function', 'public', 1, 5, 'fn noise()')",
                params![f3],
            )
            .unwrap();
        let noise_id = SymbolId(db.conn.last_insert_rowid());

        db.insert_symbol_ref(noise_id, target_id, "call", "low")
            .unwrap();

        // Without filter: both edges
        let all = db.get_file_dependencies("src/", None).unwrap();
        assert_eq!(all.len(), 2);

        // With high filter: only high-confidence edge
        let high_only = db.get_file_dependencies("src/", Some("high")).unwrap();
        assert_eq!(high_only.len(), 1);
        assert_eq!(
            high_only[0],
            ("src/a.rs".to_string(), "src/b.rs".to_string())
        );
    }

    #[test]
    fn test_get_most_referenced_symbols() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "h1").unwrap();
        // Create 3 symbols: a (target), b (source1), c (source2)
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, line_start, line_end, signature) \
                 VALUES (?1, 'a', 'fn', 'public', 1, 5, 'fn a()')",
                params![f1],
            )
            .unwrap();
        let a = SymbolId(db.conn.last_insert_rowid());
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, line_start, line_end, signature) \
                 VALUES (?1, 'b', 'fn', 'public', 6, 10, 'fn b()')",
                params![f1],
            )
            .unwrap();
        let b = SymbolId(db.conn.last_insert_rowid());
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, line_start, line_end, signature) \
                 VALUES (?1, 'c', 'fn', 'public', 11, 15, 'fn c()')",
                params![f1],
            )
            .unwrap();
        let c = SymbolId(db.conn.last_insert_rowid());

        // b -> a, c -> a (a has 2 incoming refs)
        db.insert_symbol_ref(b, a, "call", "high").unwrap();
        db.insert_symbol_ref(c, a, "call", "high").unwrap();
        // b -> c (c has 1 incoming ref)
        db.insert_symbol_ref(b, c, "call", "high").unwrap();

        let results = db.get_most_referenced_symbols(10, "", None).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
        assert_eq!(results[0].2, 2);
        assert_eq!(results[1].0, "c");
        assert_eq!(results[1].2, 1);
    }

    #[test]
    fn test_get_most_referencing_symbols() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "h1").unwrap();
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, line_start, line_end, signature) \
                 VALUES (?1, 'a', 'fn', 'public', 1, 5, 'fn a()')",
                params![f1],
            )
            .unwrap();
        let a = SymbolId(db.conn.last_insert_rowid());
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, line_start, line_end, signature) \
                 VALUES (?1, 'b', 'fn', 'public', 6, 10, 'fn b()')",
                params![f1],
            )
            .unwrap();
        let b = SymbolId(db.conn.last_insert_rowid());
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, line_start, line_end, signature) \
                 VALUES (?1, 'c', 'fn', 'public', 11, 15, 'fn c()')",
                params![f1],
            )
            .unwrap();
        let c = SymbolId(db.conn.last_insert_rowid());

        // b -> a, b -> c (b has 2 outgoing refs)
        db.insert_symbol_ref(b, a, "call", "high").unwrap();
        db.insert_symbol_ref(b, c, "call", "high").unwrap();
        // a -> c (a has 1 outgoing ref)
        db.insert_symbol_ref(a, c, "call", "high").unwrap();

        let results = db.get_most_referencing_symbols(10, "", None).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "b");
        assert_eq!(results[0].2, 2);
        assert_eq!(results[1].0, "a");
        assert_eq!(results[1].2, 1);
    }

    #[test]
    fn test_schema_version_triggers_clear() {
        let db = Database::open_in_memory().unwrap();
        // Insert a file to simulate an indexed DB
        let _file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        assert!(
            db.file_count().unwrap() > 0,
            "should have files before version bump"
        );

        // Simulate a stale version so next migrate detects a mismatch
        db.conn
            .execute(
                "INSERT OR REPLACE INTO schema_info (key, value) \
                 VALUES ('schema_version', 'old')",
                [],
            )
            .unwrap();

        // Re-run migrate — should detect version mismatch and clear
        db.migrate().unwrap();
        assert_eq!(
            db.file_count().unwrap(),
            0,
            "should be cleared after version bump"
        );

        // Verify schema version is now current
        let version: String = db
            .conn
            .query_row(
                "SELECT value FROM schema_info WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, Database::SCHEMA_VERSION);
    }

    #[test]
    fn test_search_symbols_by_details() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let matching = Symbol {
            name: "AppState".into(),
            kind: SymbolKind::Struct,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub struct AppState".into(),
            doc_comment: None,
            body: None,
            details: Some("config: Config, name: String".into()),
            attributes: None,
            impl_type: None,
        };
        let non_matching = Symbol {
            name: "Other".into(),
            kind: SymbolKind::Struct,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 10,
            line_end: 15,
            signature: "pub struct Other".into(),
            doc_comment: None,
            body: None,
            details: Some("count: usize".into()),
            attributes: None,
            impl_type: None,
        };
        store_symbols(&db, file_id, &[matching, non_matching]).unwrap();

        let results = db.search_symbols_by_details("Config", "").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "AppState");

        let empty = db.search_symbols_by_details("Nonexistent", "").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_count_symbols_by_kind() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let symbols = vec![
            Symbol {
                name: "foo".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn foo()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "bar".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn bar()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "MyStruct".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 15,
                signature: "pub struct MyStruct".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();

        let counts = db.count_symbols_by_kind("").unwrap();
        let fn_count = counts.iter().find(|(k, _)| k == "function");
        let struct_count = counts.iter().find(|(k, _)| k == "struct");
        assert_eq!(fn_count.unwrap().1, 2);
        assert_eq!(struct_count.unwrap().1, 1);
    }

    #[test]
    fn test_get_largest_functions() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let symbols = vec![
            Symbol {
                name: "small".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn small()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "large".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 10,
                line_end: 110,
                signature: "pub fn large()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "medium".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 120,
                line_end: 150,
                signature: "pub fn medium()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();

        let largest = db.get_largest_functions(2, "").unwrap();
        assert_eq!(largest.len(), 2);
        assert_eq!(largest[0].name, "large");
        assert_eq!(largest[0].lines, 101); // 110 - 10 + 1
        assert_eq!(largest[1].name, "medium");
        assert_eq!(largest[1].lines, 31); // 150 - 120 + 1
    }
}
