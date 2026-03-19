# v3 Accuracy Improvements Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the symbol reference graph more complete and accurate: method-level refs, cross-crate resolution, and stale ref cleanup.

**Architecture:** Three independent changes: (1) delete dangling symbol_refs after incremental re-index, (2) pass workspace crate map to qualified_path_to_file for cross-crate resolution, (3) track impl_type on symbols and detect self.method() calls for method-level refs.

**Tech Stack:** Rust, tree-sitter, rusqlite, SQLite

---

### Task 1: Delete stale refs after incremental re-index

Simplest change — one new DB method, one call site. Prevents dangling `symbol_refs` rows after `refresh_index`.

**Files:**
- Modify: `src/db.rs` (add `delete_stale_refs`)
- Modify: `src/indexer/mod.rs` (call it in `refresh_index`)
- Test: `src/db.rs` test module, `src/indexer/mod.rs` test module

**Step 1: Write failing test for `delete_stale_refs`**

Add to `src/db.rs` test module:

```rust
#[test]
fn test_delete_stale_refs() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
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
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "target".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn target()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let caller_id = db.get_symbol_id("caller", "src/lib.rs").unwrap().unwrap();
    let target_id = db.get_symbol_id("target", "src/lib.rs").unwrap().unwrap();
    db.insert_symbol_ref(caller_id, target_id, "call").unwrap();

    // Delete target symbol, leaving a dangling ref
    db.delete_file_data("src/lib.rs").unwrap();
    let file_id2 = db.insert_file("src/lib.rs", "hash2").unwrap();
    store_symbols(
        &db,
        file_id2,
        &[Symbol {
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
            attributes: None,
            impl_type: None,
        }],
    )
    .unwrap();

    // Stale ref should be cleaned up
    let deleted = db.delete_stale_refs().unwrap();
    assert!(deleted > 0, "should delete at least one stale ref");
}
```

Note: This test uses `impl_type: None` on `Symbol` — this field will be added in Task 3. If implementing Task 1 before Task 3, omit the `impl_type` field (it doesn't exist yet). The test structure remains the same.

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- db::tests::test_delete_stale_refs`
Expected: FAIL — `delete_stale_refs` method doesn't exist.

**Step 3: Implement `delete_stale_refs`**

Add to `src/db.rs` Database impl:

```rust
/// Delete symbol_refs where source or target symbol no longer exists.
/// Returns the number of deleted rows.
pub fn delete_stale_refs(&self) -> SqlResult<u64> {
    let deleted = self.conn.execute(
        "DELETE FROM symbol_refs \
         WHERE source_symbol_id NOT IN (SELECT id FROM symbols) \
            OR target_symbol_id NOT IN (SELECT id FROM symbols)",
        [],
    )?;
    Ok(u64::try_from(deleted).unwrap_or(0))
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib -- db::tests::test_delete_stale_refs`
Expected: PASS

**Step 5: Call `delete_stale_refs` in `refresh_index`**

In `src/indexer/mod.rs`, at the end of `refresh_index` (after the ref extraction loop and `db.commit()`), add:

```rust
let stale = db.delete_stale_refs()?;
if stale > 0 {
    tracing::info!(deleted = stale, "Cleaned up stale symbol refs");
}
```

**Step 6: Write integration test**

Add to `src/indexer/mod.rs` test module:

```rust
#[test]
fn test_refresh_cleans_stale_refs() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // Two functions, one calls the other
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn caller() { target(); }\npub fn target() {}\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Verify ref exists
    let deps = db.impact_dependents("target").unwrap();
    assert!(
        deps.iter().any(|d| d.name == "caller"),
        "caller should depend on target"
    );

    // Remove target function
    std::fs::write(src_dir.join("lib.rs"), "pub fn caller() {}\n").unwrap();
    refresh_index(&db, &config).unwrap();

    // Stale ref should be gone — no dependents for non-existent "target"
    let deps = db.impact_dependents("target").unwrap();
    assert!(deps.is_empty(), "stale refs should be cleaned up");
}
```

**Step 7: Run full tests and lint**

Run: `cargo test --lib && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings.

**Step 8: Commit**

```bash
git add src/db.rs src/indexer/mod.rs
git commit -m "fix: delete stale symbol refs after incremental re-index

delete_stale_refs() removes symbol_refs rows where the source or
target symbol no longer exists. Called at the end of refresh_index."
```

---

### Task 2: Cross-crate qualified resolution in workspaces

Pass a crate-name-to-path map to `extract_refs` so `use shared::Config` resolves to `shared/src/config.rs`.

**Files:**
- Modify: `src/indexer/parser.rs` (update `extract_refs`, `qualified_path_to_file` signatures)
- Modify: `src/indexer/mod.rs` (build crate map, pass to `extract_refs`)
- Modify: `src/db.rs` (add `get_all_crates` if missing)
- Test: `src/indexer/parser.rs` test module

**Step 1: Write failing test for cross-crate path resolution**

Add to `src/indexer/parser.rs` test module:

```rust
#[test]
fn test_qualified_path_to_file_cross_crate() {
    let mut crate_map = std::collections::HashMap::new();
    crate_map.insert("shared".to_string(), "shared".to_string());
    crate_map.insert("api".to_string(), "api".to_string());

    // shared::config::Config -> shared/src/config.rs
    let result = qualified_path_to_file_with_crates("shared::config::Config", &crate_map);
    assert_eq!(result, Some("shared/src/config.rs".to_string()));

    // api::models::User -> api/src/models.rs
    let result = qualified_path_to_file_with_crates("api::models::User", &crate_map);
    assert_eq!(result, Some("api/src/models.rs".to_string()));

    // crate:: still works
    let result = qualified_path_to_file_with_crates("crate::db::Database", &crate_map);
    assert_eq!(result, Some("src/db.rs".to_string()));

    // External crate not in map -> None
    let result = qualified_path_to_file_with_crates("serde::Serialize", &crate_map);
    assert!(result.is_none());

    // Single-segment crate path (shared::Config) -> shared/src/lib.rs
    let result = qualified_path_to_file_with_crates("shared::Config", &crate_map);
    assert_eq!(result, Some("shared/src/lib.rs".to_string()));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- parser::tests::test_qualified_path_to_file_cross_crate`
Expected: FAIL — `qualified_path_to_file_with_crates` doesn't exist.

**Step 3: Implement `qualified_path_to_file_with_crates`**

In `src/indexer/parser.rs`, add a new function and update the old one:

```rust
/// Convert a qualified path to a relative file path, using crate map for
/// cross-crate resolution in workspaces.
fn qualified_path_to_file_with_crates(
    qualified_path: &str,
    crate_map: &std::collections::HashMap<String, String>,
) -> Option<String> {
    // Try crate:: prefix first (current crate)
    if let Some(path) = qualified_path.strip_prefix("crate::") {
        let segments: Vec<&str> = path.split("::").collect();
        if segments.len() < 2 {
            return None;
        }
        let module_segments = &segments[..segments.len() - 1];
        return Some(format!("src/{}.rs", module_segments.join("/")));
    }

    // Try workspace crate prefixes
    let segments: Vec<&str> = qualified_path.split("::").collect();
    if segments.len() < 2 {
        return None;
    }
    let crate_name = segments[0];
    let crate_path = crate_map.get(crate_name)?;

    if segments.len() == 2 {
        // shared::Config -> shared/src/lib.rs
        return Some(format!("{crate_path}/src/lib.rs"));
    }
    // shared::config::Config -> shared/src/config.rs
    let module_segments = &segments[1..segments.len() - 1];
    Some(format!("{crate_path}/src/{}.rs", module_segments.join("/")))
}
```

Update the old `qualified_path_to_file` to delegate (for backward compatibility in tests):

```rust
fn qualified_path_to_file(qualified_path: &str) -> Option<String> {
    qualified_path_to_file_with_crates(qualified_path, &std::collections::HashMap::new())
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib -- parser::tests::test_qualified_path_to_file`
Expected: All pass (new and old tests).

**Step 5: Update `extract_refs` to accept crate map**

Update `extract_refs` signature:

```rust
pub fn extract_refs<S: std::hash::BuildHasher>(
    source: &str,
    file_path: &str,
    known_symbols: &std::collections::HashSet<String, S>,
    crate_map: &std::collections::HashMap<String, String>,
) -> Result<Vec<SymbolRef>, String> {
```

Thread `crate_map` through `collect_refs` → `collect_body_refs`. In `collect_body_refs`, change:

```rust
// Before:
let target_file = import_map
    .get(&name)
    .and_then(|info| qualified_path_to_file(&info.qualified_path));

// After:
let target_file = import_map
    .get(&name)
    .and_then(|info| qualified_path_to_file_with_crates(
        &info.qualified_path,
        crate_map,
    ));
```

**Step 6: Add `get_all_crates` to DB if missing**

Check if `get_all_crates` exists. If not, add to `src/db.rs`:

```rust
pub fn get_all_crates(&self) -> SqlResult<Vec<StoredCrate>> {
    let mut stmt = self.conn.prepare("SELECT id, name, path FROM crates")?;
    let mut results = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        results.push(row_to_stored_crate(row)?);
    }
    Ok(results)
}
```

**Step 7: Update callers in `src/indexer/mod.rs`**

In `extract_all_symbol_refs`, build the crate map before the file loop:

```rust
let all_crates = db.get_all_crates()?;
let crate_map: std::collections::HashMap<String, String> = all_crates
    .iter()
    .map(|c| (c.name.clone(), c.path.clone()))
    .collect();
```

Pass `&crate_map` to `parser::extract_refs`.

In `refresh_index`, same — build crate map before the dirty file ref loop, pass to `parser::extract_refs`.

**Step 8: Update all other callers of `extract_refs`**

Search for all calls to `parser::extract_refs` or `extract_refs` and add the `&crate_map` parameter. For tests that call `extract_refs` directly, pass `&HashMap::new()`.

**Step 9: Run full tests and lint**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings.

**Step 10: Commit**

```bash
git add src/indexer/parser.rs src/indexer/mod.rs src/db.rs
git commit -m "feat: cross-crate qualified resolution in workspaces

extract_refs now accepts a crate map (name -> path) built from the
crates table. qualified_path_to_file resolves use shared::Config
to shared/src/lib.rs in workspace projects."
```

---

### Task 3: Method-level refs via impl-block awareness

Track which impl type a method belongs to, detect `self.method()` calls, and resolve them precisely.

**Files:**
- Modify: `src/indexer/parser.rs` (add `impl_type` to `Symbol`, `target_context` to `SymbolRef`, update `extract_symbols`, `collect_refs`, `collect_body_refs`)
- Modify: `src/indexer/store.rs` (store `impl_type`)
- Modify: `src/db.rs` (schema migration, `get_symbol_id_in_impl`, update `store_symbol_refs`)
- Test: all three files' test modules

**Step 1: Add `impl_type` to `Symbol` struct**

In `src/indexer/parser.rs`, update the `Symbol` struct:

```rust
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
    pub attributes: Option<String>,
    /// For methods inside `impl Type`, the type name.
    pub impl_type: Option<String>,
}
```

Update ALL places that construct `Symbol` to include `impl_type: None`:
- `extract_function` (line ~303)
- `extract_named_item` (wherever it constructs Symbol)
- `extract_macro_def` (line ~409)
- `extract_symbols` impl_item branch (line ~237)
- `extract_symbols` use_declaration branch
- Any test code that constructs Symbol

**Step 2: Pass impl type through `extract_symbols`**

Modify `extract_symbols` to accept `impl_type: Option<&str>` parameter. At the root level, pass `None`. When recursing into an `impl_item`:

```rust
"impl_item" => {
    let type_name = extract_impl_type(&child, source);
    // ... existing impl symbol creation ...
    extract_symbols_with_impl(
        &child, source, file_path, symbols, trait_impls,
        type_name.as_deref(),
    );
}
```

When extracting functions inside an impl, set `impl_type`:

```rust
"function_item" => {
    if let Some(mut sym) = extract_function(&child, source, file_path) {
        sym.impl_type = impl_type.map(String::from);
        symbols.push(sym);
    }
}
```

Rather than adding a new function, the simplest approach is to add `impl_type_name: Option<&str>` as a parameter to `extract_symbols` itself. The initial call from `parse_rust_source` passes `None`. The recursive call inside `impl_item` passes `type_name.as_deref()`.

**Step 3: Write test for impl_type on symbols**

Add to parser tests:

```rust
#[test]
fn test_symbol_impl_type() {
    let source = r#"
pub struct MyStruct;

impl MyStruct {
    pub fn method(&self) {}
}

pub fn free_function() {}
"#;
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();

    let method = symbols.iter().find(|s| s.name == "method").unwrap();
    assert_eq!(method.impl_type.as_deref(), Some("MyStruct"));

    let free_fn = symbols.iter().find(|s| s.name == "free_function").unwrap();
    assert!(free_fn.impl_type.is_none());
}
```

**Step 4: Run test**

Run: `cargo test --lib -- parser::tests::test_symbol_impl_type`
Expected: PASS

**Step 5: Add `target_context` to `SymbolRef`**

In `src/indexer/parser.rs`:

```rust
pub struct SymbolRef {
    pub source_name: String,
    pub source_file: String,
    pub target_name: String,
    pub kind: RefKind,
    pub target_file: Option<String>,
    /// For self.method() calls, the impl type name.
    pub target_context: Option<String>,
}
```

Update all `SymbolRef` constructors to include `target_context: None`.

**Step 6: Detect `self.method()` in `collect_body_refs`**

Update `collect_refs` to extract impl type and pass it down:

```rust
"impl_item" => {
    let impl_type = extract_impl_type(&child, source);
    collect_refs_in_impl(
        &child, source, file_path, known_symbols,
        import_map, crate_map, impl_type.as_deref(), refs,
    );
}
```

In `collect_body_refs`, add `impl_type: Option<&str>` parameter. When processing identifiers, detect the `self.method()` pattern:

Before matching an identifier, check if its parent is a `field_expression` and the sibling is `self`:

```rust
"field_identifier" => {
    let name = node_text(&child, source);
    // Check if parent is a field_expression with self receiver
    let is_self_call = child
        .parent()
        .is_some_and(|p| {
            p.kind() == "field_expression"
                && p.child(0).is_some_and(|c| c.kind() == "self")
        });
    let target_context = if is_self_call {
        impl_type.map(String::from)
    } else {
        None
    };
    if name != fn_name
        && !is_noisy_symbol(&name)
        && !locals.contains(&name)
        && known_symbols.contains(&name)
        && seen.insert(name.clone())
    {
        let target_file = import_map
            .get(&name)
            .and_then(|info| qualified_path_to_file_with_crates(
                &info.qualified_path,
                crate_map,
            ));
        refs.push(SymbolRef {
            source_name: fn_name.to_string(),
            source_file: file_path.to_string(),
            target_name: name,
            kind: RefKind::Call,
            target_file,
            target_context,
        });
    }
}
```

Add `"field_identifier"` to the existing match alongside `"type_identifier" | "identifier"`, or handle it in its own arm.

**Step 7: Write test for self.method() detection**

```rust
#[test]
fn test_refs_self_method_has_target_context() {
    let source = r#"
pub struct MyStruct;

impl MyStruct {
    pub fn caller(&self) {
        self.helper();
    }
    pub fn helper(&self) {}
}
"#;
    let mut known = std::collections::HashSet::new();
    known.insert("caller".to_string());
    known.insert("helper".to_string());

    let refs = extract_refs(source, "src/lib.rs", &known, &std::collections::HashMap::new())
        .unwrap();
    let helper_ref = refs.iter().find(|r| r.target_name == "helper").unwrap();
    assert_eq!(
        helper_ref.target_context.as_deref(),
        Some("MyStruct"),
    );
}
```

**Step 8: Run test**

Run: `cargo test --lib -- parser::tests::test_refs_self_method`
Expected: PASS

**Step 9: DB schema migration for `impl_type`**

In `src/db.rs`, add migration (similar pattern to `migrate_docs_module_column`):

```rust
fn migrate_symbols_impl_type_column(&self) -> SqlResult<()> {
    let sql: String = self.conn.query_row(
        "SELECT sql FROM sqlite_master \
         WHERE type='table' AND name='symbols'",
        [],
        |row| row.get(0),
    )?;
    if !sql.contains("impl_type") {
        self.conn.execute_batch(
            "ALTER TABLE symbols ADD COLUMN impl_type TEXT",
        )?;
    }
    Ok(())
}
```

Call it from `migrate()`. Also add `impl_type TEXT` to the CREATE TABLE statement for new DBs.

**Step 10: Update `store_symbols` in `src/indexer/store.rs`**

Add `impl_type` to the INSERT statement:

```rust
let mut sym_stmt = db.conn.prepare(
    "INSERT INTO symbols \
     (file_id, name, kind, visibility, \
      line_start, line_end, signature, \
      doc_comment, body, details, attributes, impl_type) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, \
             ?8, ?9, ?10, ?11, ?12)",
)?;
// ...
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
])?;
```

**Step 11: Add `get_symbol_id_in_impl` to DB**

```rust
/// Look up a symbol by name within an impl block for a specific type.
pub fn get_symbol_id_in_impl(
    &self,
    name: &str,
    impl_type: &str,
) -> SqlResult<Option<SymbolId>> {
    let mut stmt = self.conn.prepare(
        "SELECT id FROM symbols \
         WHERE name = ?1 AND impl_type = ?2 \
         LIMIT 1",
    )?;
    let mut rows = stmt.query(params![name, impl_type])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}
```

**Step 12: Update `store_symbol_refs` to use `target_context`**

```rust
pub fn store_symbol_refs(&self, refs: &[crate::indexer::parser::SymbolRef]) -> SqlResult<u64> {
    let mut count = 0;
    for r in refs {
        let source_id = self.get_symbol_id(&r.source_name, &r.source_file)?;
        let target_id = if let Some(ctx) = &r.target_context {
            // self.method() — try impl-qualified lookup first
            self.get_symbol_id_in_impl(&r.target_name, ctx)?
                .or(if let Some(target_file) = &r.target_file {
                    self.get_symbol_id(&r.target_name, target_file)?
                } else {
                    None
                })
                .or(self.get_symbol_id_by_name(&r.target_name)?)
        } else if let Some(target_file) = &r.target_file {
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
```

**Step 13: Write DB integration test**

```rust
#[test]
fn test_get_symbol_id_in_impl() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "method".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 5,
                line_end: 10,
                signature: "pub fn method(&self)".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("MyStruct".into()),
            },
            Symbol {
                name: "method".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 15,
                line_end: 20,
                signature: "pub fn method(&self)".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("OtherStruct".into()),
            },
        ],
    )
    .unwrap();

    let my_id = db.get_symbol_id_in_impl("method", "MyStruct").unwrap();
    let other_id = db.get_symbol_id_in_impl("method", "OtherStruct").unwrap();
    assert!(my_id.is_some());
    assert!(other_id.is_some());
    assert_ne!(my_id, other_id, "different impl types should resolve to different symbols");

    // Non-existent impl type returns None
    assert!(db.get_symbol_id_in_impl("method", "Unknown").unwrap().is_none());
}
```

**Step 14: Fix all existing tests that construct `Symbol`**

Search for all `Symbol {` constructors in tests across the codebase. Add `impl_type: None` to each. This includes:
- `src/indexer/parser.rs` tests
- `src/indexer/mod.rs` tests
- `src/server/tools/context.rs` tests
- `tests/integration.rs`
- `tests/data_integrity.rs`
- `tests/data_quality.rs`

**Step 15: Run full tests and lint**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings.

**Step 16: Commit**

```bash
git add src/indexer/parser.rs src/indexer/store.rs src/db.rs
git commit -m "feat: method-level refs via impl-block awareness

Symbol now tracks impl_type for methods inside impl blocks.
SymbolRef tracks target_context for self.method() calls.
store_symbol_refs uses get_symbol_id_in_impl for precise method
resolution when two types have methods with the same name."
```

---

## Verification Checklist

After all tasks are complete:

1. `cargo test` — all tests pass
2. `cargo clippy --all-targets --all-features -- -D warnings` — no warnings
3. `cargo fmt --all -- --check` — properly formatted
4. Verify stale refs: index a repo, modify a file, refresh, check no dangling refs
5. Verify cross-crate: in a workspace, `use other_crate::Foo` should create a ref with `target_file`
6. Verify method refs: `self.method()` inside `impl Type` should resolve to `Type::method`, not a random `method`
