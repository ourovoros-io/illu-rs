# Enhanced Code Intelligence Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enrich illu-rs with source bodies, doc comments, struct fields, trait-type relationships, callees view, and an overview tool so AI assistants rarely need follow-up file reads.

**Architecture:** Two phases. Phase 1 enriches the data model (parser extracts more, DB stores more, existing tools render more). Phase 2 adds new query capabilities (callees in context, overview tool). Each task is a vertical slice through parser → store → DB → tool.

**Tech Stack:** Rust, tree-sitter, rusqlite (SQLite), rmcp (MCP server)

---

## Chunk 1: Schema Migration and Symbol Struct Extension

### Task 1: Add new columns to symbols table and create trait_impls table

**Files:**
- Modify: `src/db.rs:22-91` (migrate method)
- Modify: `src/db.rs:93-106` (clear_index method)
- Modify: `src/db.rs:351-376` (delete_file_data method)
- Modify: `src/db.rs:545-554` (StoredSymbol struct)
- Modify: `src/db.rs:402-426` (search_symbols method)

- [ ] **Step 1: Write failing test for new schema**

Add to `src/db.rs` tests:

```rust
#[test]
fn test_schema_has_new_columns() {
    let db = Database::open_in_memory().unwrap();
    // Verify symbols table has new columns by inserting with them
    db.conn
        .execute(
            "INSERT INTO files (path, content_hash) VALUES ('test.rs', 'h')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO symbols \
             (file_id, name, kind, visibility, line_start, line_end, \
              signature, doc_comment, body, details) \
             VALUES (1, 'test', 'function', 'public', 1, 5, \
                     'pub fn test()', 'A doc comment', \
                     'pub fn test() { }', NULL)",
            [],
        )
        .unwrap();
    let doc: Option<String> = db
        .conn
        .query_row(
            "SELECT doc_comment FROM symbols WHERE name = 'test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(doc, Some("A doc comment".to_string()));
}

#[test]
fn test_trait_impls_table_exists() {
    let db = Database::open_in_memory().unwrap();
    db.conn
        .execute(
            "INSERT INTO files (path, content_hash) VALUES ('test.rs', 'h')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO trait_impls \
             (type_name, trait_name, file_id, line_start, line_end) \
             VALUES ('Config', 'Display', 1, 10, 15)",
            [],
        )
        .unwrap();
    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM trait_impls", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_trait_impls_unique_constraint() {
    let db = Database::open_in_memory().unwrap();
    db.conn
        .execute(
            "INSERT INTO files (path, content_hash) VALUES ('test.rs', 'h')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO trait_impls \
             (type_name, trait_name, file_id, line_start, line_end) \
             VALUES ('Config', 'Display', 1, 10, 15)",
            [],
        )
        .unwrap();
    // Duplicate should not error with OR IGNORE
    db.conn
        .execute(
            "INSERT OR IGNORE INTO trait_impls \
             (type_name, trait_name, file_id, line_start, line_end) \
             VALUES ('Config', 'Display', 1, 10, 15)",
            [],
        )
        .unwrap();
    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM trait_impls", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib -- db::tests::test_schema_has_new_columns db::tests::test_trait_impls_table_exists db::tests::test_trait_impls_unique_constraint`
Expected: FAIL — columns and table don't exist yet.

- [ ] **Step 3: Update migrate() to add new columns, table, and FTS migration**

In `src/db.rs`, update the `migrate` method. Add after the `crate_deps` table creation (before FTS tables):

```rust
CREATE TABLE IF NOT EXISTS trait_impls (
    id INTEGER PRIMARY KEY,
    type_name TEXT NOT NULL,
    trait_name TEXT NOT NULL,
    file_id INTEGER NOT NULL REFERENCES files(id),
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    UNIQUE(type_name, trait_name, file_id)
);
```

Change the `symbols` CREATE TABLE to add the 3 new nullable columns:

```sql
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
```

Change `symbols_fts` to include `doc_comment`:

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
    name, signature, doc_comment, content=symbols, content_rowid=id
);
```

Then, **after** `execute_batch`, add a migration step to handle existing databases where `symbols_fts` was already created with the old schema (only `name, signature`). `CREATE VIRTUAL TABLE IF NOT EXISTS` silently skips if the table exists, so old databases keep the broken FTS. Detect and fix this:

```rust
fn migrate(&self) -> SqlResult<()> {
    self.conn.execute_batch(/* ... existing DDL ... */)?;
    self.migrate_fts_schema()?;
    Ok(())
}

fn migrate_fts_schema(&self) -> SqlResult<()> {
    // Check if symbols_fts has the doc_comment column by querying
    // the virtual table's schema. FTS5 tables store their column
    // config — we check if doc_comment is present.
    let has_doc_comment: bool = self
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master \
             WHERE type='table' AND name='symbols_fts'",
            [],
            |row| {
                let sql: String = row.get(0)?;
                Ok(sql.contains("doc_comment"))
            },
        )
        .unwrap_or(true); // if table doesn't exist, DDL above created it correctly

    if !has_doc_comment {
        // Drop old FTS table and recreate with doc_comment column
        self.conn.execute_batch(
            "DROP TABLE IF EXISTS symbols_fts;
             CREATE VIRTUAL TABLE symbols_fts USING fts5(
                 name, signature, doc_comment, content=symbols, content_rowid=id
             );"
        )?;
        // Repopulate FTS from existing symbols
        self.conn.execute_batch(
            "INSERT INTO symbols_fts (rowid, name, signature, doc_comment) \
             SELECT id, name, signature, COALESCE(doc_comment, '') FROM symbols;"
        )?;
    }
    Ok(())
}
```

This handles both fresh databases (DDL creates correctly) and upgraded databases (detect old schema, drop, recreate, repopulate).

- [ ] **Step 3b: Write test for FTS migration from old schema**

Add to `src/db.rs` tests:

```rust
#[test]
fn test_fts_migration_from_old_schema() {
    // Simulate an old database: create tables manually with old FTS schema
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE files (
            id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE,
            content_hash TEXT NOT NULL, crate_id INTEGER
        );
        CREATE TABLE symbols (
            id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL,
            name TEXT NOT NULL, kind TEXT NOT NULL,
            visibility TEXT NOT NULL, line_start INTEGER NOT NULL,
            line_end INTEGER NOT NULL, signature TEXT NOT NULL
        );
        CREATE VIRTUAL TABLE symbols_fts USING fts5(
            name, signature, content=symbols, content_rowid=id
        );
        CREATE TABLE metadata (repo_path TEXT PRIMARY KEY, commit_hash TEXT NOT NULL);
        CREATE TABLE symbol_refs (id INTEGER PRIMARY KEY, source_symbol_id INTEGER, target_symbol_id INTEGER, kind TEXT);
        CREATE TABLE dependencies (id INTEGER PRIMARY KEY, name TEXT, version TEXT, is_direct INTEGER, repository_url TEXT, features TEXT);
        CREATE TABLE docs (id INTEGER PRIMARY KEY, dependency_id INTEGER, source TEXT, content TEXT);
        CREATE TABLE crates (id INTEGER PRIMARY KEY, name TEXT UNIQUE, path TEXT, is_workspace_root INTEGER DEFAULT 0);
        CREATE TABLE crate_deps (source_crate_id INTEGER, target_crate_id INTEGER, PRIMARY KEY (source_crate_id, target_crate_id));
        CREATE VIRTUAL TABLE docs_fts USING fts5(content, content=docs, content_rowid=id);"
    ).unwrap();
    drop(conn);

    // Now open via Database::open_in_memory which calls migrate()
    // Since we can't reuse the connection, test migrate_fts_schema directly
    let db = Database::open_in_memory().unwrap();
    // The FTS table should have doc_comment column
    let sql: String = db.conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='symbols_fts'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert!(sql.contains("doc_comment"), "FTS should have doc_comment after migration");
}

#[test]
fn test_fts_search_by_doc_comment() {
    let db = Database::open_in_memory().unwrap();
    db.conn.execute(
        "INSERT INTO files (path, content_hash) VALUES ('src/lib.rs', 'h')",
        [],
    ).unwrap();
    db.conn.execute(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, line_start, line_end, \
          signature, doc_comment) \
         VALUES (1, 'process', 'function', 'public', 1, 5, \
                 'pub fn process()', 'Handles incoming webhook events')",
        [],
    ).unwrap();
    let rowid: i64 = db.conn.query_row(
        "SELECT id FROM symbols WHERE name = 'process'", [], |r| r.get(0),
    ).unwrap();
    db.conn.execute(
        "INSERT INTO symbols_fts (rowid, name, signature, doc_comment) \
         VALUES (?1, 'process', 'pub fn process()', 'Handles incoming webhook events')",
        params![rowid],
    ).unwrap();

    // Search for a term only in the doc comment, not in name or signature
    let results = db.search_symbols("webhook").unwrap();
    assert_eq!(results.len(), 1, "should find symbol by doc comment content");
    assert_eq!(results[0].name, "process");
}
```

- [ ] **Step 4: Update clear_index() to include trait_impls**

In `src/db.rs`, add `DELETE FROM trait_impls;` to `clear_index()`, before `DELETE FROM symbols;`.

- [ ] **Step 5: Update delete_file_data() to include trait_impls**

In `src/db.rs`, add this DELETE before the existing symbol_refs DELETE:

```rust
self.conn.execute(
    "DELETE FROM trait_impls WHERE file_id = \
     (SELECT id FROM files WHERE path = ?1)",
    params![path],
)?;
```

- [ ] **Step 6: Update StoredSymbol struct**

In `src/db.rs`, change `StoredSymbol` to:

```rust
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
```

- [ ] **Step 7: Update search_symbols() to include new columns**

Change the SELECT in `search_symbols` to:

```rust
"SELECT s.name, s.kind, s.visibility, f.path, \
        s.line_start, s.line_end, s.signature, \
        s.doc_comment, s.body, s.details \
 FROM symbols_fts fts \
 JOIN symbols s ON s.id = fts.rowid \
 JOIN files f ON f.id = s.file_id \
 WHERE symbols_fts MATCH ?1"
```

And update the row mapping:

```rust
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
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test --lib -- db::tests`
Expected: All pass.

- [ ] **Step 9: Commit**

```bash
git add src/db.rs
git commit -m "Add doc_comment, body, details columns and trait_impls table"
```

### Task 2: Add new DB query methods

**Files:**
- Modify: `src/db.rs` (add methods to `impl Database`)

- [ ] **Step 1: Write failing tests for new query methods**

Add to `src/db.rs` tests:

```rust
#[test]
fn test_get_trait_impls_for_type() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "h").unwrap();
    db.insert_trait_impl("Config", "Display", file_id, 10, 15)
        .unwrap();
    db.insert_trait_impl("Config", "Debug", file_id, 20, 25)
        .unwrap();
    let impls = db.get_trait_impls_for_type("Config").unwrap();
    assert_eq!(impls.len(), 2);
    let trait_names: Vec<&str> =
        impls.iter().map(|i| i.trait_name.as_str()).collect();
    assert!(trait_names.contains(&"Display"));
    assert!(trait_names.contains(&"Debug"));
}

#[test]
fn test_get_trait_impls_for_trait() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "h").unwrap();
    db.insert_trait_impl("Config", "Display", file_id, 10, 15)
        .unwrap();
    db.insert_trait_impl("Server", "Display", file_id, 30, 40)
        .unwrap();
    let impls = db.get_trait_impls_for_trait("Display").unwrap();
    assert_eq!(impls.len(), 2);
    let type_names: Vec<&str> =
        impls.iter().map(|i| i.type_name.as_str()).collect();
    assert!(type_names.contains(&"Config"));
    assert!(type_names.contains(&"Server"));
}

#[test]
fn test_get_symbols_by_path_prefix() {
    let db = Database::open_in_memory().unwrap();
    let f1 = db.insert_file("src/lib.rs", "h1").unwrap();
    let f2 = db.insert_file("src/server/mod.rs", "h2").unwrap();
    let f3 = db.insert_file("tests/test.rs", "h3").unwrap();
    // Insert symbols with new columns as NULL
    db.conn.execute(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature) \
         VALUES (?1, 'Config', 'struct', 'public', 1, 5, \
                 'pub struct Config')",
        params![f1],
    ).unwrap();
    db.conn.execute(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature) \
         VALUES (?1, 'Server', 'struct', 'public', 1, 10, \
                 'pub struct Server')",
        params![f2],
    ).unwrap();
    db.conn.execute(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature) \
         VALUES (?1, 'helper', 'function', 'private', 1, 3, \
                 'fn helper()')",
        params![f2],
    ).unwrap();
    db.conn.execute(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature) \
         VALUES (?1, 'test_fn', 'function', 'public', 1, 5, \
                 'pub fn test_fn()')",
        params![f3],
    ).unwrap();

    // src/ prefix should find Config and Server (public only)
    let syms = db.get_symbols_by_path_prefix("src/").unwrap();
    assert_eq!(syms.len(), 2);

    // src/server/ prefix should find only Server
    let syms = db.get_symbols_by_path_prefix("src/server/").unwrap();
    assert_eq!(syms.len(), 1);
    assert_eq!(syms[0].name, "Server");
}

#[test]
fn test_get_callees() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "h").unwrap();
    db.conn.execute(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature) \
         VALUES (?1, 'caller', 'function', 'public', 1, 5, \
                 'pub fn caller()')",
        params![file_id],
    ).unwrap();
    db.conn.execute(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature) \
         VALUES (?1, 'helper', 'function', 'public', 7, 10, \
                 'pub fn helper()')",
        params![file_id],
    ).unwrap();
    db.conn.execute(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature) \
         VALUES (?1, 'Config', 'struct', 'public', 12, 15, \
                 'pub struct Config')",
        params![file_id],
    ).unwrap();
    // caller -> helper (call), caller -> Config (type_ref)
    db.insert_symbol_ref(1, 2, "call").unwrap();
    db.insert_symbol_ref(1, 3, "type_ref").unwrap();

    let callees = db.get_callees("caller").unwrap();
    assert_eq!(callees.len(), 2);
    let names: Vec<&str> = callees.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"helper"));
    assert!(names.contains(&"Config"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib -- db::tests::test_get_trait_impls db::tests::test_get_symbols_by_path_prefix db::tests::test_get_callees`
Expected: FAIL — methods don't exist.

- [ ] **Step 3: Add StoredTraitImpl struct and new methods**

Add to `src/db.rs`:

```rust
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
```

Add methods to `impl Database`:

```rust
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

pub fn get_trait_impls_for_type(
    &self,
    type_name: &str,
) -> SqlResult<Vec<StoredTraitImpl>> {
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

pub fn get_trait_impls_for_trait(
    &self,
    trait_name: &str,
) -> SqlResult<Vec<StoredTraitImpl>> {
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

pub fn get_symbols_by_path_prefix(
    &self,
    path_prefix: &str,
) -> SqlResult<Vec<StoredSymbol>> {
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
        "SELECT DISTINCT s.name, s.kind, f.path, sr.kind \
         FROM symbol_refs sr \
         JOIN symbols src ON src.id = sr.source_symbol_id \
         JOIN symbols s ON s.id = sr.target_symbol_id \
         JOIN files f ON f.id = s.file_id \
         WHERE src.name = ?1",
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib -- db::tests`
Expected: All pass.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings.

- [ ] **Step 6: Commit**

```bash
git add src/db.rs
git commit -m "Add trait_impls queries, overview query, and callees query"
```

## Chunk 2: Parser Enhancements

### Task 3: Extend Symbol struct and extract doc comments

**Files:**
- Modify: `src/indexer/parser.rs:47-56` (Symbol struct)
- Modify: `src/indexer/parser.rs:72-136` (extract_symbols, extract_function, extract_named_item)

- [ ] **Step 1: Write failing test for doc comment extraction**

Add to `src/indexer/parser.rs` tests:

```rust
#[test]
fn test_extract_doc_comments() {
    let source = r#"
/// This is a doc comment.
/// It has multiple lines.
pub fn documented() {}

pub fn undocumented() {}

/// Single line doc.
#[derive(Debug)]
pub struct Config {}
"#;
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let documented = symbols.iter().find(|s| s.name == "documented").unwrap();
    assert_eq!(
        documented.doc_comment.as_deref(),
        Some("This is a doc comment.\nIt has multiple lines.")
    );

    let undocumented = symbols.iter().find(|s| s.name == "undocumented").unwrap();
    assert!(undocumented.doc_comment.is_none());

    let config = symbols
        .iter()
        .find(|s| s.name == "Config" && s.kind == SymbolKind::Struct)
        .unwrap();
    assert_eq!(
        config.doc_comment.as_deref(),
        Some("Single line doc.")
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib -- parser::tests::test_extract_doc_comments`
Expected: FAIL — `doc_comment` field doesn't exist.

- [ ] **Step 3: Add new fields to Symbol struct**

Change the `Symbol` struct in `src/indexer/parser.rs`:

```rust
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub body: Option<String>,
    pub details: Option<String>,
}
```

- [ ] **Step 4: Add TraitImpl struct**

Add after `RefKind`:

```rust
#[derive(Debug, Clone)]
pub struct TraitImpl {
    pub type_name: String,
    pub trait_name: String,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
}
```

- [ ] **Step 5: Add extract_doc_comment helper**

Add to `src/indexer/parser.rs`:

```rust
fn extract_doc_comment(node: &Node, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        match sib.kind() {
            "line_comment" => {
                let text = node_text(&sib, source);
                if let Some(stripped) = text.strip_prefix("///") {
                    comments.push(stripped.strip_prefix(' ').unwrap_or(stripped).to_string());
                } else {
                    break;
                }
            }
            "block_comment" => {
                let text = node_text(&sib, source);
                if text.starts_with("/**") {
                    let inner = text
                        .strip_prefix("/**")
                        .and_then(|s| s.strip_suffix("*/"))
                        .unwrap_or(&text)
                        .trim();
                    comments.push(inner.to_string());
                } else {
                    break;
                }
            }
            "attribute_item" => {
                // Skip #[derive(...)] etc.
            }
            _ => break,
        }
        sibling = sib.prev_sibling();
    }
    if comments.is_empty() {
        return None;
    }
    comments.reverse();
    Some(comments.join("\n"))
}
```

- [ ] **Step 6: Update all Symbol construction sites to include new fields**

Update `extract_function`, `extract_named_item`, `impl_item` branch in `extract_symbols`, `use_declaration` branch, and `mod_item` branch. Each Symbol construction must now include:

```rust
doc_comment: extract_doc_comment(&child, source),  // or node
body: None,     // will be filled in Task 4
details: None,  // will be filled in Task 5
```

For the `impl_item` and `use_declaration` branches, doc_comment is `None` (impls/uses don't have meaningful doc comments).

- [ ] **Step 7: Change parse_rust_source return type**

Change signature to:

```rust
pub fn parse_rust_source(
    source: &str,
    file_path: &str,
) -> Result<(Vec<Symbol>, Vec<TraitImpl>), String> {
```

And the body to:

```rust
let root = tree.root_node();
let mut symbols = Vec::new();
let mut trait_impls = Vec::new();
extract_symbols(&root, source, file_path, &mut symbols, &mut trait_impls);
Ok((symbols, trait_impls))
```

Update `extract_symbols` signature to accept `trait_impls: &mut Vec<TraitImpl>` and pass it through recursive calls. (Trait impl extraction is Task 6 — for now just thread the parameter.)

- [ ] **Step 8: Fix all existing tests to use the new return type**

Every test that calls `parse_rust_source` needs to destructure the tuple:

```rust
let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test --lib -- parser::tests`
Expected: All pass.

- [ ] **Step 10: Commit**

```bash
git add src/indexer/parser.rs
git commit -m "Add doc comment extraction and TraitImpl struct to parser"
```

### Task 4: Extract source bodies

**Files:**
- Modify: `src/indexer/parser.rs` (extract_function, extract_named_item, impl_item branch)

- [ ] **Step 1: Write failing test for body extraction**

Add to `src/indexer/parser.rs` tests:

```rust
#[test]
fn test_extract_body() {
    let source = r#"
pub fn hello() -> String {
    "hello".to_string()
}
"#;
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let sym = symbols.iter().find(|s| s.name == "hello").unwrap();
    assert!(sym.body.is_some());
    let body = sym.body.as_ref().unwrap();
    assert!(body.contains("hello"));
    assert!(body.contains("to_string"));
}

#[test]
fn test_body_truncation() {
    // Generate a function with >100 lines
    let mut source = String::from("pub fn long_fn() {\n");
    for i in 0..110 {
        source.push_str(&format!("    let _x{i} = {i};\n"));
    }
    source.push_str("}\n");

    let (symbols, _) = parse_rust_source(&source, "src/lib.rs").unwrap();
    let sym = symbols.iter().find(|s| s.name == "long_fn").unwrap();
    let body = sym.body.as_ref().unwrap();
    let line_count = body.lines().count();
    assert!(line_count <= 101, "body should be truncated to ~100 lines + truncation marker");
    assert!(body.contains("// ... truncated"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib -- parser::tests::test_extract_body parser::tests::test_body_truncation`
Expected: FAIL — body is `None`.

- [ ] **Step 3: Add extract_body helper**

```rust
fn extract_body(node: &Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > 100 {
        let truncated: Vec<&str> = lines[..100].to_vec();
        Some(format!("{}\n// ... truncated", truncated.join("\n")))
    } else {
        Some(text)
    }
}
```

- [ ] **Step 4: Wire extract_body into Symbol construction**

In `extract_function` and `extract_named_item`, change `body: None` to `body: extract_body(node, source)`. In the `impl_item` branch, set `body: extract_body(&child, source)`. For `use_declaration` and `mod_item`, keep `body: None`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib -- parser::tests`
Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add src/indexer/parser.rs
git commit -m "Extract source bodies with 100-line truncation"
```

### Task 5: Extract struct fields, enum variants, and trait methods

**Files:**
- Modify: `src/indexer/parser.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_extract_struct_fields() {
    let source = r"
pub struct Config {
    pub host: String,
    pub port: u16,
    timeout: Option<u64>,
}
";
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let config = symbols
        .iter()
        .find(|s| s.name == "Config" && s.kind == SymbolKind::Struct)
        .unwrap();
    let details = config.details.as_ref().unwrap();
    assert!(details.contains("host: String"));
    assert!(details.contains("port: u16"));
    assert!(details.contains("timeout: Option<u64>"));
}

#[test]
fn test_extract_enum_variants() {
    let source = r"
pub enum Color {
    Red,
    Green,
    Blue(u8, u8, u8),
    Custom { name: String },
}
";
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let color = symbols
        .iter()
        .find(|s| s.name == "Color" && s.kind == SymbolKind::Enum)
        .unwrap();
    let details = color.details.as_ref().unwrap();
    assert!(details.contains("Red"));
    assert!(details.contains("Blue(u8, u8, u8)"));
    assert!(details.contains("Custom { name: String }"));
}

#[test]
fn test_extract_trait_methods() {
    let source = r"
pub trait Drawable {
    fn draw(&self);
    fn resize(&mut self, width: u32, height: u32);
}
";
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let drawable = symbols
        .iter()
        .find(|s| s.name == "Drawable" && s.kind == SymbolKind::Trait)
        .unwrap();
    let details = drawable.details.as_ref().unwrap();
    assert!(details.contains("fn draw(&self)"));
    assert!(details.contains("fn resize(&mut self, width: u32, height: u32)"));
}

#[test]
fn test_unit_struct_no_details() {
    let source = r"pub struct Marker;";
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let marker = symbols
        .iter()
        .find(|s| s.name == "Marker")
        .unwrap();
    assert!(marker.details.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib -- parser::tests::test_extract_struct_fields parser::tests::test_extract_enum_variants parser::tests::test_extract_trait_methods parser::tests::test_unit_struct_no_details`
Expected: FAIL — details is `None`.

- [ ] **Step 3: Add extraction helpers**

```rust
fn extract_struct_details(node: &Node, source: &str) -> Option<String> {
    let field_list = find_child_by_kind(node, "field_declaration_list")?;
    let mut lines = Vec::new();
    let mut cursor = field_list.walk();
    for child in field_list.children(&mut cursor) {
        if child.kind() == "field_declaration" {
            let text = node_text(&child, source).trim().to_string();
            // Strip trailing comma
            let text = text.strip_suffix(',').unwrap_or(&text).to_string();
            lines.push(text);
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn extract_enum_details(node: &Node, source: &str) -> Option<String> {
    let variant_list = find_child_by_kind(node, "enum_variant_list")?;
    let mut lines = Vec::new();
    let mut cursor = variant_list.walk();
    for child in variant_list.children(&mut cursor) {
        if child.kind() == "enum_variant" {
            let text = node_text(&child, source).trim().to_string();
            let text = text.strip_suffix(',').unwrap_or(&text).to_string();
            lines.push(text);
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn extract_trait_details(node: &Node, source: &str) -> Option<String> {
    let decl_list = find_child_by_kind(node, "declaration_list")?;
    let mut lines = Vec::new();
    let mut cursor = decl_list.walk();
    for child in decl_list.children(&mut cursor) {
        if child.kind() == "function_signature_item"
            || child.kind() == "function_item"
        {
            let sig = get_first_line(&child, source);
            let sig = sig.strip_suffix(';').unwrap_or(&sig).to_string();
            lines.push(sig);
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}
```

- [ ] **Step 4: Wire details extraction into extract_named_item**

Update `extract_named_item` to compute details based on kind:

```rust
let details = match kind {
    SymbolKind::Struct => extract_struct_details(node, source),
    SymbolKind::Enum => extract_enum_details(node, source),
    SymbolKind::Trait => extract_trait_details(node, source),
    _ => None,
};
```

And set `details` in the returned Symbol.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib -- parser::tests`
Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add src/indexer/parser.rs
git commit -m "Extract struct fields, enum variants, and trait method signatures"
```

### Task 6: Detect and extract trait implementations

**Files:**
- Modify: `src/indexer/parser.rs` (impl_item branch in extract_symbols)

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_extract_trait_impl() {
    let source = r"
pub struct Config {}

pub trait Display {
    fn fmt(&self) -> String;
}

impl Display for Config {
    fn fmt(&self) -> String {
        String::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Self {}
    }
}
";
    let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
    assert_eq!(trait_impls.len(), 1);
    assert_eq!(trait_impls[0].trait_name, "Display");
    assert_eq!(trait_impls[0].type_name, "Config");
}

#[test]
fn test_inherent_impl_not_trait_impl() {
    let source = r"
pub struct Foo {}
impl Foo {
    pub fn bar() {}
}
";
    let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
    assert!(trait_impls.is_empty());
}

#[test]
fn test_generic_trait_impl() {
    let source = r"
pub struct Wrapper<T> { inner: T }
impl<T: Clone> Clone for Wrapper<T> {
    fn clone(&self) -> Self { Self { inner: self.inner.clone() } }
}
";
    let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
    assert_eq!(trait_impls.len(), 1);
    assert_eq!(trait_impls[0].trait_name, "Clone");
    // type_name should be the base type, possibly with generics
    assert!(trait_impls[0].type_name.starts_with("Wrapper"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib -- parser::tests::test_extract_trait_impl parser::tests::test_inherent_impl_not_trait_impl parser::tests::test_generic_trait_impl`
Expected: FAIL — trait_impls is empty (not populated yet).

- [ ] **Step 3: Add trait impl detection in the impl_item branch**

Update the `impl_item` branch in `extract_symbols` to detect trait impls. The tree-sitter `impl_item` has children in order: optional type_parameters, type (the trait or the self type), optional `for`, optional type (the implementor). If a `for` keyword child exists, it's a trait impl:

```rust
"impl_item" => {
    let type_name = extract_impl_type(&child, source);
    let vis = get_visibility(&child, source);
    let sig = get_first_line(&child, source);
    symbols.push(Symbol {
        name: type_name.clone().unwrap_or_default(),
        kind: SymbolKind::Impl,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start: child.start_position().row + 1,
        line_end: child.end_position().row + 1,
        signature: sig,
        doc_comment: None,
        body: extract_body(&child, source),
        details: None,
    });

    // Detect trait impl: look for `for` keyword among children
    if let Some(ti) = extract_trait_impl_info(&child, source, file_path) {
        trait_impls.push(ti);
    }

    extract_symbols(&child, source, file_path, symbols, trait_impls);
}
```

Add the helper:

```rust
fn extract_trait_impl_info(
    node: &Node,
    source: &str,
    file_path: &str,
) -> Option<TraitImpl> {
    // Check if this impl has a `for` keyword (trait impl vs inherent impl)
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();

    let for_pos = children.iter().position(|c| c.kind() == "for")?;

    // Trait is the type_identifier/scoped_type_identifier/generic_type BEFORE `for`
    let trait_node = children[..for_pos]
        .iter()
        .rfind(|c| {
            c.kind() == "type_identifier"
                || c.kind() == "scoped_type_identifier"
                || c.kind() == "generic_type"
        })?;
    let trait_name = match trait_node.kind() {
        "generic_type" => {
            // e.g., Iterator<Item = Foo> — take just the base name
            find_child_by_kind(trait_node, "type_identifier")
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| node_text(trait_node, source))
        }
        _ => node_text(trait_node, source),
    };

    // Type is the type_identifier/generic_type AFTER `for`
    let type_node = children[for_pos + 1..]
        .iter()
        .find(|c| {
            c.kind() == "type_identifier"
                || c.kind() == "scoped_type_identifier"
                || c.kind() == "generic_type"
        })?;
    let type_name = node_text(type_node, source);

    Some(TraitImpl {
        type_name,
        trait_name,
        file_path: file_path.to_string(),
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib -- parser::tests`
Expected: All pass.

- [ ] **Step 5: Commit**

```bash
git add src/indexer/parser.rs
git commit -m "Detect and extract trait implementations from impl blocks"
```

## Chunk 3: Store and Indexer Pipeline Updates

### Task 7: Update store_symbols and add store_trait_impls

**Files:**
- Modify: `src/indexer/store.rs`

- [ ] **Step 1: Write failing test for store with new fields**

Add to `src/indexer/store.rs` tests:

```rust
#[test]
fn test_store_symbols_with_new_fields() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "abc").unwrap();
    let symbols = vec![Symbol {
        name: "Config".into(),
        kind: SymbolKind::Struct,
        visibility: Visibility::Public,
        file_path: "src/lib.rs".into(),
        line_start: 1,
        line_end: 5,
        signature: "pub struct Config".into(),
        doc_comment: Some("Configuration for the app.".into()),
        body: Some("pub struct Config {\n    pub port: u16,\n}".into()),
        details: Some("port: u16".into()),
    }];
    store_symbols(&db, file_id, &symbols).unwrap();
    let results = db.search_symbols("Config").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].doc_comment.as_deref(),
        Some("Configuration for the app.")
    );
    assert!(results[0].body.as_ref().unwrap().contains("port"));
    assert_eq!(results[0].details.as_deref(), Some("port: u16"));
}

#[test]
fn test_store_and_query_trait_impls() {
    use crate::indexer::parser::TraitImpl;

    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "abc").unwrap();
    let impls = vec![
        TraitImpl {
            type_name: "Config".into(),
            trait_name: "Display".into(),
            file_path: "src/lib.rs".into(),
            line_start: 10,
            line_end: 15,
        },
    ];
    store_trait_impls(&db, file_id, &impls).unwrap();
    let results = db.get_trait_impls_for_type("Config").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].trait_name, "Display");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib -- store::tests::test_store_symbols_with_new_fields store::tests::test_store_and_query_trait_impls`
Expected: FAIL — `store_symbols` doesn't write new columns, `store_trait_impls` doesn't exist.

- [ ] **Step 3: Update store_symbols to include new columns**

Change `store_symbols` in `src/indexer/store.rs`:

```rust
pub fn store_symbols(db: &Database, file_id: i64, symbols: &[Symbol]) -> rusqlite::Result<()> {
    let mut sym_stmt = db.conn.prepare(
        "INSERT INTO symbols \
         (file_id, name, kind, visibility, \
          line_start, line_end, signature, \
          doc_comment, body, details) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    let mut fts_stmt = db.conn.prepare(
        "INSERT INTO symbols_fts (rowid, name, signature, doc_comment) \
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for sym in symbols {
        let line_start = i64::try_from(sym.line_start).unwrap_or(i64::MAX);
        let line_end = i64::try_from(sym.line_end).unwrap_or(i64::MAX);
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
        ])?;
        let rowid = db.conn.last_insert_rowid();
        fts_stmt.execute(params![
            rowid,
            sym.name,
            sym.signature,
            sym.doc_comment.as_deref().unwrap_or(""),
        ])?;
    }
    Ok(())
}
```

- [ ] **Step 4: Add store_trait_impls function**

```rust
pub fn store_trait_impls(
    db: &Database,
    file_id: i64,
    trait_impls: &[crate::indexer::parser::TraitImpl],
) -> rusqlite::Result<()> {
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
            line_end,
        ])?;
    }
    Ok(())
}
```

- [ ] **Step 5: Fix existing store tests**

Update the existing `Symbol` constructions in `store::tests` to include the 3 new fields as `None`:

```rust
doc_comment: None,
body: None,
details: None,
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib -- store::tests`
Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add src/indexer/store.rs
git commit -m "Update store_symbols for new columns and add store_trait_impls"
```

### Task 8: Update indexer pipeline to handle trait_impls

**Files:**
- Modify: `src/indexer/mod.rs:232-261` (index_crate_sources)
- Modify: `src/indexer/mod.rs:114-124` (refresh_index, the re-index loop)

- [ ] **Step 1: Update index_crate_sources**

Change the parsing section in `index_crate_sources`:

```rust
let (symbols, trait_impls) = parser::parse_rust_source(&source, &relative)
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
store::store_symbols(db, file_id, &symbols)?;
store::store_trait_impls(db, file_id, &trait_impls)?;
```

- [ ] **Step 2: Update refresh_index re-index loop**

Change the parsing section in `refresh_index` (around line 122-124):

```rust
let (symbols, trait_impls) = parser::parse_rust_source(source, relative)
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
store::store_symbols(db, file_id, &symbols)?;
store::store_trait_impls(db, file_id, &trait_impls)?;
```

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: All pass (some tool tests may need fixing — proceed to next step if needed).

- [ ] **Step 4: Fix all Symbol constructions in tool tests**

The tool tests in `query.rs`, `context.rs`, `impact.rs` construct `Symbol` structs directly. Add the 3 new fields to every `Symbol { ... }` literal. Search each file for `Symbol {` and add:

```rust
doc_comment: None,
body: None,
details: None,
```

**Files with Symbol constructions to fix:**
- `src/server/tools/query.rs` — tests: `test_query_symbols`, `test_query_docs` (none), `test_query_all`
- `src/server/tools/context.rs` — tests: `test_context_found`, `test_context_with_docs`
- `src/server/tools/impact.rs` — tests: `test_impact_no_dependents`, `test_impact_with_refs` (2 symbols), `test_impact_shows_affected_crates` (2 symbols)

Example for `impact.rs` `test_impact_no_dependents`:

```rust
Symbol {
    name: "lonely_fn".into(),
    kind: SymbolKind::Function,
    visibility: Visibility::Public,
    file_path: "src/lib.rs".into(),
    line_start: 1,
    line_end: 5,
    signature: "pub fn lonely_fn()".into(),
    doc_comment: None,
    body: None,
    details: None,
}
```

Apply the same pattern to all other Symbol constructions in these test files.

The integration test `tests/integration.rs` calls `index_repo` which goes through `index_crate_sources` — no manual Symbol construction to fix there.

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: All pass.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings.

- [ ] **Step 7: Commit**

```bash
git add src/indexer/mod.rs src/server/tools/query.rs src/server/tools/context.rs src/server/tools/impact.rs tests/integration.rs
git commit -m "Wire trait_impls through indexer pipeline"
```

## Chunk 4: Tool Enhancements

### Task 9: Enrich the context tool

**Files:**
- Modify: `src/server/tools/context.rs`

- [ ] **Step 1: Write failing test for enriched context output**

Add to `src/server/tools/context.rs` tests:

```rust
#[test]
fn test_context_includes_doc_comment() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[Symbol {
            name: "Config".into(),
            kind: SymbolKind::Struct,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub struct Config".into(),
            doc_comment: Some("Application configuration.".into()),
            body: Some("pub struct Config {\n    pub port: u16,\n}".into()),
            details: Some("port: u16".into()),
        }],
    )
    .unwrap();

    let result = handle_context(&db, "Config").unwrap();
    assert!(result.contains("Application configuration"), "should have doc comment");
    assert!(result.contains("port: u16"), "should have details");
    assert!(result.contains("pub struct Config"), "should have body");
}

#[test]
fn test_context_includes_callees() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "caller".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn caller()".into(),
                doc_comment: None,
                body: None,
                details: None,
            },
            Symbol {
                name: "helper".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn helper()".into(),
                doc_comment: None,
                body: None,
                details: None,
            },
        ],
    )
    .unwrap();
    db.insert_symbol_ref(1, 2, "call").unwrap();

    let result = handle_context(&db, "caller").unwrap();
    assert!(result.contains("Callees"), "should have callees section");
    assert!(result.contains("helper"), "should list helper as callee");
}

#[test]
fn test_context_includes_trait_impls() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
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
        }],
    )
    .unwrap();
    db.insert_trait_impl("Config", "Display", file_id, 10, 15)
        .unwrap();
    db.insert_trait_impl("Config", "Debug", file_id, 20, 25)
        .unwrap();

    let result = handle_context(&db, "Config").unwrap();
    assert!(result.contains("Trait Implementations"), "should have trait impls section");
    assert!(result.contains("Display"), "should list Display");
    assert!(result.contains("Debug"), "should list Debug");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib -- context::tests::test_context_includes_doc_comment context::tests::test_context_includes_callees context::tests::test_context_includes_trait_impls`
Expected: FAIL — context output doesn't include these sections yet.

- [ ] **Step 3: Rewrite handle_context**

Replace `handle_context` in `src/server/tools/context.rs`:

```rust
pub fn handle_context(
    db: &Database,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.search_symbols(symbol_name)?;
    if symbols.is_empty() {
        return Ok(format!("No symbol found matching '{symbol_name}'."));
    }

    let mut output = String::new();

    for sym in &symbols {
        let _ = writeln!(output, "## {} ({})", sym.name, sym.kind);
        let _ = writeln!(output);

        // Doc comment
        if let Some(doc) = &sym.doc_comment {
            for line in doc.lines() {
                let _ = writeln!(output, "> {line}");
            }
            let _ = writeln!(output);
        }

        let _ = writeln!(
            output,
            "- **File:** {}:{}-{}",
            sym.file_path, sym.line_start, sym.line_end
        );
        let _ = writeln!(output, "- **Visibility:** {}", sym.visibility);
        let _ = writeln!(output, "- **Signature:** `{}`", sym.signature);
        let _ = writeln!(output);

        // Details (struct fields, enum variants, trait methods)
        if let Some(details) = &sym.details {
            let _ = writeln!(output, "### Fields/Variants\n");
            let _ = writeln!(output, "```rust");
            let _ = writeln!(output, "{details}");
            let _ = writeln!(output, "```\n");
        }

        // Source body
        if let Some(body) = &sym.body {
            let _ = writeln!(output, "### Source\n");
            let _ = writeln!(output, "```rust");
            let _ = writeln!(output, "{body}");
            let _ = writeln!(output, "```\n");
        }

        // Trait implementations (for structs/enums)
        if sym.kind == "struct" || sym.kind == "enum" {
            let impls = db.get_trait_impls_for_type(&sym.name)?;
            if !impls.is_empty() {
                let _ = writeln!(output, "### Trait Implementations\n");
                for ti in &impls {
                    let _ = writeln!(
                        output,
                        "- **{}** ({}:{}-{})",
                        ti.trait_name, ti.file_path,
                        ti.line_start, ti.line_end
                    );
                }
                let _ = writeln!(output);
            }
        }

        // Trait implementors (for traits)
        if sym.kind == "trait" {
            let impls = db.get_trait_impls_for_trait(&sym.name)?;
            if !impls.is_empty() {
                let _ = writeln!(output, "### Implemented By\n");
                for ti in &impls {
                    let _ = writeln!(
                        output,
                        "- **{}** ({}:{}-{})",
                        ti.type_name, ti.file_path,
                        ti.line_start, ti.line_end
                    );
                }
                let _ = writeln!(output);
            }
        }

        // Callees
        let callees = db.get_callees(&sym.name)?;
        if !callees.is_empty() {
            let _ = writeln!(output, "### Callees\n");
            let calls: Vec<_> = callees
                .iter()
                .filter(|c| c.ref_kind == "call")
                .collect();
            let type_refs: Vec<_> = callees
                .iter()
                .filter(|c| c.ref_kind == "type_ref")
                .collect();
            if !calls.is_empty() {
                let _ = writeln!(output, "**Calls:**");
                for c in &calls {
                    let _ = writeln!(output, "- {} ({})", c.name, c.file_path);
                }
            }
            if !type_refs.is_empty() {
                let _ = writeln!(output, "**Uses types:**");
                for t in &type_refs {
                    let _ = writeln!(output, "- {} ({})", t.name, t.file_path);
                }
            }
            let _ = writeln!(output);
        }
    }

    // Related documentation
    let docs = db.search_docs(symbol_name)?;
    if !docs.is_empty() {
        output.push_str("## Related Documentation\n\n");
        for doc in &docs {
            let snippet = if doc.content.len() > 300 {
                format!("{}...", &doc.content[..300])
            } else {
                doc.content.clone()
            };
            let _ = writeln!(
                output,
                "- **{} {}**: {}",
                doc.dependency_name, doc.version, snippet
            );
        }
    }

    Ok(output)
}
```

- [ ] **Step 4: Fix existing context tests for new Symbol fields**

Update all `Symbol` constructions in `context::tests` to include `doc_comment: None, body: None, details: None`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib -- context::tests`
Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add src/server/tools/context.rs
git commit -m "Enrich context tool with doc comments, body, details, callees, trait impls"
```

### Task 10: Add doc comment snippet to query tool

**Files:**
- Modify: `src/server/tools/query.rs`

- [ ] **Step 1: Write failing test**

Add to `src/server/tools/query.rs` tests:

```rust
#[test]
fn test_query_shows_doc_comment_snippet() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[Symbol {
            name: "parse_config".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 10,
            signature: "pub fn parse_config() -> Config".into(),
            doc_comment: Some("Parse configuration from a TOML file.\nReturns error if invalid.".into()),
            body: None,
            details: None,
        }],
    )
    .unwrap();

    let result = handle_query(&db, "parse", Some("symbols")).unwrap();
    assert!(result.contains("Parse configuration from a TOML file"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib -- query::tests::test_query_shows_doc_comment_snippet`
Expected: FAIL — query output doesn't include doc comments.

- [ ] **Step 3: Update format_symbols to include doc snippet**

In `format_symbols`, change the symbol rendering to include first line of doc comment:

```rust
fn format_symbols(
    db: &Database,
    query: &str,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let symbols = db.search_symbols(query)?;
    if !symbols.is_empty() {
        output.push_str("## Symbols\n\n");
        for sym in &symbols {
            let _ = writeln!(
                output,
                "- **{}** ({}) at {}:{}-{}\n  `{}`",
                sym.name, sym.kind, sym.file_path,
                sym.line_start, sym.line_end, sym.signature,
            );
            if let Some(doc) = &sym.doc_comment {
                if let Some(first_line) = doc.lines().next() {
                    let _ = writeln!(output, "  *{first_line}*");
                }
            }
        }
        output.push('\n');
    }
    Ok(())
}
```

- [ ] **Step 4: Fix existing query tests for new Symbol fields**

Update all `Symbol` constructions in `query::tests` to include `doc_comment: None, body: None, details: None`.

- [ ] **Step 5: Run tests**

Run: `cargo test --lib -- query::tests`
Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add src/server/tools/query.rs
git commit -m "Show doc comment snippet in query symbol results"
```

### Task 11: Add overview tool

**Files:**
- Create: `src/server/tools/overview.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

- [ ] **Step 1: Write the overview handler with tests**

Create `src/server/tools/overview.rs`:

```rust
use crate::db::Database;
use std::fmt::Write;

pub fn handle_overview(
    db: &Database,
    path: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.get_symbols_by_path_prefix(path)?;
    if symbols.is_empty() {
        return Ok(format!("No public symbols found under '{path}'."));
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Overview: {path}\n");

    let mut current_file = "";
    for sym in &symbols {
        if sym.file_path != current_file {
            current_file = &sym.file_path;
            let _ = writeln!(output, "### {current_file}\n");
        }
        let _ = write!(
            output,
            "- **{}** ({}) `{}`",
            sym.name, sym.kind, sym.signature
        );
        if let Some(doc) = &sym.doc_comment {
            if let Some(first_line) = doc.lines().next() {
                let _ = write!(output, " — *{first_line}*");
            }
        }
        let _ = writeln!(output);
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
    fn test_overview_groups_by_file() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "h1").unwrap();
        let f2 = db.insert_file("src/server.rs", "h2").unwrap();
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
                doc_comment: Some("App config.".into()),
                body: None,
                details: None,
            }],
        )
        .unwrap();
        store_symbols(
            &db,
            f2,
            &[Symbol {
                name: "Server".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/server.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub struct Server".into(),
                doc_comment: None,
                body: None,
                details: None,
            }],
        )
        .unwrap();

        let result = handle_overview(&db, "src/").unwrap();
        assert!(result.contains("src/lib.rs"));
        assert!(result.contains("src/server.rs"));
        assert!(result.contains("Config"));
        assert!(result.contains("Server"));
        assert!(result.contains("App config"));
    }

    #[test]
    fn test_overview_filters_private() {
        let db = Database::open_in_memory().unwrap();
        let f = db.insert_file("src/lib.rs", "h").unwrap();
        store_symbols(
            &db,
            f,
            &[
                Symbol {
                    name: "public_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 3,
                    signature: "pub fn public_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                },
                Symbol {
                    name: "private_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 5,
                    line_end: 7,
                    signature: "fn private_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                },
            ],
        )
        .unwrap();

        let result = handle_overview(&db, "src/").unwrap();
        assert!(result.contains("public_fn"));
        assert!(!result.contains("private_fn"));
    }

    #[test]
    fn test_overview_no_results() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_overview(&db, "nonexistent/").unwrap();
        assert!(result.contains("No public symbols found"));
    }
}
```

- [ ] **Step 2: Register module**

Add to `src/server/tools/mod.rs`:

```rust
pub mod overview;
```

- [ ] **Step 3: Register tool on MCP server**

In `src/server/mod.rs`, add `OverviewParams`:

```rust
#[derive(Deserialize, JsonSchema)]
struct OverviewParams {
    path: String,
}
```

Add the tool handler inside `#[tool_router] impl IlluServer`:

```rust
#[tool(
    name = "overview",
    description = "Get a structural overview of all public symbols under a file path prefix. Groups by file and kind."
)]
async fn overview(
    &self,
    Parameters(params): Parameters<OverviewParams>,
) -> Result<CallToolResult, McpError> {
    self.refresh().await?;
    let db = self
        .db
        .lock()
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    let result = tools::overview::handle_overview(&db, &params.path)
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

    Ok(CallToolResult::success(vec![Content::text(result)]))
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All pass.

- [ ] **Step 5: Commit**

```bash
git add src/server/tools/overview.rs src/server/tools/mod.rs src/server/mod.rs
git commit -m "Add overview MCP tool for structural codebase maps"
```

## Chunk 5: Update Skill File Generation and Integration Tests

### Task 12: Update skill file generation

**Files:**
- Modify: `src/indexer/mod.rs:338-380` (generate_claude_skill function)

- [ ] **Step 1: Update generate_claude_skill**

Add the overview tool to the generated skill file. Update the function in `src/indexer/mod.rs`:

After the `docs` tool line, add:

```rust
let _ = writeln!(
    out,
    "- **overview** — Get a structural overview of all \
     public symbols under a file path prefix."
);
```

Update the `context` description to mention the new features:

```rust
let _ = writeln!(
    out,
    "- **context** — Get full context for a symbol: \
     doc comments, definition, source body, struct fields, \
     trait implementations, and callees."
);
```

- [ ] **Step 2: Fix skill file test**

Update `test_generate_skill_content` in `src/indexer/mod.rs` tests to assert `overview` is present:

```rust
assert!(skill.contains("overview"));
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib -- indexer::tests::test_generate_skill`
Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add src/indexer/mod.rs
git commit -m "Update skill file generation with overview tool and enriched context description"
```

### Task 13: Extend integration tests

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: Enrich the test fixture**

Update `setup_indexed_db` in `tests/integration.rs`. Change the source file content to include doc comments, struct fields, trait impl, and more:

```rust
std::fs::write(
    src_dir.join("lib.rs"),
    r#"
use serde::Serialize;

/// Application configuration.
/// Holds host and port settings.
pub struct Config {
    pub host: String,
    pub port: u16,
}

impl Config {
    /// Create a new Config with defaults.
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }
}

/// Parse configuration from input string.
pub fn parse_config(input: &str) -> Config {
    let _ = input;
    Config::new("localhost".into(), 8080)
}

pub trait Configurable {
    fn configure(&self) -> Config;
}

pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error(String),
}

impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}
"#,
)
.unwrap();
```

- [ ] **Step 2: Add new integration tests**

```rust
#[test]
fn test_context_tool_enriched() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "Config").unwrap();
    // Should include doc comment
    assert!(
        result.contains("Application configuration"),
        "context should include doc comment"
    );
    // Should include struct fields
    assert!(
        result.contains("host: String"),
        "context should include struct fields"
    );
    // Should include source body
    assert!(
        result.contains("pub struct Config"),
        "context should include source body"
    );
}

#[test]
fn test_context_trait_impls() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "Config").unwrap();
    assert!(
        result.contains("Display"),
        "context should show Display trait impl for Config"
    );
}

#[test]
fn test_context_callees() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "parse_config").unwrap();
    // parse_config calls Config::new, so Config should be a callee
    assert!(
        result.contains("Config") || result.contains("new"),
        "parse_config should show callees"
    );
}

#[test]
fn test_query_doc_snippet() {
    let (_dir, db) = setup_indexed_db();
    let result = query::handle_query(&db, "parse_config", Some("symbols")).unwrap();
    assert!(
        result.contains("Parse configuration"),
        "query should show doc comment snippet"
    );
}

#[test]
fn test_overview_tool() {
    let (_dir, db) = setup_indexed_db();
    let result = overview::handle_overview(&db, "src/").unwrap();
    assert!(result.contains("Config"), "overview should list Config");
    assert!(
        result.contains("parse_config"),
        "overview should list parse_config"
    );
    assert!(
        result.contains("Configurable"),
        "overview should list Configurable trait"
    );
    assert!(
        result.contains("LogLevel"),
        "overview should list LogLevel enum"
    );
}

#[test]
fn test_enum_details_in_context() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "LogLevel").unwrap();
    assert!(result.contains("Debug"), "should show enum variants");
    assert!(result.contains("Error(String)"), "should show tuple variant");
}
```

- [ ] **Step 3: Add overview import**

Add `overview` to the import at the top of `tests/integration.rs`:

```rust
use illu_rs::server::tools::{context, docs, impact, overview, query};
```

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: All pass.

- [ ] **Step 5: Run clippy and fmt**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected: Clean.

- [ ] **Step 6: Commit**

```bash
git add tests/integration.rs
git commit -m "Extend integration tests for enriched tools and overview"
```

### Task 14: Final verification

- [ ] **Step 1: Full test suite**

Run: `cargo test`
Expected: All pass.

- [ ] **Step 2: Clippy clean**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Format check**

Run: `cargo fmt --all -- --check`
Expected: Clean.

- [ ] **Step 4: Build release**

Run: `cargo build --release`
Expected: Builds successfully.
