# Nine Data Quality & Feature Improvements Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the top data quality issues (signature truncation, noisy refs, missing line numbers) and add 5 new features (call tree, body search context, module boundary analysis, index health report, git blame integration).

**Architecture:** Tasks 1-3 fix core data quality in the parser and DB. Task 4 adds a call tree view to neighborhood. Tasks 5-8 are new tools. All changes follow existing patterns. Task 1 (signature fix) should be done first as it improves multiple downstream tools.

**Tech Stack:** Rust 2024 edition, rusqlite (bundled+fts5), rmcp, tree-sitter

---

## Task 1: Fix multi-line signature truncation

The `get_first_line` function in `src/indexer/parser.rs:615-618` truncates signatures to the first line. Replace it with a function that captures everything from the function keyword up to (but not including) the opening `{`.

**Files:**
- Modify: `src/indexer/parser.rs:615-618` — replace `get_first_line` with `get_signature`

**Step 1: Write the test**

In the existing parser test module, add:

```rust
#[test]
fn test_multiline_signature_fully_captured() {
    let source = r#"
pub fn handle_query(
    db: &Database,
    query: &str,
    scope: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    todo!()
}
"#;
    let (symbols, _) = parse_rust_source(source, "test.rs").unwrap();
    let func = symbols.iter().find(|s| s.name == "handle_query").unwrap();
    assert!(
        func.signature.contains("-> Result<String"),
        "signature should include return type: {}",
        func.signature,
    );
    assert!(
        func.signature.contains("&Database"),
        "signature should include params: {}",
        func.signature,
    );
    assert!(
        !func.signature.contains("todo!"),
        "signature should not include body: {}",
        func.signature,
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- parser::tests::test_multiline_signature_fully_captured`
Expected: FAIL — current `get_first_line` returns only `pub fn handle_query(`

**Step 3: Replace `get_first_line` with `get_signature`**

Replace the function at `src/indexer/parser.rs:615-618`:

```rust
/// Extract the full signature of a symbol, from the start up to (not including) the opening `{`.
/// Falls back to first line if no `{` is found. Collapses whitespace for readability.
fn get_signature(node: &Node, source: &str) -> String {
    let text = node_text(node, source);
    // Find the opening brace
    let sig_end = text.find('{').unwrap_or(text.len());
    let raw_sig = text[..sig_end].trim();
    // Collapse internal whitespace (newlines + indentation → single space)
    let collapsed: String = raw_sig.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
}
```

This collapses multi-line signatures into a single line: `pub fn handle_query( db: &Database, query: &str, scope: Option<&str>, ) -> Result<String, Box<dyn std::error::Error>>`.

**Step 4: Update all call sites of `get_first_line` to `get_signature`**

Search for `get_first_line` in `src/indexer/parser.rs` — it's called in `extract_function`, `extract_function_signature`, `extract_named_item`, and `extract_macro_def`. Replace all calls.

NOTE: For structs/enums/traits, the first-line behavior is actually correct (the full type definition is often `pub struct Foo {`). So use `get_signature` only for functions. For non-functions, keep using a `get_first_line` that just gets the first line. Alternatively, `get_signature` works for all since it captures up to `{` — which for `pub struct Foo {` is just `pub struct Foo`.

Actually, `get_signature` handles all cases correctly:
- `pub struct Foo {` → `pub struct Foo`
- `pub fn foo(\n    x: i32,\n) -> bool {` → `pub fn foo( x: i32, ) -> bool`
- `macro_rules! my_macro {` → `macro_rules! my_macro`

So replace ALL call sites uniformly.

**Step 5: Run tests**

Run: `cargo test --lib -- parser::tests`
Expected: All pass, including the new `test_multiline_signature_fully_captured`.

NOTE: Some existing tests may assert specific signature formats (e.g., `signature_is_first_line_only` in data_integrity tests). These may need updating since signatures are no longer first-line-only. Search the test files for assertions about signature content and update them to match the new collapsed format.

**Step 6: Commit**

```
git commit -m "fix: capture full multi-line signatures instead of truncating to first line"
```

---

## Task 2: Add confidence scoring to symbol refs

Add a `confidence` column to `symbol_refs` table. Refs resolved via impl_type or file path get `high` confidence. Refs resolved via name-only fallback get `low` confidence. Expose this in tools that benefit from filtering.

**Files:**
- Modify: `src/db.rs` — add `confidence` column to schema + migration, update `insert_symbol_ref`, add `get_file_dependencies_filtered`
- Modify: `src/db.rs` — update `SymbolIdMap::resolve` to return confidence level
- Modify: `src/db.rs` — update `store_symbol_refs_fast` and `store_symbol_refs` to pass confidence
- Modify: `src/server/tools/file_graph.rs` — use high-confidence refs only by default

**Step 1: Add confidence column to schema**

In the `CREATE TABLE symbol_refs` block in `db.rs` migrate function, add:

```sql
ALTER TABLE symbol_refs ADD COLUMN confidence TEXT NOT NULL DEFAULT 'high';
```

Add this as a migration step in `Database::migrate()`. Also update the `CREATE TABLE` for fresh DBs:

```sql
CREATE TABLE IF NOT EXISTS symbol_refs (
    id INTEGER PRIMARY KEY,
    source_symbol_id INTEGER NOT NULL REFERENCES symbols(id),
    target_symbol_id INTEGER NOT NULL REFERENCES symbols(id),
    kind TEXT NOT NULL,
    confidence TEXT NOT NULL DEFAULT 'high'
);
```

**Step 2: Update `SymbolIdMap::resolve` to return confidence**

Change return type from `Option<SymbolId>` to `Option<(SymbolId, &'static str)>`:

```rust
pub fn resolve(
    &self,
    name: &str,
    target_file: Option<&str>,
    target_context: Option<&str>,
) -> Option<(SymbolId, &'static str)> {
    if let Some(ctx) = target_context
        && let Some(id) = self.impl_qualified.get(&(name.to_string(), ctx.to_string()))
    {
        return Some((*id, "high"));
    }
    if let Some(file) = target_file
        && let Some(id) = self.file_qualified.get(&(name.to_string(), file.to_string()))
    {
        return Some((*id, "high"));
    }
    self.name_only.get(name).map(|id| (*id, "low"))
}
```

**Step 3: Update `insert_symbol_ref` and callers**

Add `confidence: &str` parameter to `insert_symbol_ref`:

```rust
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
```

Update `store_symbol_refs_fast` and `store_symbol_refs` to pass confidence from the resolve result.

**CRITICAL:** Update ALL callers of `insert_symbol_ref` across the entire codebase (search for `insert_symbol_ref(`). Tests that call it directly need `"high"` as the new parameter.

**Step 4: Add filtered file dependency query**

```rust
pub fn get_file_dependencies_filtered(
    &self,
    path_prefix: &str,
    min_confidence: &str,
) -> SqlResult<Vec<(String, String)>> {
    let pattern = format!("{path_prefix}%");
    let mut stmt = self.conn.prepare_cached(
        "SELECT DISTINCT sf.path, tf.path \
         FROM symbol_refs sr \
         JOIN symbols ss ON ss.id = sr.source_symbol_id \
         JOIN symbols ts ON ts.id = sr.target_symbol_id \
         JOIN files sf ON sf.id = ss.file_id \
         JOIN files tf ON tf.id = ts.file_id \
         WHERE sf.path LIKE ?1 AND sf.path != tf.path \
           AND sr.confidence = ?2 \
         ORDER BY sf.path, tf.path",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query(params![pattern, min_confidence])?;
    while let Some(row) = rows.next()? {
        results.push((row.get(0)?, row.get(1)?));
    }
    Ok(results)
}
```

**Step 5: Update `file_graph` to use high-confidence refs by default**

In `src/server/tools/file_graph.rs`, change to use `get_file_dependencies_filtered(path, "high")` by default. Optionally add an `include_low_confidence` param.

**Step 6: Run tests, commit**

```
cargo test --lib && cargo clippy --all-targets --all-features -- -D warnings
git commit -m "feat: add confidence scoring to symbol refs, filter noisy edges"
```

---

## Task 3: Add source line numbers to callers/callees in context

Currently callers show `- handle_context (src/server/tools/context.rs)` without line numbers. Add the caller's line_start for navigability.

**Files:**
- Modify: `src/db.rs` — update `get_callers` and `get_callees` queries to include `ss.line_start`/`ts.line_start`
- Modify: `src/db.rs` — add `line_start` field to `CalleeInfo`
- Modify: `src/server/tools/context.rs` — render line numbers in callers/callees

**Step 1: Add `line_start` to `CalleeInfo`**

In `src/db.rs`, update:

```rust
pub struct CalleeInfo {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub ref_kind: String,
    pub line_start: i64,
}
```

**Step 2: Update SQL queries**

In `get_callers`, add `ss.line_start` to SELECT:

```sql
SELECT DISTINCT ss.name, ss.kind, sf.path, sr.kind, ss.line_start
FROM symbol_refs sr ...
```

In `get_callees`, add `ts.line_start`:

```sql
SELECT DISTINCT ts.name, ts.kind, f.path, sr.kind, ts.line_start
FROM symbol_refs sr ...
```

Update the row-reading code to populate the new field: `line_start: row.get(4)?`.

**Step 3: Update context rendering**

In `render_callers` in `context.rs`, change:
```rust
// Before:
let _ = writeln!(output, "- {} ({})", c.name, c.file_path);
// After:
let _ = writeln!(output, "- {} ({}:{})", c.name, c.file_path, c.line_start);
```

Same for `render_callees`.

**Step 4: Update tests that assert caller/callee output format**

Search for assertions on "Called By" and "Callees" output format across all test files.

**Step 5: Run tests, commit**

```
git commit -m "feat: add line numbers to callers/callees in context output"
```

---

## Task 4: Add call tree visualization to neighborhood

Add `direction` and `format` parameters to neighborhood. When `direction: "down"` and `format: "tree"`, render a hierarchical call tree instead of a flat list.

**Files:**
- Modify: `src/server/mod.rs` — add `direction` and `format` to `NeighborhoodParams`
- Modify: `src/server/tools/neighborhood.rs` — add tree rendering mode

**Step 1: Add parameters**

```rust
#[derive(Deserialize, JsonSchema)]
struct NeighborhoodParams {
    symbol_name: String,
    depth: Option<i64>,
    /// Direction: "both" (default), "down" (callees only), "up" (callers only)
    direction: Option<String>,
    /// Format: "list" (default), "tree" (hierarchical)
    format: Option<String>,
}
```

**Step 2: Implement tree rendering**

Add a `render_call_tree` function that does DFS and renders with indent:

```rust
fn render_call_tree(
    db: &Database,
    output: &mut String,
    name: &str,
    depth: usize,
    max_depth: usize,
    visited: &mut std::collections::HashSet<String>,
    prefix: &str,
    is_last: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connector = if prefix.is_empty() { "" } else if is_last { "└── " } else { "├── " };
    let _ = writeln!(output, "{prefix}{connector}**{name}**");

    if depth >= max_depth || !visited.insert(name.to_string()) {
        return Ok(());
    }

    let callees = db.get_callees_by_name(name)?;
    let child_prefix = if prefix.is_empty() {
        String::new()
    } else if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}│   ")
    };

    for (i, (callee, _)) in callees.iter().enumerate() {
        let last = i == callees.len() - 1;
        render_call_tree(db, output, callee, depth + 1, max_depth, visited, &child_prefix, last)?;
    }

    visited.remove(name);
    Ok(())
}
```

**Step 3: Wire into handle_neighborhood**

When `format == "tree"` and `direction == "down"`, use `render_call_tree` instead of the flat BFS.

**Step 4: Tests, commit**

```
git commit -m "feat: add tree format and direction params to neighborhood"
```

---

## Task 5: Add body search context snippets

When `scope: "bodies"` is used, show the matching line(s) from the body alongside the symbol.

**Files:**
- Modify: `src/server/tools/query.rs` — update `format_body_search` to extract and show matching lines

**Step 1: Update `format_body_search`**

After finding symbols via `search_symbols_by_body`, for each match, extract the matching line from the body:

```rust
if let Some(body) = &sym.body {
    for line in body.lines() {
        if line.to_lowercase().contains(&query.to_lowercase()) {
            let trimmed = line.trim();
            let _ = writeln!(output, "  > `{trimmed}`");
            break; // Show first matching line only
        }
    }
}
```

**Step 2: Test, commit**

```
git commit -m "feat: show matching body line in body search results"
```

---

## Task 6: New `boundary` tool — module API boundary analysis

Shows which symbols in a path are used by code OUTSIDE that path (the effective public API boundary).

**Files:**
- Create: `src/server/tools/boundary.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Implement handler**

```rust
pub fn handle_boundary(
    db: &Database,
    path: &str,
) -> Result<String, Box<dyn std::error::Error>>
```

Logic:
1. Get all symbols in the given path prefix
2. For each symbol, get its callers via `get_callers`
3. If ANY caller is from OUTSIDE the path prefix, the symbol is part of the boundary
4. Group boundary symbols by file, show which external files use them

**Output format:**

```
## Module Boundary: src/server/tools/

### Public API (used externally)

#### src/server/tools/query.rs
- **handle_query** — used by: src/server/mod.rs, src/main.rs

#### src/server/tools/context.rs
- **handle_context** — used by: src/server/mod.rs, src/main.rs

### Internal Only (safe to refactor)
- resolve_symbol (src/server/tools/mod.rs)
- qualified_name (src/server/tools/mod.rs)
- format_symbols (src/server/tools/query.rs)
```

**Step 2: Wire MCP endpoint, tests, commit**

```
git commit -m "feat: add boundary tool for module API analysis"
```

---

## Task 7: New `health` tool — index quality self-diagnosis

Reports on the quality and completeness of the index.

**Files:**
- Create: `src/server/tools/health.rs`
- Modify: `src/db.rs` — add diagnostic queries
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Add diagnostic DB methods**

```rust
/// Count refs by confidence level
pub fn count_refs_by_confidence(&self) -> SqlResult<Vec<(String, i64)>> { ... }

/// Count symbols with truncated signatures (signature ends with '(')
pub fn count_truncated_signatures(&self) -> SqlResult<i64> { ... }

/// Get symbols with highest fan-in (most refs targeting them)
pub fn get_highest_fan_in(&self, limit: i64) -> SqlResult<Vec<(String, i64)>> { ... }
```

Note: `count_truncated_signatures` checks if signature ends with `(` — after Task 1 fixes the parser, this should drop to near zero. It serves as a quality check.

**Step 2: Implement handler**

Output:

```
## Index Health

### Ref Quality
- **Total refs:** 1,234
- **High confidence:** 890 (72%)
- **Low confidence:** 344 (28%)

### Signature Quality
- **Total symbols:** 770
- **Truncated signatures:** 12 (1.6%)

### Potential Noise Sources
- **new** — 123 low-confidence refs (likely name collisions)
- **push** — 81 low-confidence refs
- **set** — 45 low-confidence refs

### Coverage
- **Files indexed:** 40
- **Parse failures:** 0
```

**Step 3: Wire MCP endpoint, tests, commit**

```
git commit -m "feat: add health tool for index quality diagnosis"
```

---

## Task 8: Git blame integration on symbols

A `blame` tool that runs `git blame` on a symbol's line range and summarizes who last modified it.

**Files:**
- Create: `src/server/tools/blame.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Implement handler**

```rust
pub fn handle_blame(
    db: &Database,
    repo_path: &Path,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>>
```

Logic:
1. Resolve symbol to get file_path, line_start, line_end
2. Run `git blame -L {line_start},{line_end} --porcelain {file_path}` in repo_path
3. Parse porcelain output to extract: author, date, commit hash, summary
4. Aggregate: most recent commit, all authors, date range

**Output:**

```
## Blame: handle_context

- **File:** src/server/tools/context.rs:6-56
- **Last modified:** 2026-03-20 by GeorgiosDelkos
- **Commit:** abc123 — feat: add sections filter to context tool
- **Authors:** GeorgiosDelkos (100%)
- **Age:** 1 day old
```

**Step 2: Wire MCP endpoint, tests**

Note: tests for this tool are tricky since they require a real git repo. Test the porcelain parsing function separately with sample input, and test the handler integration in the `self_index` test suite.

**Step 3: Commit**

```
git commit -m "feat: add blame tool for git blame integration on symbols"
```

---

## Post-Implementation Checklist

1. **Update CLAUDE.md** — add new tools, update key patterns for signature fix and confidence scoring
2. **Update `get_info()` instructions** — mention new tools
3. **Re-index the repo** — signature fix requires full re-index to populate new signatures
4. **Run full test suite:** `cargo test`
5. **Run clippy:** `cargo clippy --all-targets --all-features -- -D warnings`

## Dependency Order

- **Task 1** (signature fix) — FIRST, improves data for all other tasks
- **Task 2** (confidence scoring) — requires schema migration, do early
- **Task 3** (caller line numbers) — independent of 1-2
- **Task 4** (call tree) — independent
- **Task 5** (body snippets) — independent
- **Task 6** (boundary) — benefits from Task 2 (high-confidence refs only)
- **Task 7** (health) — depends on Task 2 (reports confidence stats)
- **Task 8** (blame) — independent

Recommended order: **1, 2, 3, 4, 5, 6, 7, 8** (sequential — most tasks modify shared files).
