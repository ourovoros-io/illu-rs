# Eight More illu-rs Improvements Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add 8 features that address remaining daily friction: file:line symbol lookup, body search, output limits, codebase stats, rename planning, context related symbols, hotspot analysis, and duplicate detection.

**Architecture:** Tasks 1-2 are new tools with thin wrappers over existing/new DB methods. Task 3 adds `limit` parameters to multiple existing tools. Tasks 4-8 are new tools with progressively more aggregation logic. All follow the same pattern: DB query in `src/db.rs`, handler in `src/server/tools/*.rs`, params + endpoint in `src/server/mod.rs`.

**Tech Stack:** Rust 2024 edition, rusqlite (bundled+fts5), rmcp, schemars

---

## Task 1: New `symbols_at` tool — lookup symbols by file + line

Exposes the existing `get_symbols_at_lines` DB method as an MCP tool. Takes a file path and a line number, returns the symbol(s) at that location.

**Files:**
- Create: `src/server/tools/symbols_at.rs`
- Modify: `src/server/tools/mod.rs` — add `pub mod symbols_at;`
- Modify: `src/server/mod.rs` — add `SymbolsAtParams` struct and MCP endpoint

**Step 1: Create handler with tests**

Create `src/server/tools/symbols_at.rs`:

```rust
use crate::db::Database;
use std::fmt::Write;

pub fn handle_symbols_at(
    db: &Database,
    file: &str,
    line: i64,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.get_symbols_at_lines(file, &[(line, line)])?;

    let mut output = String::new();

    if symbols.is_empty() {
        let _ = writeln!(
            output,
            "No symbols found at {file}:{line}.\n\
             The line may be in whitespace, a comment, or between definitions."
        );
        return Ok(output);
    }

    let _ = writeln!(output, "## Symbols at {file}:{line}\n");
    for sym in &symbols {
        let qname = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "- **{qname}** ({}) line {}-{}\n  `{}`",
            sym.kind, sym.line_start, sym.line_end, sym.signature
        );
        if let Some(doc) = &sym.doc_comment {
            if let Some(first_line) = doc.lines().next() {
                let _ = writeln!(output, "  *{first_line}*");
            }
        }
    }

    Ok(output)
}
```

Tests: symbol found at line, no symbol at line, multiple symbols overlapping (e.g., function inside impl block).

**Step 2: Wire MCP endpoint**

Add to `src/server/mod.rs`:

```rust
#[derive(Deserialize, JsonSchema)]
struct SymbolsAtParams {
    /// File path (e.g. "src/db.rs")
    file: String,
    /// Line number to look up
    line: i64,
}
```

Tool method and `pub mod symbols_at;` in tools/mod.rs. Update `get_info()`.

**Step 3: Run tests and commit**

```
cargo test --lib -- symbols_at::tests
cargo clippy --all-targets --all-features -- -D warnings
git commit -m "feat: add symbols_at tool for file:line lookup"
```

---

## Task 2: Add `bodies` scope to `query` — search within function bodies

Add a `search_symbols_by_body` DB method and a `bodies` scope to the query tool.

**Files:**
- Modify: `src/db.rs` — add `search_symbols_by_body` method
- Modify: `src/server/tools/query.rs` — add `"bodies"` scope arm
- Modify: `src/server/tools/query.rs` — update error message for unknown scope

**Step 1: Add DB method**

In `src/db.rs`, add after `search_symbols_by_doc_comment`:

```rust
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
```

DB test: insert two symbols with bodies, one containing `unwrap`, search for `unwrap`, verify only the matching one is found.

**Step 2: Add `bodies` scope to query**

In `src/server/tools/query.rs`, add to the `match scope` block:

```rust
"bodies" => format_body_search(db, query, kind, path, &mut output)?,
```

Implement `format_body_search` — same pattern as `format_doc_comments` but calls `db.search_symbols_by_body(query)?`. Update error message to include `bodies`.

**Step 3: Tests, clippy, commit**

```
cargo test --lib -- db::tests::test_search_by_body
cargo test --lib -- query::tests::test_query_bodies_scope
git commit -m "feat: add bodies scope to query tool for function body search"
```

---

## Task 3: Add `limit` parameter to `query`, `overview`, and path filtering on `context` callers/callees

Three sub-changes that all address output verbosity:

**Files:**
- Modify: `src/server/mod.rs` — add `limit` to `QueryParams` and `OverviewParams`, add `callers_path` to `ContextParams`
- Modify: `src/server/tools/query.rs` — apply limit to results
- Modify: `src/server/tools/overview.rs` — apply limit to results
- Modify: `src/server/tools/context.rs` — filter callers/callees by path prefix

### 3a: Add `limit` to query

Add `limit: Option<i64>` to `QueryParams`. In `format_symbols`, after all filters are applied but before rendering, truncate the vector:

```rust
if let Some(max) = limit {
    let max = usize::try_from(max.max(1)).unwrap_or(50);
    symbols.truncate(max);
}
```

Same for `format_doc_comments`, `format_body_search`, and `format_files`.

Test: insert 10 symbols, query with limit=3, verify only 3 returned.

### 3b: Add `limit` to overview

Add `limit: Option<i64>` to `OverviewParams`. In `handle_overview`, truncate symbols before rendering.

Test: insert 10 symbols, overview with limit=3, verify only 3 shown.

### 3c: Add `callers_path` to context

Add `callers_path: Option<String>` to `ContextParams`. Pass through to `handle_context`. In `render_callers` and `render_callees`, filter by path prefix:

```rust
fn render_callers(
    db: &Database,
    output: &mut String,
    sym: &StoredSymbol,
    callers_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut callers = db.get_callers(&sym.name, &sym.file_path)?;
    if let Some(p) = callers_path {
        callers.retain(|c| c.file_path.starts_with(p));
    }
    // ... rest unchanged
}
```

This lets users say "show me callers only from `src/`, not `tests/`."

Test: insert callers from `src/` and `tests/`, request with `callers_path: "src/"`, verify only src callers shown.

**Step: Tests, clippy, commit**

```
cargo test --lib -- query::tests context::tests overview::tests
git commit -m "feat: add limit param to query/overview, callers_path to context"
```

---

## Task 4: New `stats` tool — codebase health dashboard

Aggregation queries over existing data. No new tables.

**Files:**
- Create: `src/server/tools/stats.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Handler:**

```rust
pub fn handle_stats(
    db: &Database,
    path: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>>
```

Computes and formats:
1. **File count** — `db.file_count()` or count files with prefix
2. **Symbol count** — count from `get_symbols_by_path_prefix`
3. **Symbol breakdown by kind** — group by kind
4. **Test count** — count symbols with `test` attribute
5. **Untested function count** — count functions with no related tests (reuse logic from `handle_untested`)
6. **Top 10 most-called symbols** — SQL query: `SELECT ts.name, COUNT(*) as refs FROM symbol_refs sr JOIN symbols ts ON ts.id = sr.target_symbol_id GROUP BY ts.name ORDER BY refs DESC LIMIT 10`
7. **Top 10 most-calling symbols** — SQL query on source side
8. **Largest files** — top 5 by symbol count from `get_file_symbol_counts`

New DB methods needed:
- `get_most_referenced_symbols(limit: i64, path_prefix: &str)` — returns `Vec<(String, i64)>`
- `get_most_referencing_symbols(limit: i64, path_prefix: &str)` — returns `Vec<(String, i64)>`

**Output format:**

```
## Codebase Stats

### Overview
- **Files:** 29
- **Symbols:** 194 (68 functions, 12 structs, 3 enums, ...)
- **Tests:** 150
- **Untested functions:** 12 / 68 (82% coverage)

### Most Referenced (fragile to change)
1. **Database** — 63 references
2. **StoredSymbol** — 22 references
...

### Most Referencing (doing the most)
1. **handle_context** — 10 callees
2. **index_repo** — 8 callees
...

### Largest Files
1. **src/db.rs** — 81 symbols
2. **src/indexer/parser.rs** — 30 symbols
...
```

**MCP params:**

```rust
#[derive(Deserialize, JsonSchema)]
struct StatsParams {
    /// Filter to files under this path prefix (default: all)
    path: Option<String>,
}
```

**Tests:** stats on an empty DB returns zeros; stats on a populated DB returns correct counts.

**Commit:** `feat: add stats tool for codebase health dashboard`

---

## Task 5: New `rename_plan` tool — unified rename impact preview

Shows every location that references a symbol, grouped by category, for planning a rename.

**Files:**
- Create: `src/server/tools/rename_plan.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Handler:**

```rust
pub fn handle_rename_plan(
    db: &Database,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>>
```

Gathers from multiple sources:
1. **Definition** — resolve the symbol, show file:line
2. **Direct callers** — from `get_callers` (the call sites that need updating)
3. **Type usage** — from `search_symbols_by_signature` (signatures mentioning the name)
4. **Struct fields** — from details column search (fields typed with this name)
5. **Trait implementations** — from `get_trait_impls_for_type` / `get_trait_impls_for_trait`
6. **Doc comments mentioning it** — from `search_symbols_by_doc_comment`
7. **Total count** — summary of how many locations need changing

**Output format:**

```
## Rename Plan: `Database`

### Definition
- **Database** (struct) at src/db.rs:96-99

### Call Sites (63 references)
#### src/server/tools/context.rs
- handle_context (line 6)
- render_callers (line 195)
...
#### tests/integration.rs
- setup_indexed_db (line 10)
...

### Type Usage in Signatures (8 functions)
- handle_query: `pub fn handle_query(db: &Database, ...)`
...

### Struct Fields (0 structs)

### Trait Implementations (1)
- ServerHandler for IlluServer (src/server/mod.rs:547)

### Doc Comments Mentioning "Database" (2 symbols)
- ...

**Total: ~75 locations across 15 files**
```

**MCP params:**

```rust
#[derive(Deserialize, JsonSchema)]
struct RenamePlanParams {
    /// Symbol name to plan a rename for
    symbol_name: String,
}
```

**Tests:** rename plan for a symbol with callers, type usage, and trait impls; rename plan for a symbol with no references.

**Commit:** `feat: add rename_plan tool for unified rename impact preview`

---

## Task 6: Add `related` section to `context` tool

Show sibling symbols defined in the same file (and optionally same impl block).

**Files:**
- Modify: `src/server/tools/context.rs` — add `render_related` function

**Implementation:**

Add a new `render_related` function that:
1. Gets all symbols in the same file via `get_symbols_by_path_prefix(sym.file_path)`
2. Filters to symbols in the same impl block (same `impl_type`) or top-level if no impl_type
3. Excludes the current symbol
4. Excludes Use/Mod/EnumVariant kinds
5. Renders as a compact list

```rust
fn render_related(
    db: &Database,
    output: &mut String,
    sym: &StoredSymbol,
) -> Result<(), Box<dyn std::error::Error>> {
    let siblings = db.get_symbols_by_path_prefix(&sym.file_path)?;
    let related: Vec<_> = siblings
        .iter()
        .filter(|s| s.name != sym.name || s.line_start != sym.line_start)
        .filter(|s| s.impl_type == sym.impl_type)
        .filter(|s| {
            s.kind != SymbolKind::Use
                && s.kind != SymbolKind::Mod
                && s.kind != SymbolKind::EnumVariant
                && s.kind != SymbolKind::Impl
        })
        .collect();

    if related.is_empty() {
        return Ok(());
    }

    let label = if let Some(it) = &sym.impl_type {
        format!("Related (impl {it})")
    } else {
        "Related (same file)".to_string()
    };
    let _ = writeln!(output, "### {label}\n");
    for s in &related {
        let _ = writeln!(output, "- **{}** ({}, line {}-{})", s.name, s.kind, s.line_start, s.line_end);
    }
    let _ = writeln!(output);
    Ok(())
}
```

Add `"related"` to the sections system in `handle_context`:
```rust
if show("related") {
    render_related(db, &mut output, sym)?;
}
```

**Tests:** symbol in an impl block shows other methods in same impl; top-level function shows other top-level functions in same file; related section can be filtered via sections param.

**Commit:** `feat: add related section to context showing sibling symbols`

---

## Task 7: New `hotspots` tool — complexity and coupling analysis

Identifies high-risk symbols by combining multiple metrics.

**Files:**
- Create: `src/server/tools/hotspots.rs`
- Modify: `src/db.rs` — add `get_most_referenced_symbols` and `get_most_referencing_symbols`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**DB methods:**

```rust
pub fn get_most_referenced_symbols(
    &self,
    limit: i64,
    path_prefix: &str,
) -> SqlResult<Vec<(String, String, i64)>> {
    let pattern = format!("{path_prefix}%");
    let mut stmt = self.conn.prepare(
        "SELECT ts.name, f.path, COUNT(DISTINCT sr.source_symbol_id) as ref_count \
         FROM symbol_refs sr \
         JOIN symbols ts ON ts.id = sr.target_symbol_id \
         JOIN files f ON f.id = ts.file_id \
         WHERE f.path LIKE ?1 \
         GROUP BY ts.id \
         ORDER BY ref_count DESC \
         LIMIT ?2",
    )?;
    // ... collect (name, file_path, count) tuples
}

pub fn get_most_referencing_symbols(
    &self,
    limit: i64,
    path_prefix: &str,
) -> SqlResult<Vec<(String, String, i64)>> {
    // Same but GROUP BY sr.source_symbol_id
}
```

**Handler:**

Produces three sections:
1. **Most Referenced** (highest in-degree) — fragile, many things depend on them
2. **Most Referencing** (highest out-degree) — doing the most, possible god functions
3. **Largest Functions** (biggest line span) — complexity candidates

```rust
pub fn handle_hotspots(
    db: &Database,
    path: Option<&str>,
    limit: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>>
```

**Output format:**

```
## Hotspots

### Most Referenced (fragile to change)
| Symbol | File | References |
|--------|------|-----------|
| Database | src/db.rs | 63 |
| StoredSymbol | src/db.rs | 22 |
...

### Most Referencing (high complexity)
| Symbol | File | Callees |
|--------|------|---------|
| handle_context | src/server/tools/context.rs | 10 |
...

### Largest Functions (by line count)
| Symbol | File | Lines |
|--------|------|-------|
| search_symbols | src/db.rs | 93 |
...
```

**MCP params:**

```rust
#[derive(Deserialize, JsonSchema)]
struct HotspotsParams {
    /// Filter to files under this path prefix
    path: Option<String>,
    /// Max entries per section (default: 10)
    limit: Option<i64>,
}
```

**Tests:** verify ordering is correct (highest first); empty DB returns empty sections.

**Commit:** `feat: add hotspots tool for complexity and coupling analysis`

---

## Task 8: New `similar` tool — find symbols with similar signatures

Finds symbols that structurally resemble a given symbol (same parameter types, similar return type, similar callee pattern).

**Files:**
- Create: `src/server/tools/similar.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Handler:**

```rust
pub fn handle_similar(
    db: &Database,
    symbol_name: &str,
    path: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>>
```

Algorithm:
1. Resolve the target symbol to get its signature and callees
2. Extract the return type from the signature (text after `->`)
3. Extract parameter types from the signature
4. Search for other symbols with similar return type via `search_symbols_by_signature`
5. For each candidate, compute a similarity score:
   - Same return type: +2
   - Shares parameter types: +1 per shared type
   - Shares callees: +1 per shared callee (from `get_callees_by_name`)
6. Rank by score, return top matches

**Output format:**

```
## Similar to `handle_query`

Signature: `pub fn handle_query(db: &Database, query: &str, ...) -> Result<String, ...>`

### Similar Symbols

1. **handle_context** (score: 4) — src/server/tools/context.rs:6
   `pub fn handle_context(db: &Database, ...) -> Result<String, ...>`
   Shared: return type `Result<String>`, param `&Database`, callees: resolve_symbol

2. **handle_impact** (score: 3) — src/server/tools/impact.rs:4
   `pub fn handle_impact(db: &Database, ...) -> Result<String, ...>`
   Shared: return type `Result<String>`, param `&Database`
...
```

**MCP params:**

```rust
#[derive(Deserialize, JsonSchema)]
struct SimilarParams {
    /// Symbol to find similar symbols for
    symbol_name: String,
    /// Filter to files under this path prefix
    path: Option<String>,
}
```

**Tests:** two functions with same signature pattern rank high; unrelated functions rank low; symbol not found gives clear message.

**Commit:** `feat: add similar tool for finding structurally similar symbols`

---

## Post-Implementation Checklist

1. **Update CLAUDE.md** — add new tools to commands table and key patterns
2. **Update `get_info()` instructions** — mention all new tools
3. **Run full test suite:** `cargo test`
4. **Run clippy:** `cargo clippy --all-targets --all-features -- -D warnings`
5. **Run formatter:** `cargo fmt --all -- --check`
6. **Final commit with CLAUDE.md updates**

## Dependency Order

- **Task 1** (symbols_at): independent
- **Task 2** (body search): independent
- **Task 3** (limits): independent, but modifies query.rs/context.rs/overview.rs
- **Task 4** (stats): depends on Task 7's DB methods (most_referenced/most_referencing) — implement Task 7's DB methods first, or duplicate them
- **Task 5** (rename_plan): independent
- **Task 6** (context related): independent
- **Task 7** (hotspots): independent, provides DB methods reused by Task 4
- **Task 8** (similar): independent

Recommended order: **1, 2, 7, 4, 3, 5, 6, 8** (7 before 4 to share DB methods).

All tasks modify `src/server/mod.rs` (adding param structs), so they must be implemented sequentially.
