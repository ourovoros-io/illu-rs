# Evaluation Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the top 5 issues and add 2 capabilities identified in the 2026-03-21 illu self-evaluation.

**Architecture:** Changes span the full stack — schema migration, parser ref extraction, DB queries, tool handlers, MCP param structs, and display code. Each task is independent after Task 1 (ref_line), which other tasks don't depend on.

**Tech Stack:** Rust, SQLite, tree-sitter, rmcp

---

## Task 1: Add ref_line to symbol_refs (Issue #1 — caller lines)

**Problem:** Caller/callee lines show the calling *function's definition* line, not the line where the call actually happens. This forces an extra Read+search step every time.

**Approach:** Add a `ref_line` column to `symbol_refs`, capture the tree-sitter node's line in the parser, store it, and prefer it in display code. Keep `insert_symbol_ref` backward-compatible (existing tests pass NULL for ref_line). Bump SCHEMA_VERSION to force re-index.

**Files:**
- Modify: `src/indexer/parser.rs` (SymbolRef struct, try_add, collect_body_refs, collect_derive_refs)
- Modify: `src/db.rs` (schema, insert_symbol_ref, store_symbol_refs_fast, get_callers, get_callees, CalleeInfo, SCHEMA_VERSION)
- Modify: `src/server/tools/context.rs` (render_callers, render_callees)
- Modify: `src/server/tools/neighborhood.rs` (format_list_output, bfs_collect)
- Modify: `src/server/tools/references.rs` (handle_references)
- Modify: `src/server/tools/rename_plan.rs` (write_call_sites)
- Modify: `src/server/tools/boundary.rs` (handle_boundary)
- Test: existing tests + new tests

### Step 1: Add ref_line field to SymbolRef struct

In `src/indexer/parser.rs`, add `ref_line` to `SymbolRef`:

```rust
pub struct SymbolRef {
    pub source_name: String,
    pub source_file: String,
    pub target_name: String,
    pub kind: RefKind,
    pub target_file: Option<String>,
    pub target_context: Option<String>,
    pub ref_line: Option<i64>,  // Line where the reference occurs (1-based)
}
```

### Step 2: Update try_add to accept and store line number

In `src/indexer/parser.rs` `BodyRefCollector::try_add` (line 1351), add `line: Option<i64>` parameter:

```rust
fn try_add(
    &mut self,
    name: &str,
    kind: RefKind,
    target_context: Option<String>,
    line: Option<i64>,
    refs: &mut Vec<SymbolRef>,
) {
    if name != self.fn_name
        && !is_noisy_symbol(name)
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

### Step 3: Pass line numbers in collect_body_refs

In `src/indexer/parser.rs` `collect_body_refs` (line 1376), every call to `col.try_add` should pass the node's line:

For `type_identifier` / `identifier` cases:
```rust
let line = Some(child.start_position().row as i64 + 1);
col.try_add(&name, ref_kind, None, line, refs);
```

For `field_identifier` case:
```rust
let line = Some(child.start_position().row as i64 + 1);
col.try_add(&name, RefKind::Call, target_context, line, refs);
```

For identifiers inside `macro_invocation`:
```rust
let line = Some(mchild.start_position().row as i64 + 1);
col.try_add(&name, ref_kind, None, line, refs);
```

### Step 4: Pass line numbers in collect_derive_refs

In `src/indexer/parser.rs` `collect_derive_refs`, add `ref_line` to the `SymbolRef` pushed:

```rust
refs.push(SymbolRef {
    // ... existing fields ...
    ref_line: Some(attr_node.start_position().row as i64 + 1),
});
```

### Step 5: Add ref_line column to schema and migration

In `src/db.rs`, in `migrate()` (line 139), the `symbol_refs` CREATE TABLE already exists. Add a new migration method `migrate_symbol_refs_ref_line_column` (similar pattern to existing migrations):

```rust
fn migrate_symbol_refs_ref_line_column(&self) -> SqlResult<()> {
    let has_col: bool = self
        .conn
        .prepare("SELECT ref_line FROM symbol_refs LIMIT 0")
        .is_ok();
    if !has_col {
        self.conn
            .execute_batch("ALTER TABLE symbol_refs ADD COLUMN ref_line INTEGER")?;
    }
    Ok(())
}
```

Call it from `migrate()` before `check_schema_version()`.

Also update the CREATE TABLE in `migrate()` to include `ref_line INTEGER` for fresh databases.

### Step 6: Bump SCHEMA_VERSION

In `src/db.rs` line 247, change:
```rust
const SCHEMA_VERSION: &str = "5";
```

This forces a full re-index so all refs get ref_line populated.

### Step 7: Update insert_symbol_ref to accept ref_line

In `src/db.rs` `insert_symbol_ref` (line 558):

```rust
pub fn insert_symbol_ref(
    &self,
    source_id: SymbolId,
    target_id: SymbolId,
    kind: &str,
    confidence: &str,
    ref_line: Option<i64>,
) -> SqlResult<()> {
    self.conn.execute(
        "INSERT OR IGNORE INTO symbol_refs \
         (source_symbol_id, target_symbol_id, kind, confidence, ref_line) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![source_id, target_id, kind, confidence, ref_line],
    )?;
    Ok(())
}
```

### Step 8: Update store_symbol_refs_fast to pass ref_line

In `src/db.rs` `store_symbol_refs_fast` (line 1572):

```rust
self.insert_symbol_ref(sid, tid, &r.kind.to_string(), confidence, r.ref_line)?;
```

### Step 9: Update all existing insert_symbol_ref call sites in tests

Every test that calls `db.insert_symbol_ref(...)` needs `, None` appended as the last argument. This is ~35 call sites across test files. Mechanical change — add `, None` to each call.

### Step 10: Add ref_line to CalleeInfo and update get_callers/get_callees

In `src/db.rs` `CalleeInfo` (line 1968):
```rust
pub struct CalleeInfo {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub ref_kind: String,
    pub line_start: i64,
    pub impl_type: Option<String>,
    pub ref_line: Option<i64>,
}
```

In `get_callers` (line 1432), update SQL to include `sr.ref_line`:
```sql
SELECT DISTINCT ss.name, ss.kind, sf.path, sr.kind, ss.line_start, ss.impl_type, sr.ref_line
FROM symbol_refs sr ...
```
And add `ref_line: row.get(6)?` to the `CalleeInfo` construction.

Same for `get_callees` (line 1406) — but here ref_line is less useful (it's where the call happens in the source function, not the target). Include it anyway:
```sql
SELECT DISTINCT ts.name, ts.kind, f.path, sr.kind, ts.line_start, ts.impl_type, sr.ref_line
FROM symbol_refs sr ...
```

### Step 11: Update render_callers to show ref_line

In `src/server/tools/context.rs` `render_callers` (line 209):

For callers, `ref_line` tells us WHERE in the caller the call happens. Show it:
```rust
let line = c.ref_line.unwrap_or(c.line_start);
let _ = writeln!(output, "- {} ({}:{})", display, c.file_path, line);
```

For `render_callees` (line 233), keep showing `c.line_start` (the callee's definition line — that's where you navigate TO):
```rust
let _ = writeln!(output, "- {} ({}:{})", display, c.file_path, c.line_start);
```

### Step 12: Run tests and verify

```bash
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
```

### Step 13: Add a new test for ref_line behavior

In `src/server/tools/context.rs` tests, add a test that verifies ref_line appears in output:

```rust
#[test]
fn test_context_callers_show_ref_line() {
    let db = Database::open_in_memory().unwrap();
    // ... setup with insert_symbol_ref using ref_line: Some(42)
    // ... verify output contains ":42" for the caller line
}
```

### Step 14: Commit

```bash
git add -A
git commit -m "feat: show call-site line numbers in callers instead of function definition lines

Add ref_line column to symbol_refs table, captured from tree-sitter
node positions during ref extraction. Callers now show the line where
the call occurs, not where the calling function starts.

Bumps SCHEMA_VERSION to 5 (requires re-index)."
```

---

## Task 2: Add exclude_tests filter to graph tools (Issue #3)

**Problem:** Neighborhood, callpath, and context callers mix test functions with production callers, making architecture tracing noisy.

**Approach:** Add `exclude_tests: Option<bool>` parameter to neighborhood, callpath, and context. Filter by joining `symbols.is_test = 0` in DB queries.

**Files:**
- Modify: `src/db.rs` (get_callers, get_callees, get_callers_by_name, get_callees_by_name)
- Modify: `src/server/mod.rs` (NeighborhoodParams, CallpathParams, ContextParams)
- Modify: `src/server/tools/neighborhood.rs` (handle_neighborhood, bfs_collect)
- Modify: `src/server/tools/callpath.rs` (handle_callpath, handle_shortest_path, find_all_paths)
- Modify: `src/server/tools/context.rs` (handle_context, render_callers, render_callees)

### Step 1: Add exclude_tests param to DB caller/callee methods

In `src/db.rs`, update `get_callers` to accept `exclude_tests: bool`:

```rust
pub fn get_callers(
    &self,
    symbol_name: &str,
    target_file: &str,
    exclude_tests: bool,
) -> SqlResult<Vec<CalleeInfo>> {
```

When `exclude_tests` is true, add `AND ss.is_test = 0` to the WHERE clause. Use two separate prepared statements (or dynamically build the query) to keep the fast path unchanged.

Same for `get_callees`, `get_callers_by_name`, `get_callees_by_name`.

### Step 2: Update all existing call sites

All existing callers of `get_callers(name, file)` become `get_callers(name, file, false)`. Same for the other 3 methods.

### Step 3: Add exclude_tests to MCP param structs

In `src/server/mod.rs`:

Add to `NeighborhoodParams`:
```rust
#[schemars(description = "Exclude test functions from results (default: false)")]
pub exclude_tests: Option<bool>,
```

Add to `CallpathParams`:
```rust
#[schemars(description = "Exclude test functions from paths (default: false)")]
pub exclude_tests: Option<bool>,
```

Add to `ContextParams`:
```rust
#[schemars(description = "Exclude test functions from callers/callees (default: false)")]
pub exclude_tests: Option<bool>,
```

### Step 4: Thread exclude_tests through tool handlers

In `handle_neighborhood`, pass `exclude_tests` to `bfs_collect`. In `bfs_collect`, pass to `db.get_callers_by_name` / `db.get_callees_by_name`.

In `handle_callpath`, pass to `handle_shortest_path` / `handle_all_paths` → `db.get_callees_by_name`.

In `handle_context`, pass to `render_callers` / `render_callees` → `db.get_callers` / `db.get_callees`.

### Step 5: Add tests

Test that `exclude_tests: true` filters out `#[test]` functions from neighborhood output.

### Step 6: Commit

```bash
git add -A
git commit -m "feat: add exclude_tests filter to neighborhood, callpath, and context

When set to true, test functions are excluded from caller/callee
results, making architecture tracing cleaner."
```

---

## Task 3: Compact mode for type_usage (Issue #4)

**Problem:** `type_usage "Database"` returns 48 entries at full length — too verbose for common types.

**Approach:** Add `compact: Option<bool>` parameter. When true, group results by file with counts instead of listing every entry.

**Files:**
- Modify: `src/server/mod.rs` (TypeUsageParams)
- Modify: `src/server/tools/type_usage.rs` (handle_type_usage)

### Step 1: Add compact param to TypeUsageParams

In `src/server/mod.rs`, find `TypeUsageParams` and add:
```rust
#[schemars(description = "Group by file with counts instead of listing every entry (default: false)")]
pub compact: Option<bool>,
```

### Step 2: Add compact rendering in handle_type_usage

In `src/server/tools/type_usage.rs`, after the existing rendering logic, add compact mode. When `compact` is true, group symbols by file path and show counts:

```rust
if compact {
    // Group by file
    let mut by_file: BTreeMap<&str, usize> = BTreeMap::new();
    for sym in &returns {
        *by_file.entry(&sym.file_path).or_default() += 1;
    }
    writeln!(output, "### Returns `{type_name}` ({} sites)\n", returns.len());
    for (file, count) in &by_file {
        writeln!(output, "- {file} ({count})");
    }
    // ... same for accepts, field_matches
} else {
    // existing verbose output
}
```

### Step 3: Thread compact through MCP handler

In `src/server/mod.rs`, pass `compact.unwrap_or(false)` to `handle_type_usage`.

### Step 4: Add test

Test compact mode produces grouped output.

### Step 5: Commit

```bash
git add -A
git commit -m "feat: add compact mode to type_usage tool

Groups results by file with counts for high-usage types."
```

---

## Task 4: Function-level diff in history (Capability #2)

**Problem:** `history` shows commit messages but not the actual code changes to a function.

**Approach:** Add `show_diff: Option<bool>` parameter. When true, use `git log -p -L<start>,<end>:<file>` to show actual diffs.

**Files:**
- Modify: `src/server/mod.rs` (HistoryParams)
- Modify: `src/server/tools/history.rs` (handle_history, run_git_log)

### Step 1: Add show_diff param to HistoryParams

In `src/server/mod.rs`, find `HistoryParams` and add:
```rust
#[schemars(description = "Show code diffs for each commit (default: false)")]
pub show_diff: Option<bool>,
```

### Step 2: Add diff output to run_git_log

In `src/server/tools/history.rs`, create a new function `run_git_log_with_diff` that uses `git log -L<start>,<end>:<file>` (git's built-in function-level log):

```rust
fn run_git_log_with_diff(
    repo_path: &Path,
    file: &str,
    line_start: i64,
    line_end: i64,
    limit: i64,
) -> Result<String, Box<dyn std::error::Error>> {
    let range = format!("{},{}", line_start, line_end);
    let output = std::process::Command::new("git")
        .args([
            "-C", &repo_path.display().to_string(),
            "log", "-p",
            &format!("-L{range}:{file}"),
            &format!("-n{limit}"),
            "--no-color",
        ])
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

### Step 3: Parse and display diff output

Add diff content to each commit entry. Parse the `-L` log output (it intersperses commit headers with diff hunks). Cap diff display at ~50 lines per commit to prevent token overflow.

### Step 4: Thread show_diff through handler

In `handle_history`, when `show_diff` is true, call `run_git_log_with_diff` instead of `run_git_log` and include the diff blocks in output.

### Step 5: Add test

Test that `parse_log_output` handles diff-containing output.

### Step 6: Commit

```bash
git add -A
git commit -m "feat: add show_diff option to history tool

Shows actual code diffs per commit for a symbol's line range."
```

---

## Task 5: Minor fixes

**Files:**
- Modify: `src/server/tools/stats.rs` (pluralization)
- Modify: `src/server/tools/overview.rs` (filter mod/use noise)

### Step 1: Fix pluralization in stats

In `src/server/tools/stats.rs` (handle_stats), the line `format!("{c} {k}s")` produces "type_aliass". Replace with proper pluralization:

```rust
fn pluralize_kind(kind: &str, count: i64) -> String {
    if count == 1 {
        return format!("{count} {kind}");
    }
    match kind {
        "type_alias" => format!("{count} type_aliases"),
        _ => format!("{count} {kind}s"),
    }
}
```

Use it: `kinds_str` loop uses `pluralize_kind(k, *c)` instead of `format!("{c} {k}s")`.

### Step 2: Filter mod/use from overview

In `src/server/tools/overview.rs` `handle_overview` (line 5), after loading symbols, filter out `mod` and `use` kinds unless `include_private` is true:

```rust
symbols.retain(|s| {
    s.kind != SymbolKind::Mod && s.kind != SymbolKind::Use
});
```

This keeps overview focused on actual API surface (functions, structs, enums, traits).

### Step 3: Add tests for both fixes

Test that stats output contains "type_aliases" not "type_aliass".
Test that overview output doesn't include `pub mod` entries.

### Step 4: Commit

```bash
git add -A
git commit -m "fix: pluralize type_alias correctly in stats, filter mod/use from overview"
```

---

## Task 6: Remove confirmed dead code

**Problem:** Evaluation found genuinely orphaned symbols.

**Files:**
- Modify: `src/db.rs` (remove `store_symbol_refs`, `clear_all`)
- Modify: `src/indexer/cargo_doc.rs` (remove `parse_rustdoc_json_public`)

### Step 1: Remove Database::store_symbol_refs

The non-fast version at `src/db.rs:1595-1626` is dead code — all production paths use `store_symbol_refs_fast`. Remove the entire method.

### Step 2: Remove Database::clear_all

At `src/db.rs:421-428`, no callers and no tests. Remove it.

### Step 3: Remove parse_rustdoc_json_public

At `src/indexer/cargo_doc.rs:512-517`, no callers and no tests. Remove it.

### Step 4: Verify build and tests pass

```bash
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
```

### Step 5: Commit

```bash
git add -A
git commit -m "fix: remove dead code identified by orphaned analysis

Remove Database::store_symbol_refs (superseded by _fast variant),
Database::clear_all (no callers), and parse_rustdoc_json_public
(no callers)."
```

---

## Task 7: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

### Step 1: Document new parameters

Add to CLAUDE.md Key Patterns section:
- `ref_line` in `symbol_refs` — caller lines now show call site, not function definition
- `exclude_tests` parameter on context, neighborhood, callpath
- `compact` parameter on type_usage
- `show_diff` parameter on history

### Step 2: Commit

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md with new tool parameters"
```
