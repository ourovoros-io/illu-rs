# Evaluation Improvements Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the wildcard+kind query bug, add relevance-ranked search results, filter noisy callees from similarity scoring, and add LIKE fallback for docs topic search.

**Architecture:** All changes are in the server tools layer (`src/server/tools/`) with one new DB helper method (`count_refs_for_symbol`) and one new DB query method (`search_docs_content`). No schema changes.

**Tech Stack:** Rust, rusqlite, tree-sitter (existing)

---

### Task 1: Fix `query * kind=X` bug (wildcard + kind-only returns nothing)

**Files:**
- Modify: `src/server/tools/query.rs:70-143` (format_symbols)
- Test: `src/server/tools/query.rs` (tests module)

**Problem:** In `format_symbols`, when `query="*"` (wildcard) and only `kind` is set (no attribute, signature, or path), the if-else chain falls through to `return Ok(())`. The `kind` filter is a post-filter (`.retain()`/`.filter()`) and never seeds results.

**Fix:** Before the `else { return Ok(()); }` fallthrough, add a branch: when `kind` is set, seed with all symbols via `db.get_symbols_by_path_prefix("")` (empty prefix matches all paths).

**Step 1: Write the failing test**

```rust
#[test]
fn test_query_wildcard_kind_only() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "MyStruct".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct MyStruct".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "my_func".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn my_func()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    // Wildcard + kind=struct should return only structs
    let result = handle_query(
        &db, "*", Some("symbols"), Some("struct"), None, None, None, None,
    )
    .unwrap();
    assert!(result.contains("MyStruct"), "Should find struct: {result}");
    assert!(!result.contains("my_func"), "Should NOT find function: {result}");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- test_query_wildcard_kind_only`
Expected: FAIL — returns "No results found"

**Step 3: Implement the fix**

In `format_symbols` (query.rs), change the if-else chain that seeds `all_symbols`. Currently:

```rust
let mut all_symbols = if let Some(attr) = attribute {
    db.search_symbols_by_attribute(attr)?
} else if !is_wildcard {
    db.search_symbols(query)?
} else if let Some(sig) = signature {
    db.search_symbols_by_signature(sig)?
} else if let Some(p) = path {
    db.get_symbols_by_path_prefix(p)?
} else {
    return Ok(());
};
```

Change to:

```rust
let mut all_symbols = if let Some(attr) = attribute {
    db.search_symbols_by_attribute(attr)?
} else if !is_wildcard {
    db.search_symbols(query)?
} else if let Some(sig) = signature {
    db.search_symbols_by_signature(sig)?
} else if let Some(p) = path {
    db.get_symbols_by_path_prefix(p)?
} else if kind.is_some() {
    db.get_symbols_by_path_prefix("")?
} else {
    return Ok(());
};
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib -- test_query_wildcard_kind_only`
Expected: PASS

**Step 5: Run full query test suite**

Run: `cargo test --lib -- query::tests`
Expected: All pass

**Step 6: Commit**

```bash
git add src/server/tools/query.rs
git commit -m "fix: support wildcard query with kind-only filter"
```

---

### Task 2: Relevance-ranked search results (sort by reference count)

**Files:**
- Modify: `src/db.rs` (add `count_refs_for_symbol` method)
- Modify: `src/server/tools/query.rs` (add sort in `format_symbols`)
- Test: `src/server/tools/query.rs` (tests module)

**Problem:** Query results are sorted alphabetically. When searching "parse", a function called 49 times has the same weight as one called once. Most-referenced symbols should appear first.

**Step 1: Add DB method — write the test**

```rust
// In src/db.rs tests module
#[test]
fn test_count_refs_for_symbol() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "target_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn target_fn()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "caller_a".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn caller_a()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "caller_b".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 15,
                signature: "pub fn caller_b()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let map = SymbolIdMap::build(&db).unwrap();
    db.store_symbol_refs_fast(
        &[
            SymbolRef {
                source_name: "caller_a".into(),
                target_name: "target_fn".into(),
                source_file: "src/lib.rs".into(),
                target_file: Some("src/lib.rs".into()),
                kind: RefKind::Call,
                confidence: "high".into(),
                ref_line: Some(8),
            },
            SymbolRef {
                source_name: "caller_b".into(),
                target_name: "target_fn".into(),
                source_file: "src/lib.rs".into(),
                target_file: Some("src/lib.rs".into()),
                kind: RefKind::Call,
                confidence: "high".into(),
                ref_line: Some(13),
            },
        ],
        &map,
    )
    .unwrap();

    let count = db
        .count_refs_for_symbol("target_fn", "src/lib.rs")
        .unwrap();
    assert_eq!(count, 2);

    let count = db
        .count_refs_for_symbol("caller_a", "src/lib.rs")
        .unwrap();
    assert_eq!(count, 0);
}
```

**Step 2: Implement DB method**

Add to `impl Database` in `src/db.rs`:

```rust
pub fn count_refs_for_symbol(
    &self,
    name: &str,
    file_path: &str,
) -> SqlResult<i64> {
    let pattern = format!("{}%", escape_like(file_path));
    self.conn.query_row(
        "SELECT COUNT(DISTINCT sr.source_symbol_id) \
         FROM symbols ts \
         JOIN files f ON f.id = ts.file_id \
         JOIN symbol_refs sr ON sr.target_symbol_id = ts.id \
         WHERE ts.name = ?1 AND f.path LIKE ?2 ESCAPE '\\' \
           AND sr.confidence = 'high'",
        params![name, pattern],
        |row| row.get(0),
    )
}
```

Note: Uses `LIKE ?2` with exact path to handle the match. Actually simpler to use `= ?2`:

```rust
self.conn.query_row(
    "SELECT COUNT(DISTINCT sr.source_symbol_id) \
     FROM symbols ts \
     JOIN files f ON f.id = ts.file_id \
     JOIN symbol_refs sr ON sr.target_symbol_id = ts.id \
     WHERE ts.name = ?1 AND f.path = ?2 \
       AND sr.confidence = 'high'",
    params![name, file_path],
    |row| row.get(0),
)
```

**Step 3: Run DB test**

Run: `cargo test --lib -- test_count_refs_for_symbol`
Expected: PASS

**Step 4: Write the query ranking test**

```rust
#[test]
fn test_query_results_sorted_by_ref_count() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "parse_alpha".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn parse_alpha()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "parse_beta".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn parse_beta()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "caller1".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 15,
                signature: "pub fn caller1()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "caller2".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 17,
                line_end: 20,
                signature: "pub fn caller2()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "caller3".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 22,
                line_end: 25,
                signature: "pub fn caller3()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let map = SymbolIdMap::build(&db).unwrap();
    // parse_beta gets 3 refs, parse_alpha gets 1
    db.store_symbol_refs_fast(
        &[
            SymbolRef {
                source_name: "caller1".into(),
                target_name: "parse_beta".into(),
                source_file: "src/lib.rs".into(),
                target_file: Some("src/lib.rs".into()),
                kind: RefKind::Call,
                confidence: "high".into(),
                ref_line: Some(13),
            },
            SymbolRef {
                source_name: "caller2".into(),
                target_name: "parse_beta".into(),
                source_file: "src/lib.rs".into(),
                target_file: Some("src/lib.rs".into()),
                kind: RefKind::Call,
                confidence: "high".into(),
                ref_line: Some(18),
            },
            SymbolRef {
                source_name: "caller3".into(),
                target_name: "parse_beta".into(),
                source_file: "src/lib.rs".into(),
                target_file: Some("src/lib.rs".into()),
                kind: RefKind::Call,
                confidence: "high".into(),
                ref_line: Some(23),
            },
            SymbolRef {
                source_name: "caller1".into(),
                target_name: "parse_alpha".into(),
                source_file: "src/lib.rs".into(),
                target_file: Some("src/lib.rs".into()),
                kind: RefKind::Call,
                confidence: "high".into(),
                ref_line: Some(14),
            },
        ],
        &map,
    )
    .unwrap();

    let result = handle_query(
        &db, "parse", Some("symbols"), None, None, None, None, None,
    )
    .unwrap();
    // parse_beta (3 refs) should appear before parse_alpha (1 ref)
    let beta_pos = result.find("parse_beta").expect("should contain parse_beta");
    let alpha_pos = result.find("parse_alpha").expect("should contain parse_alpha");
    assert!(
        beta_pos < alpha_pos,
        "parse_beta (3 refs) should appear before parse_alpha (1 ref)\n{result}"
    );
}
```

**Step 5: Run test to verify it fails**

Run: `cargo test --lib -- test_query_results_sorted_by_ref_count`
Expected: FAIL — parse_alpha appears before parse_beta (alphabetical order)

**Step 6: Implement the sort in format_symbols**

Add helper function in `src/server/tools/query.rs`:

```rust
fn sort_by_ref_count(
    db: &Database,
    symbols: &mut Vec<StoredSymbol>,
) -> Result<(), Box<dyn std::error::Error>> {
    if symbols.len() <= 1 {
        return Ok(());
    }
    let mut with_counts: Vec<(i64, StoredSymbol)> =
        Vec::with_capacity(symbols.len());
    for sym in symbols.drain(..) {
        let count =
            db.count_refs_for_symbol(&sym.name, &sym.file_path)?;
        with_counts.push((count, sym));
    }
    with_counts.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.name.cmp(&b.1.name))
    });
    symbols.extend(with_counts.into_iter().map(|(_, s)| s));
    Ok(())
}
```

Call it in `format_symbols` after all filters and before the `limit` truncation:

```rust
// After the kind filter block, before the limit block:
sort_by_ref_count(db, &mut symbols)?;
```

**Step 7: Run test to verify it passes**

Run: `cargo test --lib -- test_query_results_sorted_by_ref_count`
Expected: PASS

**Step 8: Run full query test suite**

Run: `cargo test --lib -- query::tests`
Expected: All pass (existing tests don't depend on alphabetical order within results)

**Step 9: Commit**

```bash
git add src/db.rs src/server/tools/query.rs
git commit -m "feat: sort query results by reference count for relevance ranking"
```

---

### Task 3: Filter noisy callees from similarity scoring

**Files:**
- Modify: `src/server/tools/similar.rs` (score_one function)
- Test: `src/server/tools/similar.rs` (tests module)

**Problem:** `similar` scoring counts shared callees like `new`, `from`, `default` which almost every function calls. These inflate scores and produce false-positive similarity matches.

**Step 1: Write the failing test**

```rust
#[test]
fn test_similar_excludes_noisy_callees() {
    let db = Database::open_in_memory().unwrap();
    let f1 = db.insert_file("src/lib.rs", "hash1").unwrap();
    let f2 = db.insert_file("src/util.rs", "hash2").unwrap();

    store_symbols(
        &db,
        f1,
        &[
            Symbol {
                name: "build_report".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn build_report() -> String".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    store_symbols(
        &db,
        f2,
        &[
            Symbol {
                name: "build_summary".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/util.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn build_summary() -> String".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            // A constructor that both call — should be noise
            Symbol {
                name: "new".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/util.rs".into(),
                line_start: 12,
                line_end: 15,
                signature: "pub fn new() -> Self".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let map = SymbolIdMap::build(&db).unwrap();
    db.store_symbol_refs_fast(
        &[
            SymbolRef {
                source_name: "build_report".into(),
                target_name: "new".into(),
                source_file: "src/lib.rs".into(),
                target_file: Some("src/util.rs".into()),
                kind: RefKind::Call,
                confidence: "high".into(),
                ref_line: Some(3),
            },
            SymbolRef {
                source_name: "build_summary".into(),
                target_name: "new".into(),
                source_file: "src/util.rs".into(),
                target_file: Some("src/util.rs".into()),
                kind: RefKind::Call,
                confidence: "high".into(),
                ref_line: Some(3),
            },
        ],
        &map,
    )
    .unwrap();

    let result = handle_similar(&db, "build_report", None).unwrap();
    // Should find build_summary via return type match, but NOT via shared callee "new"
    assert!(result.contains("build_summary"));
    assert!(
        !result.contains("shared callees: new"),
        "Should not count 'new' as meaningful shared callee\n{result}"
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- test_similar_excludes_noisy_callees`
Expected: FAIL — output contains "shared callees: new"

**Step 3: Implement the fix**

Add constant at the top of `similar.rs`:

```rust
const NOISY_SIMILAR_CALLEES: &[&str] = &[
    "new", "from", "into", "default", "clone", "build", "init",
    "fmt", "write", "writeln", "push", "len", "is_empty",
    "to_string", "to_owned", "as_str", "as_ref",
    "iter", "collect", "map", "filter",
];
```

In `score_one`, filter the shared callees:

```rust
// Replace the existing shared callees block:
if !target_callees.is_empty() {
    let cand_callees: HashSet<String> = db
        .get_callees_by_name(cand_name, None, false)?
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let shared: Vec<_> = target_callees
        .intersection(&cand_callees)
        .filter(|name| {
            !NOISY_SIMILAR_CALLEES.contains(&name.as_str())
        })
        .collect();
    if !shared.is_empty() {
        score += shared.len();
        let names: Vec<&str> =
            shared.iter().take(3).map(|s| s.as_str()).collect();
        reasons.push(format!(
            "shared callees: {}",
            names.join(", ")
        ));
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib -- test_similar_excludes_noisy_callees`
Expected: PASS

**Step 5: Run full similar test suite**

Run: `cargo test --lib -- similar::tests`
Expected: All pass

**Step 6: Commit**

```bash
git add src/server/tools/similar.rs
git commit -m "fix: exclude noisy callees from similarity scoring"
```

---

### Task 4: Add LIKE fallback for docs topic search

**Files:**
- Modify: `src/db.rs` (add `search_docs_content` method)
- Modify: `src/server/tools/docs.rs` (add LIKE fallback in `handle_docs_with_topic`)
- Test: `src/server/tools/docs.rs` (tests module)

**Problem:** Docs topic search uses FTS5 only. Terms like "FTS5" fail because FTS tokenization splits on digits or doesn't match the token boundaries. A LIKE fallback catches these cases.

**Step 1: Write the failing test**

```rust
#[test]
fn test_docs_topic_like_fallback() {
    let db = Database::open_in_memory().unwrap();
    let dep_id = db
        .insert_dependency("rusqlite", "0.31.0", true, None)
        .unwrap();
    db.store_doc_with_module(
        dep_id,
        "docs.rs",
        "rusqlite supports FTS5 full-text search via virtual tables",
        "features",
    )
    .unwrap();

    // "FTS5" may not match via FTS tokenization, but LIKE fallback should find it
    let result = handle_docs(&db, "rusqlite", Some("FTS5")).unwrap();
    assert!(
        result.contains("FTS5"),
        "LIKE fallback should find FTS5 in doc content\n{result}"
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- test_docs_topic_like_fallback`
Expected: FAIL — returns "no docs match topic 'FTS5'"

**Step 3: Add DB method**

Add to `impl Database` in `src/db.rs`:

```rust
pub fn search_docs_content(
    &self,
    dep_name: &str,
    query: &str,
) -> SqlResult<Vec<DocResult>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let pattern = format!("%{}%", escape_like(query));
    let mut stmt = self.conn.prepare(
        "SELECT d.content, d.source, dep.name, dep.version, d.module \
         FROM docs d \
         JOIN dependencies dep ON dep.id = d.dependency_id \
         WHERE dep.name = ?1 AND d.content LIKE ?2 ESCAPE '\\' \
         LIMIT 10",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query(params![dep_name, pattern])?;
    while let Some(row) = rows.next()? {
        results.push(row_to_doc_result(row)?);
    }
    Ok(results)
}
```

**Step 4: Add LIKE fallback in `handle_docs_with_topic`**

In `src/server/tools/docs.rs`, in `handle_docs_with_topic`, after the FTS search and `filtered` check, add a LIKE fallback before returning "no docs match":

```rust
// After: let filtered: Vec<_> = results.iter()...
// Replace the `if filtered.is_empty() {` block:

if filtered.is_empty() {
    // LIKE fallback for terms FTS can't tokenize (e.g. "FTS5")
    let like_results = db.search_docs_content(dep_name, topic)?;
    if !like_results.is_empty() {
        let mut output = String::new();
        let _ = writeln!(output, "## {dep_name} — {topic}\n");
        for doc in &like_results {
            let _ = writeln!(
                output,
                "### {} ({})\n\n{}\n",
                doc.dependency_name, doc.source, doc.content
            );
        }
        return Ok(output);
    }

    let dep = db.get_dependency_by_name(dep_name)?;
    // ... rest of existing error handling unchanged
}
```

**Step 5: Run test to verify it passes**

Run: `cargo test --lib -- test_docs_topic_like_fallback`
Expected: PASS

**Step 6: Run full docs test suite**

Run: `cargo test --lib -- docs::tests`
Expected: All pass

**Step 7: Commit**

```bash
git add src/db.rs src/server/tools/docs.rs
git commit -m "feat: add LIKE fallback for docs topic search"
```

---

### Task 5: Final verification

**Step 1: Run full test suite**

Run: `cargo test`
Expected: All pass

**Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings

**Step 3: Run fmt check**

Run: `cargo fmt --all -- --check`
Expected: No formatting issues

**Step 4: Update CLAUDE.md**

Add documentation for new behaviors:
- `query` wildcard+kind behavior
- Relevance ranking in query results
- Similar noisy callee filtering
- Docs LIKE fallback

**Step 5: Final commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md with evaluation improvements"
```
