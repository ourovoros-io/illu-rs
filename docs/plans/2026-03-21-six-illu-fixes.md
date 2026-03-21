# Six illu-rs Fixes: Agent Quality-of-Life

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix 6 bugs discovered during agent self-evaluation of illu tools, ranked by severity.

**Architecture:** All fixes are in `src/db.rs` (DB methods) and `src/server/tools/` (handlers). No schema changes. No new tables. Pure logic fixes.

**Tech Stack:** Rust, rusqlite, tree-sitter (no new deps)

---

## Task 1: Fix impact/test_impact for Type::method symbols (Critical)

The `impact` and `test_impact` tools return "No dependents found" for ANY method with `impl_type` (e.g. `Database::open`, `Database::migrate`). Works fine for standalone functions. Root cause: `impact_dependents_with_depth` and `get_related_tests` in `db.rs` use `WHERE s.name = ?1` with the raw string `"Database::open"`, but DB stores `name = "open"` with `impl_type = "Database"`.

**Files:**
- Modify: `src/db.rs:838-876` (`impact_dependents_with_depth`)
- Modify: `src/db.rs:880-910` (`get_related_tests`)
- Modify: `src/server/tools/impact.rs:40` (pass resolved name + impl_type)
- Modify: `src/server/tools/impact.rs:71` (pass resolved name + impl_type)
- Modify: `src/server/tools/test_impact.rs` (same pattern)
- Test: `src/server/tools/impact.rs` (existing tests + new test)

**Step 1: Write failing test for impact with impl_type methods**

Add to `src/server/tools/impact.rs` tests module:

```rust
#[test]
fn test_impact_with_impl_type() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "open".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn open() -> Self".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("Database".into()),
            },
            Symbol {
                name: "caller_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 20,
                signature: "pub fn caller_fn()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let open_id = db.get_symbol_id("open", "src/lib.rs").unwrap().unwrap();
    let caller_id = db.get_symbol_id("caller_fn", "src/lib.rs").unwrap().unwrap();
    db.insert_symbol_ref(caller_id, open_id, "call", "high").unwrap();

    let result = handle_impact(&db, "Database::open", None, false).unwrap();
    assert!(
        result.contains("caller_fn"),
        "impact should find callers for Type::method syntax, got: {result}"
    );
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test --lib -- tools::impact::tests::test_impact_with_impl_type
```

Expected: FAIL — `"No dependents found"` does not contain `"caller_fn"`.

**Step 3: Fix DB methods to accept split name + impl_type**

In `src/db.rs`, modify `impact_dependents_with_depth` to accept optional `impl_type`:

```rust
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
            WHERE s.name = ?1
              AND (?3 IS NULL OR s.impl_type = ?3)
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
    // ... rest unchanged
```

Same pattern for `get_related_tests`:

```rust
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
            WHERE s.name = ?1
              AND (?2 IS NULL OR s.impl_type = ?2)
          UNION
            ...
```

**Step 4: Update all callers of these DB methods**

In `src/server/tools/impact.rs`, use the resolved symbol to extract name and impl_type:

```rust
// After line 17 (after the symbols.is_empty() check), extract the name parts:
let (base_name, impl_type) = if let Some((it, method)) = symbol_name.split_once("::") {
    (method, Some(it))
} else {
    (symbol_name, None)
};

// Line 40: pass split parts
let dependents = db.impact_dependents_with_depth(base_name, impl_type, depth)?;

// Line 71: pass split parts
let tests = db.get_related_tests(base_name, impl_type)?;
```

Apply same pattern to `src/server/tools/test_impact.rs`.

Also update all other callers of `get_related_tests`:
- `src/server/tools/context.rs:281` — `render_tested_by` passes `&sym.name`. Fix: also pass `sym.impl_type.as_deref()`.
- `src/server/tools/stats.rs:59` — passes `&sym.name`. Fix: also pass `sym.impl_type.as_deref()`.

**Step 5: Run tests**

```bash
cargo test --lib -- tools::impact::tests
```

Expected: ALL PASS including the new `test_impact_with_impl_type`.

**Step 6: Commit**

```bash
git add src/db.rs src/server/tools/impact.rs src/server/tools/test_impact.rs src/server/tools/context.rs src/server/tools/stats.rs
git commit -m "fix: impact and get_related_tests handle Type::method syntax

The CTE seed queries used WHERE s.name = ?1 with the unsplit
'Type::method' string, but DB stores name='method' with
impl_type='Type'. Now accepts optional impl_type parameter."
```

---

## Task 2: Fix orphaned false positives (shares root cause with Task 1)

After Task 1 lands, `get_related_tests` now handles impl_type. But `handle_orphaned` also calls `get_unreferenced_symbols` which is a separate code path.

**Files:**
- Modify: `src/server/tools/orphaned.rs`
- Test: `src/server/tools/orphaned.rs`

**Step 1: Write failing test**

Add to `src/server/tools/orphaned.rs` tests (create if needed):

```rust
#[test]
fn test_orphaned_excludes_test_called_methods() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "open_in_memory".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn open_in_memory() -> Self".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some("Database".into()),
            },
            Symbol {
                name: "test_db".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 20,
                signature: "fn test_db()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: Some("test".into()),
                impl_type: None,
            },
        ],
    )
    .unwrap();

    // test_db calls open_in_memory
    let open_id = db.get_symbol_id("open_in_memory", "src/lib.rs").unwrap().unwrap();
    let test_id = db.get_symbol_id("test_db", "src/lib.rs").unwrap().unwrap();
    db.insert_symbol_ref(test_id, open_id, "call", "high").unwrap();

    let result = handle_orphaned(&db, None, None).unwrap();
    assert!(
        !result.contains("open_in_memory"),
        "method called by tests should NOT be orphaned, got: {result}"
    );
}
```

**Step 2: Run to verify failure**

```bash
cargo test --lib -- tools::orphaned::tests::test_orphaned_excludes_test_called_methods
```

**Step 3: Fix**

The orphaned tool finds the intersection of "unreferenced" and "untested". The "unreferenced" part uses `get_unreferenced_symbols` (SQL LEFT JOIN on `symbol_refs`) — this should already work since refs are by symbol ID, not name. The issue is likely in the "untested" check which calls `get_related_tests`. Since Task 1 fixed `get_related_tests` to accept `impl_type`, update the orphaned handler to pass `sym.impl_type.as_deref()`.

Read `src/server/tools/orphaned.rs` to find the exact `get_related_tests` call and add the `impl_type` parameter.

**Step 4: Run tests**

```bash
cargo test --lib -- tools::orphaned::tests
```

**Step 5: Commit**

```bash
git add src/server/tools/orphaned.rs
git commit -m "fix: orphaned passes impl_type to get_related_tests"
```

---

## Task 3: Fix stats most-referenced inflation by low-confidence refs

`handle_stats` calls `get_most_referenced_symbols(5, prefix, None)` — no confidence filter. This makes `new` appear with 129 refs because all `Type::new()` calls resolved via name-only fallback (low confidence) get counted. Hotspots correctly uses `Some("high")`.

Also, stats displays bare `name` not qualified `ImplType::name`, making it ambiguous.

**Files:**
- Modify: `src/server/tools/stats.rs:90`
- Test: `src/server/tools/stats.rs`

**Step 1: Write failing test**

```rust
#[test]
fn test_stats_most_referenced_uses_high_confidence() {
    let db = Database::open_in_memory().unwrap();
    let f = db.insert_file("src/lib.rs", "h1").unwrap();

    let symbols = vec![
        make_sym("real_fn", SymbolKind::Function, "src/lib.rs", None),
        make_sym("noise_fn", SymbolKind::Function, "src/lib.rs", None),
        make_sym("caller1", SymbolKind::Function, "src/lib.rs", None),
        make_sym("caller2", SymbolKind::Function, "src/lib.rs", None),
        make_sym("caller3", SymbolKind::Function, "src/lib.rs", None),
    ];
    store_symbols(&db, f, &symbols).unwrap();

    let real_id = sym_id(&db, "real_fn");
    let noise_id = sym_id(&db, "noise_fn");
    let c1 = sym_id(&db, "caller1");
    let c2 = sym_id(&db, "caller2");
    let c3 = sym_id(&db, "caller3");

    // real_fn: 2 high-confidence refs
    db.insert_symbol_ref(c1, real_id, "call", "high").unwrap();
    db.insert_symbol_ref(c2, real_id, "call", "high").unwrap();

    // noise_fn: 3 refs but all low-confidence
    db.insert_symbol_ref(c1, noise_id, "call", "low").unwrap();
    db.insert_symbol_ref(c2, noise_id, "call", "low").unwrap();
    db.insert_symbol_ref(c3, noise_id, "call", "low").unwrap();

    let result = handle_stats(&db, None).unwrap();
    // real_fn should appear in Most Referenced (high confidence)
    // noise_fn should NOT (only low confidence refs)
    assert!(
        result.contains("real_fn"),
        "high-confidence refs should appear in most referenced"
    );
    assert!(
        !result.contains("noise_fn"),
        "low-confidence refs should NOT inflate most referenced"
    );
}
```

**Step 2: Run to verify failure**

```bash
cargo test --lib -- tools::stats::tests::test_stats_most_referenced_uses_high_confidence
```

**Step 3: Fix**

In `src/server/tools/stats.rs:90`, change:

```rust
// Before:
let most_ref = db.get_most_referenced_symbols(5, prefix, None)?;

// After:
let most_ref = db.get_most_referenced_symbols(5, prefix, Some("high"))?;
```

Also fix the display to show qualified names (line 94). The DB returns `(name, file, count)` — we need the `impl_type` too. Either:
- Add a new DB method that returns impl_type, OR
- Look up the symbol by name+file to get impl_type

Simplest: modify `get_most_referenced_symbols` return type to include `Option<String>` for impl_type, or add a post-lookup. The minimal change is a post-lookup in stats.rs formatting:

```rust
for (name, file, count) in &most_ref {
    // Qualify name if it has an impl_type
    let display = if let Ok(Some(sym)) = db.get_symbol_by_name_and_file(name, file) {
        if let Some(it) = &sym.impl_type {
            format!("{it}::{name}")
        } else {
            name.clone()
        }
    } else {
        name.clone()
    };
    let _ = writeln!(output, "- **{display}** ({file}) — {count} refs");
}
```

Check if `get_symbol_by_name_and_file` exists or if we need a simpler lookup. If not, add a lightweight method to `db.rs`.

**Step 4: Run tests**

```bash
cargo test --lib -- tools::stats::tests
```

**Step 5: Commit**

```bash
git add src/db.rs src/server/tools/stats.rs
git commit -m "fix: stats uses high-confidence refs, shows qualified names"
```

---

## Task 4: Overview breadth-first distribution across files

When `limit` is set, overview fills it linearly — one large file eats the budget. For agent orientation, we want breadth: at least 1 symbol per file, then distribute remaining budget proportionally.

**Files:**
- Modify: `src/server/tools/overview.rs:28-73`
- Test: `src/server/tools/overview.rs`

**Step 1: Write failing test**

```rust
#[test]
fn test_overview_limit_distributes_across_files() {
    let db = Database::open_in_memory().unwrap();

    // File A: 20 symbols, File B: 5 symbols
    let file_a = db.insert_file("src/big.rs", "h1").unwrap();
    let file_b = db.insert_file("src/small.rs", "h2").unwrap();

    let big_syms: Vec<_> = (0..20)
        .map(|i| Symbol {
            name: format!("big_fn_{i}"),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/big.rs".into(),
            line_start: i * 10 + 1,
            line_end: i * 10 + 5,
            signature: format!("pub fn big_fn_{i}()"),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        })
        .collect();
    store_symbols(&db, file_a, &big_syms).unwrap();

    let small_syms: Vec<_> = (0..5)
        .map(|i| Symbol {
            name: format!("small_fn_{i}"),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/small.rs".into(),
            line_start: i * 10 + 1,
            line_end: i * 10 + 5,
            signature: format!("pub fn small_fn_{i}()"),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        })
        .collect();
    store_symbols(&db, file_b, &small_syms).unwrap();

    // With limit=10, both files should have symbols shown
    let result = handle_overview(&db, "src/", false, Some(10)).unwrap();
    assert!(
        result.contains("### src/big.rs"),
        "big file should appear"
    );
    assert!(
        result.contains("### src/small.rs"),
        "small file should also appear with breadth-first distribution"
    );
    // small.rs should have at least 1 symbol
    assert!(
        result.contains("small_fn_"),
        "small file should have at least one symbol shown"
    );
}
```

**Step 2: Run to verify failure**

```bash
cargo test --lib -- tools::overview::tests::test_overview_limit_distributes_across_files
```

Expected: FAIL — limit=10 shows only 10 symbols from `src/big.rs`, nothing from `src/small.rs`.

**Step 3: Implement breadth-first distribution**

Replace the linear iteration in `handle_overview` with a two-pass approach:

1. Group symbols by file
2. If limit is set: round-robin distribute — first pass gives each file 1 symbol, second pass distributes remaining proportionally
3. Render in file order

```rust
// After getting symbols and before rendering:
let max_symbols = limit.map(|l| usize::try_from(l.max(1)).unwrap_or(usize::MAX));

// Group symbols by file, preserving order within each file
let mut by_file: Vec<(&str, Vec<&StoredSymbol>)> = Vec::new();
let mut current_file = "";
for sym in &symbols {
    if sym.kind == SymbolKind::EnumVariant {
        continue;
    }
    if sym.file_path != current_file {
        current_file = &sym.file_path;
        by_file.push((current_file, Vec::new()));
    }
    if let Some(last) = by_file.last_mut() {
        last.1.push(sym);
    }
}

// Determine per-file budget
let selected: Vec<(&str, Vec<&StoredSymbol>)> = if let Some(max) = max_symbols {
    let file_count = by_file.len();
    if file_count == 0 {
        Vec::new()
    } else {
        // Each file gets at least 1, then distribute remaining proportionally
        let base = 1.min(max / file_count.max(1));
        let mut remaining = max.saturating_sub(file_count * base);
        let total_extra: usize = by_file.iter().map(|(_, s)| s.len().saturating_sub(base)).sum();
        by_file.into_iter().map(|(file, syms)| {
            let extra = if total_extra > 0 && remaining > 0 {
                let fair = (syms.len().saturating_sub(base) * remaining) / total_extra.max(1);
                let take = fair.min(remaining).min(syms.len().saturating_sub(base));
                remaining = remaining.saturating_sub(take);
                take
            } else {
                0
            };
            let count = (base + extra).min(syms.len());
            (file, syms.into_iter().take(count).collect())
        }).collect()
    }
} else {
    by_file
};

// Render from selected
```

This ensures every file with symbols gets at least 1 entry when `limit` is set.

**Step 4: Run tests**

```bash
cargo test --lib -- tools::overview::tests
```

All existing tests plus the new one should pass.

**Step 5: Commit**

```bash
git add src/server/tools/overview.rs
git commit -m "fix: overview distributes limit across files breadth-first

Previously limit=30 could be consumed by one large file.
Now each file gets at least 1 symbol, remaining budget
distributed proportionally."
```

---

## Task 5: Context prefers exact match over fuzzy

When asking for `index_repo` context, it also returns `open_or_index` and a `use` statement. Root cause: `resolve_symbol` always uses FTS-based `search_symbols` which returns partial matches.

**Files:**
- Modify: `src/server/tools/mod.rs:39-50` (`resolve_symbol`)
- Modify: `src/db.rs` (add exact match method if needed)
- Test: `src/server/tools/context.rs`

**Step 1: Write failing test**

Add to `src/server/tools/context.rs` tests:

```rust
#[test]
fn test_context_exact_match_preferred() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "index_repo".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn index_repo()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "open_or_index".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 20,
                signature: "pub fn open_or_index()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let result = handle_context(&db, "index_repo", false, None, None, None).unwrap();
    assert!(
        result.contains("index_repo"),
        "should find exact match"
    );
    assert!(
        !result.contains("open_or_index"),
        "should NOT return fuzzy matches when exact match exists"
    );
}
```

**Step 2: Run to verify failure**

```bash
cargo test --lib -- tools::context::tests::test_context_exact_match_preferred
```

**Step 3: Fix resolve_symbol**

In `src/server/tools/mod.rs`, modify `resolve_symbol` to try exact name match first:

```rust
pub(crate) fn resolve_symbol(
    db: &Database,
    name: &str,
) -> Result<Vec<StoredSymbol>, Box<dyn std::error::Error>> {
    // 1. Try Type::method qualified lookup
    if let Some((impl_type, method)) = name.split_once("::") {
        let results = db.search_symbols_by_impl(impl_type, method)?;
        if !results.is_empty() {
            return Ok(results);
        }
    }

    // 2. Try exact name match
    let exact = db.search_symbols_exact(name)?;
    if !exact.is_empty() {
        return Ok(exact);
    }

    // 3. Fall back to FTS/fuzzy
    Ok(db.search_symbols(name)?)
}
```

Add `search_symbols_exact` to `src/db.rs`:

```rust
pub fn search_symbols_exact(&self, name: &str) -> SqlResult<Vec<StoredSymbol>> {
    let mut stmt = self.conn.prepare_cached(
        "SELECT s.id, s.name, s.kind, s.visibility, f.path, \
                s.line_start, s.line_end, s.signature, s.doc_comment, \
                s.body, s.details, s.attributes, s.impl_type \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         WHERE s.name = ?1 \
         ORDER BY f.path, s.line_start"
    )?;
    // ... row mapping same as search_symbols
}
```

**Step 4: Run tests**

```bash
cargo test --lib -- tools::context::tests
cargo test --lib -- tools::impact::tests
```

**Step 5: Commit**

```bash
git add src/db.rs src/server/tools/mod.rs
git commit -m "fix: resolve_symbol prefers exact name match over FTS

Prevents context/impact from returning fuzzy matches when
an exact symbol name exists."
```

---

## Task 6: Query default scope to "symbols" instead of "all"

When searching for code symbols (the most common use case), docs results are noise. The default `scope: "all"` runs both symbol and docs search. Change to `"symbols"` — users who want docs can explicitly use `scope: "docs"` or `scope: "all"`.

**Files:**
- Modify: `src/server/tools/query.rs:18`
- Modify: `src/server/mod.rs` (tool description for query)
- Test: `src/server/tools/query.rs`

**Step 1: Write failing test**

Add to query.rs tests (or modify existing):

```rust
#[test]
fn test_query_default_scope_is_symbols() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[Symbol {
            name: "parse".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub fn parse()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }],
    )
    .unwrap();

    // Add a doc that matches "parse"
    let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
    db.store_doc(dep_id, "docs.rs", "parse and serialize data").unwrap();

    // Default scope should NOT include docs
    let result = handle_query(&db, "parse", None, None, None, None, None, None).unwrap();
    assert!(result.contains("## Symbols"), "should have symbols section");
    assert!(
        !result.contains("## Documentation"),
        "default scope should not include docs"
    );

    // Explicit "all" scope should include both
    let result = handle_query(&db, "parse", Some("all"), None, None, None, None, None).unwrap();
    assert!(result.contains("## Symbols"));
    assert!(result.contains("## Documentation"));
}
```

**Step 2: Run to verify failure**

```bash
cargo test --lib -- tools::query::tests::test_query_default_scope_is_symbols
```

**Step 3: Fix**

In `src/server/tools/query.rs:18`, change:

```rust
// Before:
let scope = scope.unwrap_or("all");

// After:
let scope = scope.unwrap_or("symbols");
```

Update the tool description in `src/server/mod.rs` for the query tool to document the new default:

Find the `#[tool(description = "...")]` for query and update to mention `scope` defaults to `"symbols"`. Also update the JsonSchema description for the scope field.

**Step 4: Run tests**

```bash
cargo test --lib -- tools::query::tests
```

**Step 5: Run full test suite + clippy**

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

**Step 6: Commit**

```bash
git add src/server/tools/query.rs src/server/mod.rs
git commit -m "fix: query defaults to 'symbols' scope instead of 'all'

Symbol search is the most common use case. Docs results
are noise when looking for code. Use scope='all' or
scope='docs' to explicitly include dependency docs."
```

---

## Final: Update CLAUDE.md

After all tasks, update `CLAUDE.md` to document:
- `impact` and `test_impact` now work with `Type::method` syntax
- `query` default scope changed from `all` to `symbols`
- `overview` now distributes `limit` across files breadth-first

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md with behavioral changes from six fixes"
```
