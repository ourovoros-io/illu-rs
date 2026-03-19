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

fn is_fts_safe(query: &str) -> bool {
    !query
        .chars()
        .any(|c| matches!(c, '"' | '%' | '\'' | '\\' | '*' | '(' | ')'))
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
                attributes TEXT
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

            CREATE INDEX IF NOT EXISTS idx_symbols_name
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
                ON docs(dependency_id);",
        )?;
        self.migrate_fts_schema()?;
        self.migrate_docs_module_column()
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
        let pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT f.path, COUNT(s.id) \
             FROM files f \
             LEFT JOIN symbols s ON s.file_id = f.id AND s.visibility = 'public' \
             WHERE f.path LIKE ?1 \
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
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO symbol_refs \
             (source_symbol_id, target_symbol_id, kind) \
             VALUES (?1, ?2, ?3)",
            params![source_id, target_id, kind],
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

    pub fn get_crate_count(&self) -> SqlResult<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM crates", [], |row| row.get(0))
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

    pub fn impact_dependents(&self, symbol_name: &str) -> SqlResult<Vec<ImpactEntry>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE deps(id, name, file_path, depth, via) AS (
                SELECT s.id, s.name, f.path, 0, ''
                FROM symbols s
                JOIN files f ON f.id = s.file_id
                WHERE s.name = ?1
              UNION
                SELECT s2.id, s2.name, f2.path, deps.depth + 1,
                       CASE WHEN deps.via = '' THEN deps.name
                            ELSE deps.via || ' -> ' || deps.name
                       END
                FROM deps
                JOIN symbol_refs sr ON sr.target_symbol_id = deps.id
                JOIN symbols s2 ON s2.id = sr.source_symbol_id
                JOIN files f2 ON f2.id = s2.file_id
                WHERE deps.depth < 5
            )
            SELECT DISTINCT name, file_path, depth, via FROM deps
            WHERE depth > 0
            ORDER BY depth, name
            LIMIT 100",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![symbol_name])?;
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
                           s.doc_comment, s.body, s.details, s.attributes \
                    FROM combined c \
                    JOIN symbols s ON s.id = c.sid \
                    JOIN files f ON f.id = s.file_id \
                    ORDER BY CASE WHEN s.name = ?2 THEN 0 ELSE 1 END, \
                             c.source, s.name \
                    LIMIT 50",
                    query.to_string(),
                )
            } else {
                let escaped = query.replace('%', r"\%").replace('_', r"\_");
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
                           s.doc_comment, s.body, s.details, s.attributes \
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
            let escaped = query.replace('%', r"\%").replace('_', r"\_");
            let like_pattern = format!("%{escaped}%");
            let mut stmt = self.conn.prepare_cached(
                "SELECT s.name, s.kind, s.visibility, f.path, \
                       s.line_start, s.line_end, s.signature, \
                       s.doc_comment, s.body, s.details, s.attributes \
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
                    s.doc_comment, s.body, s.details, s.attributes \
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
        let pattern = format!("{path_prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE f.path LIKE ?1 \
               AND s.visibility IN ('public', 'pub(crate)') \
             ORDER BY f.path, s.line_start",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
        }
        Ok(results)
    }

    pub fn search_symbols_by_attribute(&self, attr: &str) -> SqlResult<Vec<StoredSymbol>> {
        let pattern = format!("%{attr}%");
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.visibility, f.path, \
                    s.line_start, s.line_end, s.signature, \
                    s.doc_comment, s.body, s.details, s.attributes \
             FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.attributes LIKE ?1 \
             ORDER BY s.name",
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query(params![pattern])?;
        while let Some(row) = rows.next()? {
            results.push(row_to_stored_symbol(row)?);
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

    /// Insert symbol references from parsed refs, looking up IDs by name.
    /// Caller should wrap in a transaction for performance.
    pub fn store_symbol_refs(&self, refs: &[crate::indexer::parser::SymbolRef]) -> SqlResult<u64> {
        let mut count = 0;
        for r in refs {
            let source_id = self.get_symbol_id(&r.source_name, &r.source_file)?;
            let target_id = if let Some(target_file) = &r.target_file {
                self.get_symbol_id(&r.target_name, target_file)?
                    .or(self.get_symbol_id_by_name(&r.target_name)?)
            } else {
                self.get_symbol_id_by_name(&r.target_name)?
            };
            if let (Some(sid), Some(tid)) = (source_id, target_id) {
                self.insert_symbol_ref(sid, tid, &r.kind.to_string())?;
                count += 1;
            }
        }
        Ok(count)
    }
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
        db.insert_symbol_ref(main_id, sym_id, "type_ref").unwrap();

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
        let callees = db.get_callees("main").unwrap();
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

        db.insert_symbol_ref(beta_id, alpha_id, "call").unwrap();

        // Verify the ref exists
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM symbol_refs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Delete alpha's symbol directly (bypassing FK by disabling checks),
        // simulating a scenario where the symbol row is gone but refs remain.
        db.conn
            .execute_batch("PRAGMA foreign_keys = OFF")
            .unwrap();
        db.conn
            .execute("DELETE FROM symbols WHERE id = ?1", params![alpha_id])
            .unwrap();
        db.conn
            .execute_batch("PRAGMA foreign_keys = ON")
            .unwrap();

        // The ref is now dangling (target_symbol_id points to nothing)
        let deleted = db.delete_stale_refs().unwrap();
        assert_eq!(deleted, 1, "should delete the dangling ref");

        let remaining: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM symbol_refs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
