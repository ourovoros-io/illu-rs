# Data Quality & Performance Improvements Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce low-confidence refs from 64% to ~30%, fix 36 stale truncated signatures, and add targeted SQL queries to replace load-all-and-filter patterns for large-repo performance.

**Architecture:** Task 1 improves the ref resolution in `collect_body_refs` to resolve same-file and same-crate symbols without import maps. Task 2 forces a full re-index to regenerate all signatures with the new parser. Task 3 replaces 4 load-all-symbols patterns with targeted SQL queries.

**Tech Stack:** Rust 2024 edition, rusqlite, tree-sitter

---

## Task 1: Improve ref resolution — same-file and same-crate fallbacks

The biggest cause of low-confidence refs: when `fn foo()` calls `bar()` defined in the same file, `bar` isn't in the import map, so `target_file` is `None` → name-only fallback → low confidence.

**Fix:** In `collect_body_refs`, after the import map lookup fails, check if the symbol is defined in the same file (intra-file resolution). Also improve crate-level resolution for `crate::` qualified paths.

**Files:**
- Modify: `src/indexer/parser.rs` — update `collect_body_refs` to add same-file fallback

**Step 1: Update `collect_body_refs` to try same-file resolution**

In `src/indexer/parser.rs`, in `collect_body_refs`, the current code (around line 992) does:

```rust
let target_file = ctx.import_map.get(&name).and_then(|info| {
    qualified_path_to_file_with_crates(&info.qualified_path, ctx.crate_map)
});
```

Change to:

```rust
let target_file = ctx.import_map.get(&name)
    .and_then(|info| {
        qualified_path_to_file_with_crates(&info.qualified_path, ctx.crate_map)
    })
    .or_else(|| {
        // Same-file fallback: if the symbol is known and not imported,
        // it might be defined in the current file
        if ctx.known_symbols.contains(&name) {
            Some(ctx.file_path.to_string())
        } else {
            None
        }
    });
```

This is conservative — it uses the source file as `target_file` for ANY known symbol that isn't in the import map. This works because:
- If the symbol IS in the same file, the file-qualified lookup in `SymbolIdMap::resolve` will find it → high confidence
- If the symbol is NOT in the same file, the file-qualified lookup will fail, and it falls back to name-only → still low confidence (same as before)

So this can only IMPROVE confidence — it can't make anything worse.

**Step 2: Write a test**

Add to the parser test module:

```rust
#[test]
fn test_same_file_refs_have_target_file() {
    let source = r#"
fn helper() -> i32 { 42 }

pub fn caller() -> i32 {
    helper()
}
"#;
    let known_symbols: std::collections::HashSet<String> =
        ["helper", "caller"].iter().map(|s| s.to_string()).collect();
    let crate_map = std::collections::HashMap::new();
    let refs = extract_refs(source, "src/lib.rs", &known_symbols, &crate_map).unwrap();
    let caller_to_helper = refs.iter().find(|r| r.source_name == "caller" && r.target_name == "helper");
    assert!(caller_to_helper.is_some(), "should find ref from caller to helper");
    let r = caller_to_helper.unwrap();
    assert_eq!(r.target_file.as_deref(), Some("src/lib.rs"), "same-file ref should have target_file set");
}
```

**Step 3: Run tests, verify, commit**

```
cargo test --lib -- parser::tests::test_same_file_refs_have_target_file
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
git commit -m "fix: resolve same-file symbol refs to improve confidence from 64% to ~30%"
```

---

## Task 2: Force full re-index to fix stale truncated signatures

The 36 truncated signatures are stale data from before the `get_signature` fix. The index needs to be fully rebuilt. The simplest approach: delete the `clear_code_index` + re-run `index_repo` when the DB schema version changes.

**Files:**
- Modify: `src/db.rs` — add a schema version check that triggers full re-index

**Step 1: Add schema version metadata**

In `src/db.rs`, in the `migrate` method, add a schema version that gets bumped when breaking changes occur:

```rust
// At the end of migrate(), add:
const SCHEMA_VERSION: &str = "2";
let current_version: Option<String> = self.conn
    .query_row(
        "SELECT value FROM metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    )
    .ok();

if current_version.as_deref() != Some(SCHEMA_VERSION) {
    tracing::info!("Schema version changed, clearing code index for full re-index");
    self.clear_code_index()?;
    self.conn.execute(
        "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?1)",
        params![SCHEMA_VERSION],
    )?;
}
```

This means the next time the server starts, the old signatures get cleared and `index_repo` regenerates everything with the new `get_signature` function.

Check that the `metadata` table exists — it should already have `repo_path` and `commit_hash` entries. If not, create it in migrate.

**Step 2: Test by verifying the re-index happens**

After running the new binary, check that `handle_docs` now has a full signature:

```
sqlite3 .illu/index.db "SELECT signature FROM symbols WHERE name = 'handle_docs' AND kind = 'function'"
```

Expected: `pub fn handle_docs( db: &Database, dep_name: &str, topic: Option<&str>, ) -> Result<String, Box<dyn std::error::Error>>`

**Step 3: Commit**

```
git commit -m "fix: add schema versioning to trigger full re-index after parser changes"
```

---

## Task 3: Replace load-all-symbols patterns with targeted SQL queries

Four tools load ALL symbols via `get_symbols_by_path_prefix_filtered("", true)` and then filter in Rust. Replace with targeted SQL queries.

**Files:**
- Modify: `src/db.rs` — add targeted query methods
- Modify: `src/server/tools/rename_plan.rs` — use `search_symbols_by_details` instead of load-all
- Modify: `src/server/tools/type_usage.rs` — use `search_symbols_by_details` instead of load-all
- Modify: `src/server/tools/stats.rs` — use SQL COUNT queries instead of loading all symbols
- Modify: `src/server/tools/hotspots.rs` — use SQL query for largest functions

### 3a: Add `search_symbols_by_details` DB method

Replace the "load all structs and filter by details" pattern with a SQL query:

```rust
pub fn search_symbols_by_details(
    &self,
    query: &str,
    path_prefix: &str,
) -> SqlResult<Vec<StoredSymbol>> {
    let pattern = format!("%{}%", escape_like(query));
    let path_pattern = format!("{path_prefix}%");
    let mut stmt = self.conn.prepare(
        "SELECT s.name, s.kind, s.visibility, f.path, \
                s.line_start, s.line_end, s.signature, \
                s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         WHERE s.details LIKE ?1 ESCAPE '\\' \
           AND s.kind = 'struct' \
           AND f.path LIKE ?2 \
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
```

### 3b: Add `count_symbols_by_kind` DB method for stats

```rust
pub fn count_symbols_by_kind(
    &self,
    path_prefix: &str,
) -> SqlResult<Vec<(String, i64)>> {
    let pattern = format!("{path_prefix}%");
    let mut stmt = self.conn.prepare(
        "SELECT s.kind, COUNT(*) \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         WHERE f.path LIKE ?1 \
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
```

### 3c: Add `get_largest_functions` DB method for hotspots

```rust
pub fn get_largest_functions(
    &self,
    limit: i64,
    path_prefix: &str,
) -> SqlResult<Vec<(String, String, Option<String>, i64)>> {
    let pattern = format!("{path_prefix}%");
    let mut stmt = self.conn.prepare(
        "SELECT s.name, f.path, s.impl_type, (s.line_end - s.line_start + 1) as lines \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         WHERE s.kind = 'function' AND f.path LIKE ?1 \
         ORDER BY lines DESC \
         LIMIT ?2",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query(params![pattern, limit])?;
    while let Some(row) = rows.next()? {
        results.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
    }
    Ok(results)
}
```

### 3d: Update tools to use new methods

**`rename_plan.rs`:** Replace `get_symbols_by_path_prefix_filtered("", true)?` with `search_symbols_by_details(base_name, "")?`

**`type_usage.rs`:** Replace `get_symbols_by_path_prefix_filtered(prefix, true)?` with `search_symbols_by_details(type_name, prefix)?`

**`stats.rs`:** Replace the load-all + kind counting loop with `count_symbols_by_kind(prefix)?`

**`hotspots.rs`:** Replace the load-all + sort + truncate with `get_largest_functions(max, prefix)?`

### 3e: Tests, clippy, commit

Add DB tests for each new method. Verify existing tool tests still pass.

```
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
git commit -m "perf: replace load-all-symbols patterns with targeted SQL queries"
```

---

## Post-Implementation Verification

After all 3 tasks:

1. **Re-index the repo**: Run the binary to trigger schema version re-index
2. **Check health**: `illu health` should show higher high-confidence ratio (~60-70% vs current 35%)
3. **Check signatures**: `illu health` truncated count should be 0
4. **Run full test suite**: `cargo test` — all passing
5. **Verify file_graph noise reduction**: `illu file_graph src/server/tools/callpath.rs` should show only real deps
6. **Update CLAUDE.md** with final tool count and patterns

## Dependency Order

- Task 1 (ref quality) — independent, most impactful
- Task 2 (re-index) — independent, fixes stale data
- Task 3 (performance) — independent, improves scalability

All three are independent and can be done in any order. Recommended: 1, 2, 3.
