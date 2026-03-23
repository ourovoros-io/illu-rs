# Ref Extraction & Test Detection Fixes

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix 4 bugs in illu's ref extraction and test detection that cause false positives in `unused`, incomplete callee lists, and noise in `test_impact`.

**Architecture:** Three independent root causes — (A) crate name hyphen/underscore mismatch breaks all cross-module import resolution, (B) `is_test` uses substring matching that catches non-test attributes, (C) `NOISY_SYMBOL_NAMES` blocks qualified calls like `illu_rs::status::clear()`. All fixes are in `src/indexer/parser.rs`, `src/indexer/mod.rs`, and `src/indexer/store.rs`.

**Tech Stack:** Rust, tree-sitter, rusqlite

---

## Root Cause Analysis

### Bug A: Crate name normalization (`illu-rs` vs `illu_rs`)

**Files:** `src/indexer/mod.rs:182-183`, `src/indexer/parser.rs:808-842`

`extract_package_name` returns the raw Cargo.toml name (`illu-rs` with hyphen). This goes into `crate_map` as `{"illu-rs": "."}`. But Rust import paths use `illu_rs` (underscore). When `qualified_path_to_files_with_crates` looks up `crate_map.get("illu_rs")`, it gets `None`.

**Impact:** ALL import-resolved paths from `main.rs` fail. `resolve_target_file` falls back to `known_symbols.contains(name) → Some(current_file)`, producing wrong `target_file`. `SymbolIdMap::resolve` then falls to name-only → `confidence = "low"`. Since `get_callees` filters `confidence = 'high'`, all cross-module refs from `main.rs` are invisible in callee lists.

**Symptoms:**
- Bug 1: `IlluServer::new` appears "unused" (ref may exist but callee visibility is broken)
- Bug 2: `illu_rs::status::clear()` appears "unused"
- Bug 4: `main`'s callee list shows only same-file symbols

**Fix:** Normalize crate names in `crate_map` by replacing hyphens with underscores. This is what Rust itself does (Cargo normalizes `foo-bar` → `foo_bar` for module paths).

### Bug B: `is_test` substring matching

**Files:** `src/indexer/store.rs:49-52`, `src/db.rs:404-414`

```rust
// store.rs:52 — index time
let is_test = sym.attributes.as_deref().is_some_and(|a| a.contains("test"));

// db.rs:414 — migration
"UPDATE symbols SET is_test = 1 WHERE attributes LIKE '%test%'"
```

Both use substring matching. The attribute `tool(name = "test_impact", ...)` contains "test" → the `IlluServer::test_impact` handler gets `is_test = 1`.

**Symptoms:**
- Bug 3: Non-test `#[tool]` handlers appear in `test_impact` results

**Fix:** Match `test` as a complete attribute token, not a substring. Check for `== "test"` or starts-with `"test("` or `"tokio::test"` etc. — basically, the attribute must BE `test` or be a test macro like `tokio::test`, `rstest`, etc.

### Bug C: `NOISY_SYMBOL_NAMES` blocks qualified calls

**File:** `src/indexer/parser.rs:708-769`, `src/indexer/parser.rs:1375-1399`

`clear` is in `NOISY_SYMBOL_NAMES` (line 739). When `illu_rs::status::clear()` falls through to `handle_scoped_call` (because `handle_crate_path` only handles `crate::` prefix), `try_add("clear", ...)` is called. But `try_add` checks `!is_noisy_symbol(name)` and rejects it.

This is partially fixed by Bug A (crate name normalization will let `handle_crate_path` succeed for `illu_rs::` paths via crate_map). But the underlying issue remains: `try_add` filters noisy names even when they have a type/module context that makes them unambiguous.

**Fix:** In `try_add`, skip the noisy filter when `target_context` is provided (qualified calls are unambiguous). This mirrors how `try_add_qualified` already bypasses the noisy filter.

---

## Task 1: Fix crate name normalization

**Files:**
- Modify: `src/indexer/mod.rs:136-139` (crate_map construction)
- Modify: `src/indexer/mod.rs:339-342` (same pattern in refresh path)
- Test: `src/indexer/mod.rs` (existing test module)

### Step 1: Write failing test

Add to the test module in `src/indexer/mod.rs`:

```rust
#[test]
#[expect(clippy::unwrap_used, reason = "tests")]
fn test_crate_map_normalizes_hyphens() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    // Create a crate with hyphenated name
    std::fs::create_dir_all(repo.join("src/status")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2024"
"#,
    )
    .unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub mod status;\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("src/status.rs"),
        "pub fn clear() {}\n",
    )
    .unwrap();
    // main.rs imports via underscore crate name
    std::fs::write(
        repo.join("src/main.rs"),
        r#"use my_crate::status::clear;
fn main() { clear(); }
"#,
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: repo.to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // The ref from main→clear should exist
    let callers = db.get_callers_by_name("clear", None, false).unwrap();
    let has_main_caller = callers.iter().any(|(name, _)| name == "main");
    assert!(
        has_main_caller,
        "main should be a caller of clear (via my_crate:: import), got: {callers:?}"
    );
}
```

### Step 2: Run test to verify it fails

Run: `cargo test --lib -- test_crate_map_normalizes_hyphens`
Expected: FAIL — main is not listed as caller of clear

### Step 3: Fix crate_map construction

In `src/indexer/mod.rs`, find both `crate_map` construction sites (lines ~136 and ~339) and normalize the name:

```rust
let crate_map: std::collections::HashMap<String, String> = all_crates
    .iter()
    .map(|c| (c.name.replace('-', "_"), c.path.clone()))
    .collect();
```

Change `.map(|c| (c.name.clone(), c.path.clone()))` to `.map(|c| (c.name.replace('-', "_"), c.path.clone()))` in BOTH locations.

### Step 4: Run test to verify it passes

Run: `cargo test --lib -- test_crate_map_normalizes_hyphens`
Expected: PASS

### Step 5: Run full test suite

Run: `cargo test`
Expected: All pass

### Step 6: Commit

```
fix: normalize crate name hyphens to underscores in crate_map

Cargo normalizes `foo-bar` to `foo_bar` for Rust module paths, but
crate_map used the raw Cargo.toml name. This broke all cross-module
ref resolution for crates with hyphens (e.g. `illu-rs`).
```

---

## Task 2: Fix `is_test` detection

**Files:**
- Modify: `src/indexer/store.rs:49-52`
- Modify: `src/db.rs:404-414`
- Test: `src/indexer/store.rs` (existing test module)

### Step 1: Write failing test

Add to the test module in `src/indexer/store.rs`:

```rust
#[test]
#[expect(clippy::unwrap_used, reason = "tests")]
fn test_is_test_excludes_tool_with_test_in_name() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/server.rs", "abc123").unwrap();

    let test_fn = ParsedSymbol {
        name: "test_something".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Private,
        line_start: 1,
        line_end: 5,
        signature: Some("fn test_something()".to_string()),
        doc_comment: None,
        body: None,
        details: None,
        attributes: Some("test".to_string()),
        impl_type: None,
    };

    let tool_fn = ParsedSymbol {
        name: "test_impact".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Private,
        line_start: 10,
        line_end: 20,
        signature: Some("async fn test_impact()".to_string()),
        doc_comment: None,
        body: None,
        details: None,
        attributes: Some(
            "tool(name = \"test_impact\", description = \"Show which tests break\")".to_string(),
        ),
        impl_type: Some("IlluServer".to_string()),
    };

    store_symbols(&db, file_id, &[test_fn, tool_fn]).unwrap();

    // test_something should be is_test = 1
    let tests: Vec<String> = db
        .conn
        .prepare("SELECT name FROM symbols WHERE is_test = 1")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(tests.contains(&"test_something".to_string()), "test fn should be is_test=1");
    assert!(
        !tests.contains(&"test_impact".to_string()),
        "tool handler should NOT be is_test=1, got: {tests:?}"
    );
}
```

Note: you may need to add `use` imports for `ParsedSymbol`, `SymbolKind`, `Visibility` and expose `store_symbols` if it's not already `pub`. Adjust imports as needed.

### Step 2: Run test to verify it fails

Run: `cargo test --lib -- test_is_test_excludes_tool_with_test_in_name`
Expected: FAIL — tool_fn has is_test=1

### Step 3: Fix is_test detection in store.rs

Replace line 49-52 in `src/indexer/store.rs`:

```rust
// Before:
let is_test = sym
    .attributes
    .as_deref()
    .is_some_and(|a| a.contains("test"));

// After:
let is_test = sym
    .attributes
    .as_deref()
    .is_some_and(is_test_attribute);
```

Add helper function (at module level or nearby):

```rust
/// Check if an attributes string indicates a test function.
/// Matches: `test`, `tokio::test`, `rstest`, `test_case`, etc.
/// Rejects: `tool(name = "test_impact", ...)` where "test" is just a substring.
fn is_test_attribute(attrs: &str) -> bool {
    attrs.split(", ").any(|attr| {
        let attr = attr.trim();
        attr == "test"
            || attr.ends_with("::test")
            || attr.starts_with("test(")
            || attr == "rstest"
            || attr.starts_with("rstest(")
            || attr.starts_with("test_case(")
    })
}
```

### Step 4: Fix migration in db.rs

Replace line 414 in `src/db.rs`:

```rust
// Before:
"UPDATE symbols SET is_test = 1 WHERE attributes LIKE '%test%'"

// After — match 'test' as standalone attribute or test macro
"UPDATE symbols SET is_test = 1 \
 WHERE attributes = 'test' \
    OR attributes LIKE 'test,%' ESCAPE '\\' \
    OR attributes LIKE '%, test%' ESCAPE '\\' \
    OR attributes LIKE '%::test%' ESCAPE '\\' \
    OR attributes LIKE 'rstest%' ESCAPE '\\' \
    OR attributes LIKE '%, rstest%' ESCAPE '\\'"
```

Alternatively, for simpler SQL that's still correct: just reset all `is_test` during migration and re-derive from attributes using the Rust helper. Since the migration only runs once (on schema upgrade), it's fine to scan all rows:

```rust
fn migrate_symbols_is_test_column(&self) -> SqlResult<()> {
    let has_is_test = self
        .conn
        .prepare("SELECT is_test FROM symbols LIMIT 0")
        .is_ok();
    if !has_is_test {
        self.conn.execute_batch(
            "ALTER TABLE symbols ADD COLUMN is_test INTEGER NOT NULL DEFAULT 0",
        )?;
        // Use precise matching — 'test' as standalone attribute or test macro
        self.conn.execute_batch(
            "UPDATE symbols SET is_test = 1 \
             WHERE attributes = 'test' \
                OR attributes LIKE 'test, %' \
                OR attributes LIKE '%, test' \
                OR attributes LIKE '%, test, %' \
                OR attributes LIKE '%::test' \
                OR attributes LIKE '%::test, %' \
                OR attributes LIKE '%::test(%' \
                OR attributes LIKE 'rstest%' \
                OR attributes LIKE '%, rstest%'"
        )?;
    }
    Ok(())
}
```

### Step 5: Run test to verify it passes

Run: `cargo test --lib -- test_is_test_excludes_tool_with_test_in_name`
Expected: PASS

### Step 6: Run full test suite

Run: `cargo test`
Expected: All pass

### Step 7: Commit

```
fix: use precise is_test matching instead of substring contains

The old `attributes.contains("test")` matched tool handlers like
`tool(name = "test_impact", ...)`, making them appear as test
functions in test_impact results.
```

---

## Task 3: Skip noisy filter for qualified calls

**Files:**
- Modify: `src/indexer/parser.rs:1375-1399` (`try_add` method)
- Test: `src/indexer/parser.rs` (existing test module)

### Step 1: Write failing test

Add to the test module in `src/indexer/parser.rs`:

```rust
#[test]
#[expect(clippy::unwrap_used, reason = "tests")]
fn test_qualified_call_bypasses_noisy_filter() {
    // `clear` is in NOISY_SYMBOL_NAMES, but `Status::clear()` should still
    // be captured because the type qualification makes it unambiguous.
    let source = r#"
pub struct Status;
impl Status {
    pub fn clear(&self) {}
}
fn caller() {
    let s = Status;
    Status::clear(&s);
}
"#;
    let known = ["Status", "clear", "caller"]
        .into_iter()
        .map(String::from)
        .collect::<std::collections::HashSet<_>>();
    let crate_map = std::collections::HashMap::<String, String>::new();
    let refs = extract_refs(source, "src/status.rs", &known, &crate_map).unwrap();
    let has_clear_ref = refs
        .iter()
        .any(|r| r.source_name == "caller" && r.target_name == "clear");
    assert!(
        has_clear_ref,
        "qualified call Status::clear() should bypass noisy filter, got: {refs:?}"
    );
}
```

### Step 2: Run test to verify it fails

Run: `cargo test --lib -- test_qualified_call_bypasses_noisy_filter`
Expected: FAIL — `clear` is filtered by `is_noisy_symbol`

### Step 3: Fix try_add to skip noisy filter for qualified calls

In `src/indexer/parser.rs`, modify `try_add` (around line 1383):

```rust
fn try_add(
    &mut self,
    name: &str,
    kind: RefKind,
    target_context: Option<String>,
    line: Option<i64>,
    refs: &mut Vec<SymbolRef>,
) {
    // Qualified calls (with target_context) bypass the noisy filter —
    // `Status::clear()` is unambiguous unlike a bare `clear()`.
    let noisy = target_context.is_none() && is_noisy_symbol(name);
    if name != self.fn_name
        && !noisy
        && !self.locals.contains(name)
        && self.ctx.known_symbols.contains(name)
        && self.seen.insert(name.to_string())
    {
        refs.push(SymbolRef {
            source_name: self.fn_name.to_string(),
            source_file: self.ctx.file_path.to_string(),
            target_name: name.to_string(),
            kind,
            target_file: resolve_target_file(name, self.ctx),
            target_context,
            ref_line: line,
        });
    }
}
```

### Step 4: Run test to verify it passes

Run: `cargo test --lib -- test_qualified_call_bypasses_noisy_filter`
Expected: PASS

### Step 5: Run full test suite

Run: `cargo test`
Expected: All pass (check for no regressions in ref counts)

### Step 6: Commit

```
fix: skip noisy symbol filter for qualified calls

Bare `clear()` is rightfully noisy, but `Status::clear()` is
unambiguous. Skip the noisy filter when target_context is present,
matching how try_add_qualified already works for crate:: paths.
```

---

## Task 4: Fix `seen` dedup to include target_context

**Files:**
- Modify: `src/indexer/parser.rs:1387` (`seen.insert` in `try_add`)
- Test: `src/indexer/parser.rs` (existing test module)

The `seen` set deduplicates by bare name. This means `Foo::new()` and `Bar::new()` in the same function body — only the first is captured. The fix is to include target_context in the dedup key.

### Step 1: Write failing test

```rust
#[test]
#[expect(clippy::unwrap_used, reason = "tests")]
fn test_seen_dedup_includes_context() {
    // Two different Type::new() calls in the same function should both be captured
    let source = r#"
pub struct Foo;
impl Foo { pub fn new() -> Self { Self } }
pub struct Bar;
impl Bar { pub fn new() -> Self { Self } }
fn caller() {
    let _ = Foo::new();
    let _ = Bar::new();
}
"#;
    let known = ["Foo", "Bar", "new", "caller"]
        .into_iter()
        .map(String::from)
        .collect::<std::collections::HashSet<_>>();
    let crate_map = std::collections::HashMap::<String, String>::new();
    let refs = extract_refs(source, "src/lib.rs", &known, &crate_map).unwrap();
    let new_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.source_name == "caller" && r.target_name == "new")
        .collect();
    assert_eq!(
        new_refs.len(),
        2,
        "Both Foo::new() and Bar::new() should be captured, got: {new_refs:?}"
    );
}
```

### Step 2: Run test to verify it fails

Run: `cargo test --lib -- test_seen_dedup_includes_context`
Expected: FAIL — only 1 ref captured

### Step 3: Fix seen dedup key

In `src/indexer/parser.rs`, modify the `seen.insert` in `try_add` (line ~1387):

```rust
// Before:
&& self.seen.insert(name.to_string())

// After — include target_context in dedup key so Foo::new and Bar::new are distinct:
&& self.seen.insert(match &target_context {
    Some(ctx) => format!("{ctx}::{name}"),
    None => name.to_string(),
})
```

### Step 4: Run test to verify it passes

Run: `cargo test --lib -- test_seen_dedup_includes_context`
Expected: PASS

### Step 5: Run full test suite

Run: `cargo test`
Expected: All pass

### Step 6: Commit

```
fix: include target_context in seen dedup key

Foo::new() and Bar::new() in the same function were deduplicated
because seen only tracked the bare name "new". Now deduplicates
by "Foo::new" vs "Bar::new".
```

---

## Task 5: Integration test — verify all bugs are fixed end-to-end

**Files:**
- Test: `src/indexer/mod.rs` (existing test module)

### Step 1: Write integration test

```rust
#[test]
#[expect(clippy::unwrap_used, reason = "tests")]
fn test_cross_module_refs_with_hyphenated_crate() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    std::fs::create_dir_all(repo.join("src/server")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        r#"[package]
name = "my-app"
version = "0.1.0"
edition = "2024"
"#,
    )
    .unwrap();

    std::fs::write(
        repo.join("src/lib.rs"),
        "pub mod server;\npub mod status;\n",
    )
    .unwrap();

    std::fs::write(
        repo.join("src/status.rs"),
        "pub fn clear() {}\npub fn set(_msg: &str) {}\n",
    )
    .unwrap();

    std::fs::write(
        repo.join("src/server.rs"),
        r#"pub struct MyServer;
impl MyServer {
    pub fn new() -> Self { Self }
}
"#,
    )
    .unwrap();

    std::fs::write(
        repo.join("src/main.rs"),
        r#"use my_app::server::MyServer;
use my_app::status;

fn main() {
    let _s = MyServer::new();
    status::clear();
    status::set("ready");
}
"#,
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: repo.to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Bug A: MyServer::new should have main as caller
    let new_callers = db.get_callers_by_name("new", None, false).unwrap();
    let main_calls_new = new_callers.iter().any(|(name, _)| name == "main");
    assert!(main_calls_new, "main should call MyServer::new, callers: {new_callers:?}");

    // Bug C: status::clear should have main as caller (clear is in NOISY_SYMBOL_NAMES)
    let clear_callers = db.get_callers_by_name("clear", None, false).unwrap();
    let main_calls_clear = clear_callers.iter().any(|(name, _)| name == "main");
    assert!(main_calls_clear, "main should call status::clear, callers: {clear_callers:?}");

    // Bug A: main's callees should include cross-module symbols (high confidence)
    let main_callees = db.get_callees("main", "src/main.rs", false).unwrap();
    let callee_names: Vec<&str> = main_callees.iter().map(|c| c.name.as_str()).collect();
    assert!(
        callee_names.contains(&"new"),
        "main callees should include 'new', got: {callee_names:?}"
    );
    assert!(
        callee_names.contains(&"clear"),
        "main callees should include 'clear', got: {callee_names:?}"
    );

    // Verify unreferenced doesn't include MyServer::new or clear
    let unused = db.get_unreferenced_symbols(None, true).unwrap();
    let unused_names: Vec<String> = unused
        .iter()
        .map(|s| {
            if let Some(it) = &s.impl_type {
                format!("{}::{}", it, s.name)
            } else {
                s.name.clone()
            }
        })
        .collect();
    assert!(
        !unused_names.contains(&"MyServer::new".to_string()),
        "MyServer::new should NOT be unused, unused: {unused_names:?}"
    );
    assert!(
        !unused_names.contains(&"clear".to_string()),
        "clear should NOT be unused, unused: {unused_names:?}"
    );
}
```

### Step 2: Run test

Run: `cargo test --lib -- test_cross_module_refs_with_hyphenated_crate`
Expected: PASS (all 4 assertions)

### Step 3: Write is_test integration test

```rust
#[test]
#[expect(clippy::unwrap_used, reason = "tests")]
fn test_is_test_precise_matching() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"test-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();

    std::fs::write(
        repo.join("src/lib.rs"),
        r#"
pub fn test_impact() {}

#[test]
fn test_real() {}

pub fn not_a_test() {}
"#,
    )
    .unwrap();
    std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig { repo_path: repo.to_path_buf() };
    index_repo(&db, &config).unwrap();

    let tests: Vec<String> = db
        .conn
        .prepare("SELECT name FROM symbols WHERE is_test = 1")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert!(tests.contains(&"test_real".to_string()), "test_real should be is_test=1");
    assert!(
        !tests.contains(&"test_impact".to_string()),
        "test_impact (no #[test] attr) should NOT be is_test=1, got: {tests:?}"
    );
}
```

### Step 4: Run test

Run: `cargo test --lib -- test_is_test_precise_matching`
Expected: PASS

### Step 5: Lint and format

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all -- --check`
Expected: Clean

### Step 6: Commit

```
test: add integration tests for ref extraction and is_test fixes
```

---

## Task 6: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

Add/update these entries in the Key Patterns section:

```markdown
- **Crate name normalization** — `crate_map` replaces hyphens with underscores in crate names (Cargo normalizes `foo-bar` → `foo_bar` for module paths). Both `extract_all_symbol_refs` and `rebuild_refs_for_files` apply this.
- **is_test precision** — `is_test` column uses `is_test_attribute()` helper that matches `test`, `tokio::test`, `rstest`, `test_case` as complete attribute tokens. Rejects `tool(name = "test_impact", ...)` where "test" is only a substring.
- **Qualified calls bypass noisy filter** — `try_add` skips `is_noisy_symbol` check when `target_context` is present. `Status::clear()` is unambiguous even though bare `clear()` is noisy.
- **Seen dedup includes context** — `BodyRefCollector.seen` uses `"Type::name"` as key when target_context is present, so `Foo::new()` and `Bar::new()` in the same function are both captured.
```

### Step 1: Make the edits

Update the Key Patterns section in `CLAUDE.md` with the entries above. Remove or update any contradicting entries.

### Step 2: Commit

```
docs: update CLAUDE.md with ref extraction fix patterns
```

---

## Execution Order

Tasks 1-4 are independent and can be parallelized. Task 5 depends on all of 1-4. Task 6 depends on 5.

```
Task 1 (crate name)  ─┐
Task 2 (is_test)      ├─→ Task 5 (integration) → Task 6 (docs)
Task 3 (noisy filter) ─┤
Task 4 (seen dedup)   ─┘
```

## Diagnostic Note

If Bug 1 (`IlluServer::new` unused) persists after fixing Tasks 1+3+4, add a debug test that calls `extract_refs` on main.rs-style code and prints the raw `Vec<SymbolRef>` to inspect whether the ref is extracted but fails at storage, or never extracted at all. The tree-sitter AST for `#[tokio::main] async fn main()` may need verification.
