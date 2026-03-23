# Evaluation Bugfixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 5 bugs discovered during the illu self-evaluation: stale index, missing "did you mean?" suggestions, enum variant leak in call graphs, boundary confidence gap, and hotspots exclude_tests gap.

**Architecture:** All fixes are independent. Each touches 1-3 files. The stale index fix is in the indexer module. The other 4 are in the db/server/tools modules. No new files needed.

**Tech Stack:** Rust, SQLite (rusqlite), tree-sitter

---

## File Map

| Fix | Files Modified |
|-----|---------------|
| 1. Stale index | `src/indexer/mod.rs` |
| 2. "Did you mean?" | `src/server/tools/mod.rs`, 12 tool handler files |
| 3. Enum variant leak | `src/db.rs` |
| 4. Boundary confidence | `src/db.rs`, `src/server/tools/boundary.rs` |
| 5. Hotspots exclude_tests | `src/db.rs`, `src/server/tools/hotspots.rs` |

---

### Task 1: Fix stale index — refresh_index misses committed changes and never updates metadata

**Files:**
- Modify: `src/indexer/mod.rs` — `refresh_index` (lines 61-124), add `committed_changed_rs_files` helper

**Context:**
- `refresh_index` uses `git_changed_rs_files` which calls `git status --porcelain` — only shows uncommitted/staged/untracked changes
- After commits with clean working tree, `git status` returns nothing → committed changes invisible
- `refresh_index` never calls `update_metadata` → stored commit hash never updates → `freshness` always STALE
- `update_metadata` (line 391-396) stores commit hash via `db.set_metadata()`
- `get_current_commit_hash` (line 696-708) reads `git rev-parse HEAD`

- [ ] **Step 1: Write failing test — committed changes detected**

In the existing test module for `src/indexer/mod.rs` (or `tests/integration.rs`), add a test that:
1. Creates a git repo with an .rs file, indexes it
2. Modifies the .rs file, commits it
3. Calls `refresh_index`
4. Verifies the changed file was re-indexed (symbol changes reflected)
5. Verifies `db.get_commit_hash()` returns the NEW commit hash

```rust
#[test]
fn test_refresh_detects_committed_changes() {
    // Setup: create temp git repo, write initial file, commit, full index
    // Then: modify file, commit again (clean working tree)
    // Then: call refresh_index
    // Assert: changed symbols are updated AND commit hash matches HEAD
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib -- indexer::tests::test_refresh_detects_committed_changes -v`
Expected: FAIL — refresh_index returns 0 (no changes detected)

- [ ] **Step 3: Implement the fix**

In `refresh_index` (src/indexer/mod.rs), add two changes:

**A) Detect committed changes by comparing stored hash to HEAD:**

After getting `existing` files and before calling `git_changed_rs_files`, add:

```rust
// Detect committed changes since last indexed commit
let stored_hash = db
    .get_commit_hash(&config.repo_path.display().to_string())?;
let current_head = get_current_commit_hash(&config.repo_path).ok();
let committed_changes = match (&stored_hash, &current_head) {
    (Some(old), Some(new)) if old != new => {
        committed_changed_rs_files(&config.repo_path, old)
    }
    _ => Vec::new(),
};
```

Add the helper function:

```rust
fn committed_changed_rs_files(repo_path: &std::path::Path, since_hash: &str) -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", &format!("{since_hash}..HEAD")])
        .current_dir(repo_path)
        .output();
    let Ok(output) = output else { return Vec::new() };
    if !output.status.success() { return Vec::new() }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| {
            std::path::Path::new(l)
                .extension()
                .is_some_and(|ext| ext == "rs")
        })
        .map(String::from)
        .collect()
}
```

Merge committed changes into the candidate list:

```rust
let mut candidate_files = git_changed_rs_files(&config.repo_path, &existing);
// Merge in committed changes not already in the list
let candidate_set: std::collections::HashSet<&str> =
    candidate_files.iter().map(String::as_str).collect();
for path in &committed_changes {
    if !candidate_set.contains(path.as_str()) {
        candidate_files.push(path.clone());
    }
}
let dirty_files = collect_dirty_files(&config.repo_path, &candidate_files, &existing);
```

**B) Always update metadata after refresh:**

At the end of `refresh_index`, before the `Ok(count)` return, add:

```rust
// Update stored commit hash so freshness reports correctly
if current_head.is_some() {
    update_metadata(db, config)?;
}
```

This must run even when `dirty_files.is_empty()` (HEAD changed but no .rs files changed).

Move the `current_head` binding to before the early return for empty dirty files, and restructure so metadata always updates when HEAD changed.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib -- indexer::tests::test_refresh_detects_committed_changes -v`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/indexer/mod.rs
git commit -m "fix: refresh_index detects committed changes and updates metadata"
```

---

### Task 2: Add "Did you mean?" suggestions on symbol not found

**Files:**
- Modify: `src/server/tools/mod.rs:68-74` — `symbol_not_found` function
- Modify: 12 tool handler files that call `symbol_not_found` (add `db` parameter)

**Context:**
- Current `symbol_not_found(name: &str) -> String` returns static text
- All 12 callers already have `db: &Database` in scope
- `db.search_symbols(name)` does FTS fuzzy search
- For `Type::method` names, the method part should be searched separately

- [ ] **Step 1: Write failing test**

Add test in `src/server/tools/mod.rs` test module (or nearby):

```rust
#[test]
fn test_symbol_not_found_suggests_similar() {
    let db = Database::open_in_memory().unwrap();
    // Insert a symbol named "resolve_symbol"
    let fid = db.insert_file("src/mod.rs", "hash1").unwrap();
    db.store_symbol(fid, &Symbol {
        name: "resolve_symbol".into(),
        kind: SymbolKind::Function,
        // ... minimal fields
    }).unwrap();

    let result = symbol_not_found(&db, "resolve_sym");
    assert!(result.contains("Did you mean"), "Should suggest: {result}");
    assert!(result.contains("resolve_symbol"), "Should contain match: {result}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib -- tools::tests::test_symbol_not_found_suggests_similar -v`
Expected: FAIL — symbol_not_found doesn't accept db parameter

- [ ] **Step 3: Implement symbol_not_found with suggestions**

In `src/server/tools/mod.rs`, change the function:

```rust
pub(crate) fn symbol_not_found(db: &Database, name: &str) -> String {
    let mut suggestions = Vec::new();

    // Try FTS search on the full name
    if let Ok(results) = db.search_symbols(name) {
        suggestions.extend(results);
    }

    // For Type::method names, also try searching the method part alone
    if suggestions.is_empty() {
        if let Some((_, method)) = name.split_once("::") {
            if let Ok(results) = db.search_symbols(method) {
                suggestions.extend(results);
            }
        }
    }

    // Deduplicate by (name, impl_type), take top 3
    let mut seen = std::collections::HashSet::new();
    let suggestions: Vec<_> = suggestions
        .into_iter()
        .filter(|s| {
            let key = format!(
                "{}::{}",
                s.impl_type.as_deref().unwrap_or(""),
                s.name
            );
            seen.insert(key)
        })
        .take(3)
        .collect();

    if suggestions.is_empty() {
        format!(
            "No symbol found matching '{name}'.\n\
             Try `Type::method` syntax for methods \
             (e.g. `Database::new`), or use `query` to search."
        )
    } else {
        let mut out = format!("No symbol found matching '{name}'.\n\nDid you mean:\n");
        for s in &suggestions {
            let qname = qualified_name(s);
            let _ = writeln!(out, "- `{qname}` ({} at {}:{})", s.kind, s.file_path, s.line_start);
        }
        let _ = write!(out, "\nUse `query` to search more broadly.");
        out
    }
}
```

- [ ] **Step 4: Update all 12 callers**

Each caller changes from `super::symbol_not_found(symbol_name)` to `super::symbol_not_found(db, symbol_name)`. The files:

1. `src/server/tools/blame.rs` — `handle_blame`
2. `src/server/tools/callpath.rs` — `handle_callpath` (2 calls: `from` and `to`)
3. `src/server/tools/context.rs` — `handle_context`
4. `src/server/tools/crate_impact.rs` — `handle_crate_impact`
5. `src/server/tools/graph_export.rs` — `export_symbol_graph`
6. `src/server/tools/history.rs` — `handle_history`
7. `src/server/tools/impact.rs` — `handle_impact`
8. `src/server/tools/neighborhood.rs` — `handle_neighborhood`
9. `src/server/tools/references.rs` — `handle_references`
10. `src/server/tools/rename_plan.rs` — `handle_rename_plan`
11. `src/server/tools/similar.rs` — `handle_similar`
12. `src/server/tools/test_impact.rs` — `handle_test_impact`

Pattern: `return Ok(super::symbol_not_found(symbol_name));` → `return Ok(super::symbol_not_found(db, symbol_name));`

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: All pass. Existing "not found" tests still pass (empty DB → no suggestions → same message format).

- [ ] **Step 6: Commit**

```bash
git add src/server/tools/
git commit -m "feat: add 'did you mean?' suggestions when symbol not found"
```

---

### Task 3: Fix enum variant leak in neighborhood call graphs

**Files:**
- Modify: `src/db.rs` — `get_callees_by_name` (line 1562), `get_callers_by_name` (line 1596)

**Context:**
- Both queries filter `ts.kind NOT IN ('const', 'static')` but `'enum_variant'` is missing
- Enum variant comparisons (e.g., `SymbolKind::Impl`) appear in call graphs as function calls
- The fix is adding `'enum_variant'` to the NOT IN clause in all 4 SQL strings (2 per function × 2 functions)

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_callees_by_name_excludes_enum_variants() {
    let db = Database::open_in_memory().unwrap();
    // Insert a function and an enum variant, create a ref between them
    // Call get_callees_by_name
    // Assert the enum variant is NOT in the results
}
```

- [ ] **Step 2: Run test to verify it fails**

Expected: FAIL — enum variant appears in results

- [ ] **Step 3: Implement the fix**

In `get_callees_by_name` (db.rs:1562), change both SQL strings:
```sql
AND ts.kind NOT IN ('const', 'static')
```
to:
```sql
AND ts.kind NOT IN ('const', 'static', 'enum_variant')
```

In `get_callers_by_name` (db.rs:1596), change both SQL strings:
```sql
AND ss.kind NOT IN ('const', 'static')
```
to:
```sql
AND ss.kind NOT IN ('const', 'static', 'enum_variant')
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "fix: exclude enum variants from call graph traversal"
```

---

### Task 4: Fix boundary confidence gap — add min_confidence to get_callers

**Files:**
- Modify: `src/db.rs` — `get_callers` (line 1519) — add `min_confidence: Option<&str>` parameter
- Modify: `src/server/tools/boundary.rs` — pass `None` for min_confidence
- Modify: `src/server/tools/context.rs` — pass `Some("high")` for min_confidence
- Modify: `src/server/tools/references.rs` — pass `Some("high")` for min_confidence

**Context:**
- `get_callers` hard-codes `sr.confidence = 'high'` in SQL
- Tool handler calls from `server/mod.rs` via paths like `tools::context::handle_context` are low-confidence refs
- Boundary tool uses `get_callers` to detect external usage → misclassifies 31/36 tool handlers as "Internal Only"
- Fix: parameterize confidence, let boundary use all confidences (inclusive = correct for boundary check)

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_boundary_detects_low_confidence_external_callers() {
    let db = Database::open_in_memory().unwrap();
    // Insert a function in src/server/tools/context.rs
    // Insert a caller in src/server/mod.rs with LOW confidence ref
    // Call handle_boundary for "src/server/tools/"
    // Assert the function appears in Public API, not Internal Only
}
```

- [ ] **Step 2: Run test to verify it fails**

Expected: FAIL — function shows as Internal Only

- [ ] **Step 3: Add min_confidence parameter to get_callers**

In `src/db.rs`, change `get_callers` signature:

```rust
pub fn get_callers(
    &self,
    symbol_name: &str,
    target_file: &str,
    exclude_tests: bool,
    min_confidence: Option<&str>,
) -> SqlResult<Vec<CalleeInfo>> {
```

Change both SQL strings from:
```sql
AND sr.confidence = 'high'
```
to:
```sql
AND (?3 IS NULL OR sr.confidence = ?3)
```

Update the `query` call to include the new parameter:
```rust
let mut rows = stmt.query(params![symbol_name, target_file, min_confidence])?;
```

- [ ] **Step 4: Update callers**

1. `src/server/tools/boundary.rs` (`handle_boundary` line ~30):
   ```rust
   // WAS: let callers = db.get_callers(&sym.name, &sym.file_path, false)?;
   let callers = db.get_callers(&sym.name, &sym.file_path, false, None)?;
   ```

2. `src/server/tools/context.rs` (`render_callers` line ~230):
   ```rust
   // WAS: for c in db.get_callers(&sym.name, &sym.file_path, exclude_tests)? {
   for c in db.get_callers(&sym.name, &sym.file_path, exclude_tests, Some("high"))? {
   ```

3. `src/server/tools/references.rs` (`render_call_sites` line ~65):
   ```rust
   // WAS: for c in db.get_callers(&sym.name, &sym.file_path, false)? {
   for c in db.get_callers(&sym.name, &sym.file_path, false, Some("high"))? {
   ```

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: All pass

- [ ] **Step 6: Commit**

```bash
git add src/db.rs src/server/tools/boundary.rs src/server/tools/context.rs src/server/tools/references.rs
git commit -m "fix: boundary tool uses all confidence levels for external caller detection"
```

---

### Task 5: Fix hotspots exclude_tests on Most Referencing section

**Files:**
- Modify: `src/db.rs` — `get_most_referencing_symbols` (line 1859) — add `exclude_tests: bool` parameter
- Modify: `src/server/tools/hotspots.rs` — pass `exclude_tests` through

**Context:**
- `get_most_referencing_symbols` has no `exclude_tests` parameter
- When `exclude_tests=true`, "Most Referenced" and "Largest Functions" filter tests but "Most Referencing" doesn't
- Fix: add `exclude_tests` param, filter `ss.is_test = 0` when true (exclude test functions as ref sources)
- Pattern matches `get_most_referenced_symbols_filtered` which already has this

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_most_referencing_excludes_tests() {
    let db = Database::open_in_memory().unwrap();
    // Insert a test function with many outgoing refs
    // Insert a production function with fewer outgoing refs
    // Call get_most_referencing_symbols with exclude_tests=true
    // Assert test function is NOT in results
}
```

- [ ] **Step 2: Run test to verify it fails**

Expected: FAIL — test function appears in results

- [ ] **Step 3: Implement the fix**

In `src/db.rs`, change `get_most_referencing_symbols`:

```rust
pub fn get_most_referencing_symbols(
    &self,
    limit: i64,
    path_prefix: &str,
    min_confidence: Option<&str>,
    exclude_tests: bool,
) -> SqlResult<Vec<(String, String, i64)>> {
```

Add the `exclude_tests` branch to SQL (same pattern as `get_largest_functions`):

```rust
let sql = if exclude_tests {
    "SELECT ss.name, f.path, \
            COUNT(DISTINCT sr.target_symbol_id) as ref_count \
     FROM symbol_refs sr \
     JOIN symbols ss ON ss.id = sr.source_symbol_id \
     JOIN files f ON f.id = ss.file_id \
     WHERE f.path LIKE ?1 ESCAPE '\\' \
       AND (?3 IS NULL OR sr.confidence = ?3) \
       AND ss.is_test = 0 \
     GROUP BY ss.id \
     ORDER BY ref_count DESC \
     LIMIT ?2"
} else {
    // existing SQL unchanged
};
```

In `src/server/tools/hotspots.rs`, update the call:

```rust
// WAS: let most_referencing = db.get_most_referencing_symbols(max, prefix, Some("high"))?;
let most_referencing = db.get_most_referencing_symbols(max, prefix, Some("high"), exclude_tests)?;
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/db.rs src/server/tools/hotspots.rs
git commit -m "fix: hotspots exclude_tests applies to Most Referencing section"
```

---

### Task 6: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt --all -- --check`
Expected: No formatting issues
