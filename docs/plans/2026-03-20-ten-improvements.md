# Ten illu-rs Improvements Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add 10 missing features to illu-rs that improve daily agent effectiveness: scoped queries, selective context, trait browsing, neighborhood graphs, combinable filters, doc comment search, type usage analysis, file import graph, lightweight diff changes, and multi-path callpath.

**Architecture:** Each feature is either a new MCP tool or an enhancement to an existing one. All changes flow through the same pattern: add/modify DB query in `src/db.rs`, add/modify handler in `src/server/tools/*.rs`, wire param struct + MCP endpoint in `src/server/mod.rs`. No new tables are needed except for Task 10 (file imports).

**Tech Stack:** Rust 2024 edition, rusqlite (bundled+fts5), rmcp, tree-sitter, schemars

---

## Task 1: Add `path` filter to `query` tool

Allows scoping symbol search to a file or directory prefix: `query "get" --path src/db.rs`

**Files:**
- Modify: `src/db.rs` — new `search_symbols_scoped` method
- Modify: `src/server/tools/query.rs` — pass path to DB query
- Modify: `src/server/mod.rs:75-83` — add `path` field to `QueryParams`

**Step 1: Write the failing test**

In `src/server/tools/query.rs`, add to the test module:

```rust
#[test]
fn test_query_path_filter() {
    let db = Database::open_in_memory().unwrap();
    let f1 = db.insert_file("src/db.rs", "h1").unwrap();
    let f2 = db.insert_file("src/server/mod.rs", "h2").unwrap();
    store_symbols(
        &db,
        f1,
        &[Symbol {
            name: "get_symbol".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/db.rs".into(),
            line_start: 1,
            line_end: 10,
            signature: "pub fn get_symbol()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }],
    )
    .unwrap();
    store_symbols(
        &db,
        f2,
        &[Symbol {
            name: "get_info".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/server/mod.rs".into(),
            line_start: 1,
            line_end: 10,
            signature: "pub fn get_info()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }],
    )
    .unwrap();

    // With path filter: only src/db.rs results
    let result = handle_query(
        &db, "get", Some("symbols"), None, None, None, Some("src/db.rs"),
    )
    .unwrap();
    assert!(result.contains("get_symbol"));
    assert!(!result.contains("get_info"));

    // Without path filter: both
    let result = handle_query(&db, "get", Some("symbols"), None, None, None, None).unwrap();
    assert!(result.contains("get_symbol"));
    assert!(result.contains("get_info"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- query::tests::test_query_path_filter`
Expected: Compile error — `handle_query` doesn't accept a 7th parameter yet.

**Step 3: Add `path` parameter to `QueryParams`**

In `src/server/mod.rs`, add to `QueryParams`:

```rust
/// Filter to files under this path prefix (e.g. "src/db.rs", "src/server/")
path: Option<String>,
```

Pass it through in the `query` method:

```rust
let result = tools::query::handle_query(
    &db,
    &params.query,
    params.scope.as_deref(),
    params.kind.as_deref(),
    params.attribute.as_deref(),
    params.signature.as_deref(),
    params.path.as_deref(),
)
```

**Step 4: Add `path` parameter to `handle_query` and `format_symbols`**

In `src/server/tools/query.rs`:

```rust
pub fn handle_query(
    db: &Database,
    query: &str,
    scope: Option<&str>,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    path: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    // ... pass path to format_symbols
}

fn format_symbols(
    db: &Database,
    query: &str,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    path: Option<&str>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let all_symbols = /* existing logic */;

    // Apply path filter AFTER fetching
    let all_symbols = if let Some(p) = path {
        all_symbols.into_iter().filter(|s| s.file_path.starts_with(p)).collect()
    } else {
        all_symbols
    };
    // ... rest unchanged
}
```

Post-filter approach is simplest since `search_symbols`, `search_symbols_by_attribute`, and `search_symbols_by_signature` all return `Vec<StoredSymbol>` with `file_path` already populated. Adding path to every DB query would duplicate logic across 3 branches.

**Step 5: Fix all existing callers of `handle_query`**

Update any callers passing the old signature. Existing tests pass `None` for the new `path` param. Update `src/main.rs` if it calls `handle_query` directly.

**Step 6: Run tests to verify pass**

Run: `cargo test --lib -- query::tests`
Expected: All pass including `test_query_path_filter`.

**Step 7: Commit**

```bash
git add src/server/mod.rs src/server/tools/query.rs
git commit -m "feat: add path filter to query tool"
```

---

## Task 2: Make `query` filters combinable (attribute + signature)

Currently `attribute` and `signature` are in an if/else chain — you can't combine them. Fix: apply all filters additively via post-filtering.

**Files:**
- Modify: `src/server/tools/query.rs:39-98` — refactor `format_symbols` filter logic

**Step 1: Write the failing test**

```rust
#[test]
fn test_query_combined_attribute_and_signature() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "test_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn test_fn(db: &Database)".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: Some("test".into()),
                impl_type: None,
            },
            Symbol {
                name: "other_test".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn other_test()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: Some("test".into()),
                impl_type: None,
            },
        ],
    )
    .unwrap();

    // Combined: attribute=test AND signature contains Database
    let result = handle_query(
        &db, "", Some("symbols"), None,
        Some("test"), Some("Database"), None,
    )
    .unwrap();
    assert!(result.contains("test_fn"), "should find fn with both attribute and signature");
    assert!(!result.contains("other_test"), "should exclude fn without matching signature");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- query::tests::test_query_combined_attribute_and_signature`
Expected: FAIL — `other_test` will appear because current code uses if/else (attribute OR signature, not AND).

**Step 3: Refactor `format_symbols` to apply filters additively**

Replace the if/else chain in `format_symbols` with sequential filtering:

```rust
fn format_symbols(
    db: &Database,
    query: &str,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    path: Option<&str>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    // Start with the broadest search
    let mut symbols = if let Some(attr) = attribute {
        db.search_symbols_by_attribute(attr)?
    } else if !query.is_empty() {
        db.search_symbols(query)?
    } else {
        // No query and no attribute — need a base set
        // Use signature search if available, else empty
        if let Some(sig) = signature {
            db.search_symbols_by_signature(sig)?
        } else {
            Vec::new()
        }
    };

    // Apply name filter if we started from attribute/signature
    if attribute.is_some() || (query.is_empty() && signature.is_some()) {
        if !query.is_empty() {
            let q = query.to_lowercase();
            symbols.retain(|s| s.name.to_lowercase().contains(&q));
        }
    }

    // Apply signature filter (if we didn't start from it)
    if let Some(sig) = signature {
        if attribute.is_some() || !query.is_empty() {
            let sig_lower = sig.to_lowercase();
            symbols.retain(|s| s.signature.to_lowercase().contains(&sig_lower));
        }
    }

    // Apply path filter
    if let Some(p) = path {
        symbols.retain(|s| s.file_path.starts_with(p));
    }

    // Apply kind filter
    if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        symbols.retain(|s| s.kind.to_string().to_lowercase() == k_lower);
    } else {
        symbols.retain(|s| {
            s.kind != crate::indexer::parser::SymbolKind::Use
                && s.kind != crate::indexer::parser::SymbolKind::Mod
                && s.kind != crate::indexer::parser::SymbolKind::EnumVariant
        });
    }

    // Render
    if !symbols.is_empty() {
        output.push_str("## Symbols\n\n");
        for sym in &symbols {
            let qname = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qname}** ({}) at {}:{}-{}\n  `{}`",
                sym.kind, sym.file_path, sym.line_start, sym.line_end, sym.signature,
            );
            if let Some(doc) = &sym.doc_comment
                && let Some(first_line) = doc.lines().next()
            {
                let _ = writeln!(output, "  *{first_line}*");
            }
        }
        output.push('\n');
    }
    Ok(())
}
```

**Step 4: Run tests to verify pass**

Run: `cargo test --lib -- query::tests`
Expected: All pass.

**Step 5: Commit**

```bash
git add src/server/tools/query.rs
git commit -m "feat: make query attribute+signature filters combinable"
```

---

## Task 3: Add doc comment search for internal symbols

Add `scope: "doc_comments"` to query, which searches symbol doc comments using LIKE. Also consider adding a `doc` parameter for filtering by doc content alongside name search.

**Files:**
- Modify: `src/db.rs` — new `search_symbols_by_doc_comment` method
- Modify: `src/server/tools/query.rs` — add `doc_comments` scope

**Step 1: Write the failing test for DB method**

In `src/db.rs` test module:

```rust
#[test]
fn test_search_by_doc_comment() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "safe_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn safe_fn()".into(),
                doc_comment: Some("Thread-safe implementation.".into()),
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "other_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn other_fn()".into(),
                doc_comment: Some("Does something else.".into()),
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let results = db.search_symbols_by_doc_comment("thread").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "safe_fn");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- db::tests::test_search_by_doc_comment`
Expected: Compile error — `search_symbols_by_doc_comment` doesn't exist.

**Step 3: Implement `search_symbols_by_doc_comment` in `src/db.rs`**

```rust
pub fn search_symbols_by_doc_comment(
    &self,
    query: &str,
) -> SqlResult<Vec<StoredSymbol>> {
    let pattern = format!("%{}%", escape_like(query));
    let mut stmt = self.conn.prepare(
        "SELECT s.name, s.kind, s.visibility, f.path, \
                s.line_start, s.line_end, s.signature, \
                s.doc_comment, s.body, s.details, s.attributes, s.impl_type \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         WHERE s.doc_comment LIKE ?1 ESCAPE '\\' \
         ORDER BY s.name \
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

**Step 4: Run DB test to verify pass**

Run: `cargo test --lib -- db::tests::test_search_by_doc_comment`
Expected: PASS

**Step 5: Write query tool test for doc_comments scope**

```rust
#[test]
fn test_query_doc_comments_scope() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[Symbol {
            name: "my_fn".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub fn my_fn()".into(),
            doc_comment: Some("Handles thread safety.".into()),
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }],
    )
    .unwrap();

    let result = handle_query(
        &db, "thread", Some("doc_comments"), None, None, None, None,
    )
    .unwrap();
    assert!(result.contains("my_fn"));
}
```

**Step 6: Add `doc_comments` scope to `handle_query`**

In the `match scope` block in `handle_query`, add:

```rust
"doc_comments" => format_doc_comments(db, query, kind, path, &mut output)?,
```

Implement `format_doc_comments`:

```rust
fn format_doc_comments(
    db: &Database,
    query: &str,
    kind: Option<&str>,
    path: Option<&str>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut symbols = db.search_symbols_by_doc_comment(query)?;
    if let Some(p) = path {
        symbols.retain(|s| s.file_path.starts_with(p));
    }
    if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        symbols.retain(|s| s.kind.to_string().to_lowercase() == k_lower);
    }
    if !symbols.is_empty() {
        output.push_str("## Symbols (by doc comment)\n\n");
        for sym in &symbols {
            let qname = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qname}** ({}) at {}:{}-{}",
                sym.kind, sym.file_path, sym.line_start, sym.line_end,
            );
            if let Some(doc) = &sym.doc_comment
                && let Some(first_line) = doc.lines().next()
            {
                let _ = writeln!(output, "  *{first_line}*");
            }
        }
        output.push('\n');
    }
    Ok(())
}
```

Also update the error message for unknown scope to include `doc_comments`.

**Step 7: Run tests to verify pass**

Run: `cargo test --lib -- query::tests`
Expected: All pass.

**Step 8: Commit**

```bash
git add src/db.rs src/server/tools/query.rs
git commit -m "feat: add doc_comments scope to query tool"
```

---

## Task 4: Add `sections` filter to `context` tool

Allow requesting specific sections: `source`, `callers`, `callees`, `tested_by`, `traits`, `docs`. Reduces output from 56KB to just what's needed.

**Files:**
- Modify: `src/server/mod.rs:85-92` — add `sections` to `ContextParams`
- Modify: `src/server/tools/context.rs:6-38` — conditional rendering

**Step 1: Write the failing test**

```rust
#[test]
fn test_context_sections_filter() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
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
                line_end: 10,
                signature: "pub fn target_fn()".into(),
                doc_comment: None,
                body: Some("pub fn target_fn() { callee_fn() }".into()),
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "callee_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 20,
                signature: "pub fn callee_fn()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let caller_id = db.get_symbol_id("target_fn", "src/lib.rs").unwrap().unwrap();
    let callee_id = db.get_symbol_id("callee_fn", "src/lib.rs").unwrap().unwrap();
    db.insert_symbol_ref(caller_id, callee_id, "call").unwrap();

    // Request only "source" section — should have source, not callees
    let result = handle_context(
        &db, "target_fn", false, None, Some(&["source"]),
    )
    .unwrap();
    assert!(result.contains("### Source"), "should include source section");
    assert!(!result.contains("### Callees"), "should NOT include callees");

    // Request only "callees" — should have callees, not source
    let result = handle_context(
        &db, "target_fn", false, None, Some(&["callees"]),
    )
    .unwrap();
    assert!(result.contains("### Callees"), "should include callees");
    assert!(!result.contains("### Source"), "should NOT include source");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- context::tests::test_context_sections_filter`
Expected: Compile error — `handle_context` doesn't accept a `sections` parameter.

**Step 3: Add `sections` to `ContextParams` and handler**

In `src/server/mod.rs`, update `ContextParams`:

```rust
#[derive(Deserialize, JsonSchema)]
struct ContextParams {
    symbol_name: String,
    /// Return full untruncated source body (default: false)
    full_body: Option<bool>,
    /// Filter results to a specific file path (e.g. "src/db.rs")
    file: Option<String>,
    /// Select specific sections: source, callers, callees, tested_by, traits, docs.
    /// Omit for all sections (default behavior).
    sections: Option<Vec<String>>,
}
```

Pass through in the `context` method:

```rust
let sections: Option<Vec<&str>> = params.sections.as_ref()
    .map(|v| v.iter().map(String::as_str).collect());
let result = tools::context::handle_context(
    &db,
    &params.symbol_name,
    full_body,
    params.file.as_deref(),
    sections.as_deref(),
)
```

**Step 4: Update `handle_context` to accept and use sections**

```rust
pub fn handle_context(
    db: &Database,
    symbol_name: &str,
    full_body: bool,
    file: Option<&str>,
    sections: Option<&[&str]>,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = resolve_symbols(db, symbol_name, file)?;
    if symbols.is_empty() {
        return Ok(format!(
            "No symbol found matching '{symbol_name}'.\n\
            Try a partial name or use `query` to search."
        ));
    }

    let show = |name: &str| -> bool {
        sections.is_none() || sections.is_some_and(|s| s.contains(&name))
    };

    let repo_root = db.repo_root();
    let mut output = String::new();

    for sym in &symbols {
        // Header always shown
        render_symbol_header(&mut output, sym);
        if show("source") {
            render_symbol_details(&mut output, sym, full_body, repo_root);
        }
        if show("traits") {
            render_trait_info(db, &mut output, sym)?;
        }
        if show("callers") {
            render_callers(db, &mut output, sym)?;
        }
        if show("callees") {
            render_callees(db, &mut output, sym)?;
        }
        if show("tested_by") {
            render_tested_by(db, &mut output, sym)?;
        }
    }

    if show("docs") {
        let base_name = symbol_name
            .split_once("::")
            .map_or(symbol_name, |(_, m)| m);
        render_related_docs(db, &mut output, base_name)?;
    }

    Ok(output)
}
```

**Step 5: Fix all callers**

Update `handle_batch_context` in `batch_context.rs` to pass `None` for sections. Update `src/main.rs` if it calls `handle_context` directly.

**Step 6: Run tests to verify pass**

Run: `cargo test --lib -- context::tests`
Expected: All pass.

**Step 7: Commit**

```bash
git add src/server/mod.rs src/server/tools/context.rs src/server/tools/batch_context.rs
git commit -m "feat: add sections filter to context tool"
```

---

## Task 5: New `implements` tool

Standalone tool for trait/type relationship queries. Uses existing `get_trait_impls_for_type` and `get_trait_impls_for_trait` DB methods.

**Files:**
- Create: `src/server/tools/implements.rs`
- Modify: `src/server/tools/mod.rs:1` — add `pub mod implements;`
- Modify: `src/server/mod.rs` — add `ImplementsParams` struct and MCP endpoint

**Step 1: Write the failing test**

Create `src/server/tools/implements.rs`:

```rust
use crate::db::Database;
use std::fmt::Write;

pub fn handle_implements(
    db: &Database,
    trait_name: Option<&str>,
    type_name: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    todo!()
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    #[test]
    fn test_implements_by_trait() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "Display".into(),
                    kind: SymbolKind::Trait,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 5,
                    signature: "pub trait Display".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "MyStruct".into(),
                    kind: SymbolKind::Struct,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 7,
                    line_end: 10,
                    signature: "pub struct MyStruct".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();
        db.insert_trait_impl("MyStruct", "Display", file_id, 12, 20)
            .unwrap();

        let result = handle_implements(&db, Some("Display"), None).unwrap();
        assert!(result.contains("MyStruct"));
        assert!(result.contains("Display"));
    }

    #[test]
    fn test_implements_by_type() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "Database".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct Database".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();
        db.insert_trait_impl("Database", "Debug", file_id, 10, 15)
            .unwrap();
        db.insert_trait_impl("Database", "Clone", file_id, 17, 22)
            .unwrap();

        let result = handle_implements(&db, None, Some("Database")).unwrap();
        assert!(result.contains("Debug"));
        assert!(result.contains("Clone"));
    }

    #[test]
    fn test_implements_requires_one_param() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_implements(&db, None, None).unwrap();
        assert!(result.contains("Provide"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- implements::tests`
Expected: FAIL — `todo!()` panics.

**Step 3: Implement `handle_implements`**

```rust
pub fn handle_implements(
    db: &Database,
    trait_name: Option<&str>,
    type_name: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();

    match (trait_name, type_name) {
        (Some(t), None) => {
            let impls = db.get_trait_impls_for_trait(t)?;
            let _ = writeln!(output, "## Types implementing `{t}`\n");
            if impls.is_empty() {
                let _ = writeln!(output, "No implementations found for trait `{t}`.");
            } else {
                for ti in &impls {
                    let _ = writeln!(
                        output,
                        "- **{}** ({}:{}-{})",
                        ti.type_name, ti.file_path, ti.line_start, ti.line_end
                    );
                }
            }
        }
        (None, Some(ty)) => {
            let impls = db.get_trait_impls_for_type(ty)?;
            let _ = writeln!(output, "## Traits implemented by `{ty}`\n");
            if impls.is_empty() {
                let _ = writeln!(output, "No trait implementations found for type `{ty}`.");
            } else {
                for ti in &impls {
                    let _ = writeln!(
                        output,
                        "- **{}** ({}:{}-{})",
                        ti.trait_name, ti.file_path, ti.line_start, ti.line_end
                    );
                }
            }
        }
        (Some(t), Some(ty)) => {
            let impls = db.get_trait_impls_for_type(ty)?;
            let filtered: Vec<_> = impls.iter().filter(|i| i.trait_name == t).collect();
            let _ = writeln!(output, "## `{ty}` implementation of `{t}`\n");
            if filtered.is_empty() {
                let _ = writeln!(output, "`{ty}` does not implement `{t}`.");
            } else {
                for ti in &filtered {
                    let _ = writeln!(
                        output,
                        "- {}:{}-{}",
                        ti.file_path, ti.line_start, ti.line_end
                    );
                }
            }
        }
        (None, None) => {
            let _ = writeln!(
                output,
                "Provide at least one of `trait_name` or `type_name`."
            );
        }
    }

    Ok(output)
}
```

**Step 4: Wire MCP endpoint**

Add to `src/server/tools/mod.rs`:
```rust
pub mod implements;
```

Add to `src/server/mod.rs`:
```rust
#[derive(Deserialize, JsonSchema)]
struct ImplementsParams {
    /// Trait name to find implementors of
    trait_name: Option<String>,
    /// Type name to find trait implementations for
    type_name: Option<String>,
}
```

Add the MCP tool method:
```rust
#[tool(
    name = "implements",
    description = "Query trait/type relationships. Use trait_name to find all types implementing a trait, type_name to find all traits a type implements, or both to check a specific implementation."
)]
async fn implements(
    &self,
    Parameters(params): Parameters<ImplementsParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(
        trait_name = ?params.trait_name,
        type_name = ?params.type_name,
        "Tool call: implements"
    );
    let _guard = crate::status::StatusGuard::new("implements");
    self.refresh()?;
    let db = self.lock_db()?;
    let result = tools::implements::handle_implements(
        &db,
        params.trait_name.as_deref(),
        params.type_name.as_deref(),
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

Update `get_info()` instructions string to mention `implements`.

**Step 5: Run tests to verify pass**

Run: `cargo test --lib -- implements::tests`
Expected: All pass.

**Step 6: Commit**

```bash
git add src/server/tools/implements.rs src/server/tools/mod.rs src/server/mod.rs
git commit -m "feat: add implements tool for trait/type relationship queries"
```

---

## Task 6: New `neighborhood` tool

Bidirectional graph exploration: show everything within N hops of a symbol (both callers and callees).

**Files:**
- Create: `src/server/tools/neighborhood.rs`
- Modify: `src/db.rs` — add `get_callers_by_name` method
- Modify: `src/server/tools/mod.rs` — add `pub mod neighborhood;`
- Modify: `src/server/mod.rs` — add params struct and MCP endpoint

**Step 1: Write DB test for `get_callers_by_name`**

```rust
#[test]
fn test_get_callers_by_name() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
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
                ..default_symbol()
            },
            Symbol {
                name: "caller_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 10,
                signature: "pub fn caller_fn()".into(),
                ..default_symbol()
            },
        ],
    )
    .unwrap();
    let target_id = db.get_symbol_id("target_fn", "src/lib.rs").unwrap().unwrap();
    let caller_id = db.get_symbol_id("caller_fn", "src/lib.rs").unwrap().unwrap();
    db.insert_symbol_ref(caller_id, target_id, "call").unwrap();

    let callers = db.get_callers_by_name("target_fn").unwrap();
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].0, "caller_fn");
}
```

Note: use a `default_symbol()` helper if one exists, otherwise inline the fields like other tests.

**Step 2: Implement `get_callers_by_name` in `src/db.rs`**

Mirrors `get_callees_by_name` but reverses direction:

```rust
pub fn get_callers_by_name(
    &self,
    symbol_name: &str,
) -> SqlResult<Vec<(String, String)>> {
    let mut stmt = self.conn.prepare_cached(
        "SELECT DISTINCT ss.name, sf.path \
         FROM symbol_refs sr \
         JOIN symbols ss ON ss.id = sr.source_symbol_id \
         JOIN symbols ts ON ts.id = sr.target_symbol_id \
         JOIN files sf ON sf.id = ss.file_id \
         WHERE ts.name = ?1",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query(params![symbol_name])?;
    while let Some(row) = rows.next()? {
        results.push((row.get(0)?, row.get(1)?));
    }
    Ok(results)
}
```

**Step 3: Write the neighborhood handler test**

```rust
#[test]
fn test_neighborhood_bidirectional() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    // a -> center -> b
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "caller_a".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1, line_end: 5,
                signature: "pub fn caller_a()".into(),
                doc_comment: None, body: None, details: None,
                attributes: None, impl_type: None,
            },
            Symbol {
                name: "center".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7, line_end: 12,
                signature: "pub fn center()".into(),
                doc_comment: None, body: None, details: None,
                attributes: None, impl_type: None,
            },
            Symbol {
                name: "callee_b".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 14, line_end: 18,
                signature: "pub fn callee_b()".into(),
                doc_comment: None, body: None, details: None,
                attributes: None, impl_type: None,
            },
        ],
    )
    .unwrap();

    let a_id = db.get_symbol_id("caller_a", "src/lib.rs").unwrap().unwrap();
    let c_id = db.get_symbol_id("center", "src/lib.rs").unwrap().unwrap();
    let b_id = db.get_symbol_id("callee_b", "src/lib.rs").unwrap().unwrap();
    db.insert_symbol_ref(a_id, c_id, "call").unwrap();
    db.insert_symbol_ref(c_id, b_id, "call").unwrap();

    let result = handle_neighborhood(&db, "center", None).unwrap();
    assert!(result.contains("caller_a"), "should show caller");
    assert!(result.contains("callee_b"), "should show callee");
    assert!(result.contains("center"), "should show the center symbol");
}
```

**Step 4: Implement `handle_neighborhood`**

```rust
use crate::db::Database;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

pub fn handle_neighborhood(
    db: &Database,
    symbol_name: &str,
    depth: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(depth.unwrap_or(2).max(1)).unwrap_or(2);

    let syms = super::resolve_symbol(db, symbol_name)?;
    if syms.is_empty() {
        return Ok(format!("Symbol '{symbol_name}' not found."));
    }

    let base = symbol_name
        .split_once("::")
        .map_or(symbol_name, |(_, m)| m);

    // BFS outward (callees)
    let mut outward: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    outward.insert(base.to_string(), 0);
    queue.push_back((base.to_string(), 0));
    while let Some((current, d)) = queue.pop_front() {
        if d >= max_depth {
            continue;
        }
        for (callee, _file) in db.get_callees_by_name(&current)? {
            if !outward.contains_key(&callee) {
                outward.insert(callee.clone(), d + 1);
                queue.push_back((callee, d + 1));
            }
        }
    }

    // BFS inward (callers)
    let mut inward: BTreeMap<String, usize> = BTreeMap::new();
    inward.insert(base.to_string(), 0);
    queue.push_back((base.to_string(), 0));
    while let Some((current, d)) = queue.pop_front() {
        if d >= max_depth {
            continue;
        }
        for (caller, _file) in db.get_callers_by_name(&current)? {
            if !inward.contains_key(&caller) {
                inward.insert(caller.clone(), d + 1);
                queue.push_back((caller, d + 1));
            }
        }
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Neighborhood: {symbol_name} (depth {max_depth})\n");

    // Callers section
    let callers: Vec<_> = inward.iter()
        .filter(|(n, _)| n.as_str() != base)
        .collect();
    if !callers.is_empty() {
        let _ = writeln!(output, "### Callers (upstream)\n");
        for (name, d) in &callers {
            let _ = writeln!(output, "- **{name}** (depth {d})");
        }
        let _ = writeln!(output);
    }

    // Center
    let _ = writeln!(output, "### Center: {base}\n");
    if let Some(sym) = syms.first() {
        let _ = writeln!(
            output,
            "- {}:{}-{} — `{}`",
            sym.file_path, sym.line_start, sym.line_end, sym.signature
        );
    }
    let _ = writeln!(output);

    // Callees section
    let callees: Vec<_> = outward.iter()
        .filter(|(n, _)| n.as_str() != base)
        .collect();
    if !callees.is_empty() {
        let _ = writeln!(output, "### Callees (downstream)\n");
        for (name, d) in &callees {
            let _ = writeln!(output, "- **{name}** (depth {d})");
        }
        let _ = writeln!(output);
    }

    if callers.is_empty() && callees.is_empty() {
        let _ = writeln!(output, "No connections found within depth {max_depth}.");
    }

    Ok(output)
}
```

**Step 5: Wire MCP endpoint**

Add `pub mod neighborhood;` to `src/server/tools/mod.rs`.

Add to `src/server/mod.rs`:

```rust
#[derive(Deserialize, JsonSchema)]
struct NeighborhoodParams {
    /// Symbol to explore around
    symbol_name: String,
    /// Max hops in each direction (default: 2)
    depth: Option<i64>,
}
```

And the tool method:

```rust
#[tool(
    name = "neighborhood",
    description = "Explore the local call graph around a symbol. Shows callers (upstream) and callees (downstream) within N hops. Use for understanding a symbol's role in the architecture."
)]
async fn neighborhood(
    &self,
    Parameters(params): Parameters<NeighborhoodParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(symbol = %params.symbol_name, depth = ?params.depth, "Tool call: neighborhood");
    let _guard = crate::status::StatusGuard::new(
        &format!("neighborhood ▸ {}", params.symbol_name),
    );
    self.refresh()?;
    let db = self.lock_db()?;
    let result = tools::neighborhood::handle_neighborhood(
        &db, &params.symbol_name, params.depth,
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

Update `get_info()` instructions to mention `neighborhood`.

**Step 6: Run tests**

Run: `cargo test --lib -- neighborhood::tests`
Expected: All pass.

**Step 7: Commit**

```bash
git add src/db.rs src/server/tools/neighborhood.rs src/server/tools/mod.rs src/server/mod.rs
git commit -m "feat: add neighborhood tool for bidirectional graph exploration"
```

---

## Task 7: Lightweight `diff_impact` mode (`changes_only`)

Add a `changes_only` flag to `diff_impact`. When true, list changed symbols without computing downstream impact.

**Files:**
- Modify: `src/server/mod.rs:122-126` — add `changes_only` to `DiffImpactParams`
- Modify: `src/server/tools/diff_impact.rs` — early return after symbol detection

**Step 1: Write the failing test**

In `src/server/tools/diff_impact.rs` test module (if tests exist) or create one:

```rust
#[test]
fn test_diff_impact_changes_only() {
    // This test validates that the changes_only flag is accepted
    // and produces output with "Changed Symbols" but NOT "Downstream Impact"
    // We can test the parse_diff + symbol lookup without actual git
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[Symbol {
            name: "my_fn".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 10,
            signature: "pub fn my_fn()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }],
    )
    .unwrap();

    // Simulate a diff hunk that touches lines 1-10 of src/lib.rs
    let hunks = vec![DiffHunk {
        file_path: "src/lib.rs".into(),
        line_ranges: vec![(1, 10)],
    }];

    let result = format_changed_symbols(&db, &hunks).unwrap();
    assert!(result.contains("my_fn"));
}
```

**Step 2: Refactor diff_impact to extract `format_changed_symbols`**

Extract the symbol-detection portion of `handle_diff_impact` into a reusable function. Then `handle_diff_impact` can call it and optionally skip the downstream analysis.

Add `changes_only: bool` parameter to `handle_diff_impact`:

```rust
pub fn handle_diff_impact(
    db: &Database,
    repo_path: &str,
    git_ref: Option<&str>,
    changes_only: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let diff_output = run_git_diff(repo_path, git_ref)?;
    let hunks = parse_diff(&diff_output);

    // ... detect changed symbols ...

    if changes_only {
        // Format just the changed symbols and return
        return Ok(output);
    }

    // ... existing downstream impact logic ...
}
```

**Step 3: Wire `changes_only` through params**

In `src/server/mod.rs`:

```rust
#[derive(Deserialize, JsonSchema)]
struct DiffImpactParams {
    /// Git ref range (e.g. "HEAD~3..HEAD", "main"). Omit for unstaged changes.
    git_ref: Option<String>,
    /// Only list changed symbols, skip downstream impact analysis (default: false)
    changes_only: Option<bool>,
}
```

Pass through:
```rust
let result = tools::diff_impact::handle_diff_impact(
    &db, repo_path, params.git_ref.as_deref(),
    params.changes_only.unwrap_or(false),
)
```

**Step 4: Run tests**

Run: `cargo test --lib -- diff_impact::tests`
Expected: All pass.

**Step 5: Commit**

```bash
git add src/server/mod.rs src/server/tools/diff_impact.rs
git commit -m "feat: add changes_only mode to diff_impact"
```

---

## Task 8: Multi-path `callpath` (find all paths, not just shortest)

Add `all_paths: bool` parameter. When true, find up to N paths (default 5) instead of just the shortest.

**Files:**
- Modify: `src/server/mod.rs:128-136` — add `all_paths` and `max_paths` to `CallpathParams`
- Modify: `src/server/tools/callpath.rs` — add DFS-based all-paths algorithm

**Step 1: Write the failing test**

```rust
#[test]
fn test_callpath_all_paths() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    // a -> b -> d, a -> c -> d (two paths from a to d)
    store_symbols(
        &db,
        file_id,
        &[
            Symbol { name: "a".into(), kind: SymbolKind::Function, visibility: Visibility::Public, file_path: "src/lib.rs".into(), line_start: 1, line_end: 3, signature: "pub fn a()".into(), doc_comment: None, body: None, details: None, attributes: None, impl_type: None },
            Symbol { name: "b".into(), kind: SymbolKind::Function, visibility: Visibility::Public, file_path: "src/lib.rs".into(), line_start: 5, line_end: 7, signature: "pub fn b()".into(), doc_comment: None, body: None, details: None, attributes: None, impl_type: None },
            Symbol { name: "c".into(), kind: SymbolKind::Function, visibility: Visibility::Public, file_path: "src/lib.rs".into(), line_start: 9, line_end: 11, signature: "pub fn c()".into(), doc_comment: None, body: None, details: None, attributes: None, impl_type: None },
            Symbol { name: "d".into(), kind: SymbolKind::Function, visibility: Visibility::Public, file_path: "src/lib.rs".into(), line_start: 13, line_end: 15, signature: "pub fn d()".into(), doc_comment: None, body: None, details: None, attributes: None, impl_type: None },
        ],
    )
    .unwrap();

    let a_id = db.get_symbol_id("a", "src/lib.rs").unwrap().unwrap();
    let b_id = db.get_symbol_id("b", "src/lib.rs").unwrap().unwrap();
    let c_id = db.get_symbol_id("c", "src/lib.rs").unwrap().unwrap();
    let d_id = db.get_symbol_id("d", "src/lib.rs").unwrap().unwrap();
    db.insert_symbol_ref(a_id, b_id, "call").unwrap();
    db.insert_symbol_ref(a_id, c_id, "call").unwrap();
    db.insert_symbol_ref(b_id, d_id, "call").unwrap();
    db.insert_symbol_ref(c_id, d_id, "call").unwrap();

    let result = handle_callpath(&db, "a", "d", None, true, None).unwrap();
    assert!(result.contains("2 paths found"), "should find two paths");
    assert!(result.contains("a → b → d") || result.contains("a → c → d"));
}
```

**Step 2: Add `all_paths` parameter to `handle_callpath`**

```rust
pub fn handle_callpath(
    db: &Database,
    from: &str,
    to: &str,
    max_depth: Option<i64>,
    all_paths: bool,
    max_paths: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    if all_paths {
        handle_all_paths(db, from, to, max_depth, max_paths)
    } else {
        handle_shortest_path(db, from, to, max_depth)
    }
}
```

Extract existing BFS into `handle_shortest_path`. Add DFS-based `handle_all_paths`:

```rust
fn handle_all_paths(
    db: &Database,
    from: &str,
    to: &str,
    max_depth: Option<i64>,
    max_paths: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(max_depth.unwrap_or(10).max(1)).unwrap_or(10);
    let max_paths = usize::try_from(max_paths.unwrap_or(5).max(1)).unwrap_or(5);

    let from_syms = super::resolve_symbol(db, from)?;
    if from_syms.is_empty() {
        return Ok(format!("Source symbol '{from}' not found."));
    }
    let to_syms = super::resolve_symbol(db, to)?;
    if to_syms.is_empty() {
        return Ok(format!("Target symbol '{to}' not found."));
    }

    let from_name = base_name(from);
    let to_name = base_name(to);

    let mut paths: Vec<Vec<String>> = Vec::new();
    let mut current_path = vec![from_name.to_string()];
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(from_name.to_string());

    dfs_all_paths(
        db, to_name, max_depth, max_paths,
        &mut current_path, &mut visited, &mut paths,
    )?;

    let mut output = String::new();
    let _ = writeln!(output, "## All Call Paths: {from} → {to}\n");

    if paths.is_empty() {
        let _ = writeln!(output, "No paths found within depth {max_depth}.");
    } else {
        let _ = writeln!(output, "**{} paths found:**\n", paths.len());
        for (i, path) in paths.iter().enumerate() {
            let _ = writeln!(
                output,
                "{}. `{}` ({} hops)",
                i + 1,
                path.join(" → "),
                path.len() - 1
            );
        }
    }

    Ok(output)
}

fn dfs_all_paths(
    db: &Database,
    target: &str,
    max_depth: usize,
    max_paths: usize,
    current_path: &mut Vec<String>,
    visited: &mut HashSet<String>,
    paths: &mut Vec<Vec<String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if paths.len() >= max_paths || current_path.len() > max_depth {
        return Ok(());
    }

    let current = current_path.last().unwrap().clone();
    let callees = db.get_callees_by_name(&current)?;

    for (callee, _file) in callees {
        if callee == target {
            let mut path = current_path.clone();
            path.push(callee);
            paths.push(path);
            if paths.len() >= max_paths {
                return Ok(());
            }
            continue;
        }
        if !visited.contains(&callee) {
            visited.insert(callee.clone());
            current_path.push(callee.clone());
            dfs_all_paths(db, target, max_depth, max_paths, current_path, visited, paths)?;
            current_path.pop();
            visited.remove(&callee);
        }
    }

    Ok(())
}
```

**Step 3: Wire params**

In `src/server/mod.rs`, update `CallpathParams`:

```rust
#[derive(Deserialize, JsonSchema)]
struct CallpathParams {
    /// Source symbol name
    from: String,
    /// Target symbol name
    to: String,
    /// Max search depth (default: 10)
    max_depth: Option<i64>,
    /// Find all paths instead of just the shortest (default: false)
    all_paths: Option<bool>,
    /// Max number of paths when all_paths=true (default: 5)
    max_paths: Option<i64>,
}
```

Update callpath tool method to pass new params:

```rust
let result = tools::callpath::handle_callpath(
    &db, &params.from, &params.to, params.max_depth,
    params.all_paths.unwrap_or(false),
    params.max_paths,
)
```

**Step 4: Run tests**

Run: `cargo test --lib -- callpath::tests`
Expected: All pass.

**Step 5: Commit**

```bash
git add src/server/mod.rs src/server/tools/callpath.rs
git commit -m "feat: add all_paths mode to callpath tool"
```

---

## Task 9: New `type_usage` tool

Find where a type is used as struct fields, function parameters, and return types via signature/details text search.

**Files:**
- Create: `src/server/tools/type_usage.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_type_usage_in_signatures() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "Config".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct Config".into(),
                doc_comment: None,
                body: None,
                details: Some("port: u16\nhost: String".into()),
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "load_config".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 12,
                signature: "pub fn load_config() -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "use_config".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 14,
                line_end: 18,
                signature: "pub fn use_config(cfg: &Config)".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "AppState".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 20,
                line_end: 24,
                signature: "pub struct AppState".into(),
                doc_comment: None,
                body: None,
                details: Some("config: Config\nname: String".into()),
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "unrelated".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 26,
                line_end: 30,
                signature: "pub fn unrelated()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ],
    )
    .unwrap();

    let result = handle_type_usage(&db, "Config", None).unwrap();
    assert!(result.contains("load_config"), "should find fn returning Config");
    assert!(result.contains("use_config"), "should find fn accepting Config");
    assert!(result.contains("AppState"), "should find struct with Config field");
    assert!(!result.contains("unrelated"), "should not include unrelated fn");
}
```

**Step 2: Implement `handle_type_usage`**

```rust
use crate::db::Database;
use std::fmt::Write;

pub fn handle_type_usage(
    db: &Database,
    type_name: &str,
    path: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();
    let _ = writeln!(output, "## Type Usage: `{type_name}`\n");

    // Find in function signatures (parameters and return types)
    let mut sig_matches = db.search_symbols_by_signature(type_name)?;
    // Exclude the type's own definition
    sig_matches.retain(|s| s.name != type_name);
    if let Some(p) = path {
        sig_matches.retain(|s| s.file_path.starts_with(p));
    }

    // Split into "returns" and "accepts"
    let mut returns = Vec::new();
    let mut accepts = Vec::new();
    for sym in &sig_matches {
        let sig = &sym.signature;
        if let Some(ret_part) = sig.split_once("->").map(|(_, r)| r) {
            if ret_part.contains(type_name) {
                returns.push(sym);
                continue;
            }
        }
        // If type appears before ->, it's a parameter
        accepts.push(sym);
    }

    if !returns.is_empty() {
        let _ = writeln!(output, "### Returns `{type_name}`\n");
        for sym in &returns {
            let qname = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qname}** ({}:{}) — `{}`",
                sym.file_path, sym.line_start, sym.signature
            );
        }
        let _ = writeln!(output);
    }

    if !accepts.is_empty() {
        let _ = writeln!(output, "### Accepts `{type_name}`\n");
        for sym in &accepts {
            let qname = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qname}** ({}:{}) — `{}`",
                sym.file_path, sym.line_start, sym.signature
            );
        }
        let _ = writeln!(output);
    }

    // Find as struct fields (search details column)
    let all_structs = db.search_symbols_by_signature("struct")?;
    let mut field_matches: Vec<_> = all_structs
        .into_iter()
        .filter(|s| s.name != type_name)
        .filter(|s| {
            s.details
                .as_deref()
                .is_some_and(|d| d.contains(type_name))
        })
        .collect();
    if let Some(p) = path {
        field_matches.retain(|s| s.file_path.starts_with(p));
    }

    if !field_matches.is_empty() {
        let _ = writeln!(output, "### Contains `{type_name}` as field\n");
        for sym in &field_matches {
            let _ = writeln!(
                output,
                "- **{}** ({}:{})",
                sym.name, sym.file_path, sym.line_start
            );
        }
        let _ = writeln!(output);
    }

    // Also check impl_type — methods on this type
    let impl_methods = db.search_symbols_by_impl(type_name, "")?;
    // Actually, search_symbols_by_impl requires both type and name.
    // We'd need a new DB method to get all methods of a type.
    // For now, skip or use a broader approach.

    if returns.is_empty() && accepts.is_empty() && field_matches.is_empty() {
        let _ = writeln!(output, "No usage of `{type_name}` found in signatures or struct fields.");
    }

    Ok(output)
}
```

Note: The "Contains as field" approach has a limitation — it searches all structs' `details` fields by text. This works well for common types but may have edge cases with substring matches (e.g., "Id" matching "WidgetId"). This is documented as a best-effort tool.

**Step 3: Wire MCP endpoint**

Similar pattern as other tools. Add `pub mod type_usage;` to tools/mod.rs.

```rust
#[derive(Deserialize, JsonSchema)]
struct TypeUsageParams {
    /// Type name to find usages of
    type_name: String,
    /// Filter to files under this path prefix
    path: Option<String>,
}
```

```rust
#[tool(
    name = "type_usage",
    description = "Find where a type is used: as function parameters, return types, and struct fields. Best-effort text search on signatures and struct details."
)]
async fn type_usage(
    &self,
    Parameters(params): Parameters<TypeUsageParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(type_name = %params.type_name, "Tool call: type_usage");
    let _guard = crate::status::StatusGuard::new(
        &format!("type_usage ▸ {}", params.type_name),
    );
    self.refresh()?;
    let db = self.lock_db()?;
    let result = tools::type_usage::handle_type_usage(
        &db, &params.type_name, params.path.as_deref(),
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 4: Run tests**

Run: `cargo test --lib -- type_usage::tests`
Expected: All pass.

**Step 5: Commit**

```bash
git add src/server/tools/type_usage.rs src/server/tools/mod.rs src/server/mod.rs
git commit -m "feat: add type_usage tool for structural type analysis"
```

---

## Task 10: File-level import graph

Show which files import from which other files, based on `use` declarations.

**Files:**
- Modify: `src/db.rs` — new `file_imports` table, insert/query methods
- Modify: `src/indexer/mod.rs` — store file imports during indexing
- Create: `src/server/tools/file_graph.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Add `file_imports` table to DB schema**

In `src/db.rs`, in the `CREATE TABLE` section of `open`:

```sql
CREATE TABLE IF NOT EXISTS file_imports (
    id INTEGER PRIMARY KEY,
    source_file_id INTEGER NOT NULL REFERENCES files(id),
    target_path TEXT NOT NULL,
    import_path TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_file_imports_source ON file_imports(source_file_id);
CREATE INDEX IF NOT EXISTS idx_file_imports_target ON file_imports(target_path);
```

Add methods:

```rust
pub fn insert_file_import(
    &self,
    source_file_id: FileId,
    target_path: &str,
    import_path: &str,
) -> SqlResult<()> {
    self.conn.execute(
        "INSERT INTO file_imports (source_file_id, target_path, import_path) VALUES (?1, ?2, ?3)",
        params![source_file_id, target_path, import_path],
    )?;
    Ok(())
}

pub fn get_file_imports(&self, path_prefix: &str) -> SqlResult<Vec<(String, String)>> {
    let pattern = format!("{path_prefix}%");
    let mut stmt = self.conn.prepare(
        "SELECT DISTINCT sf.path, fi.target_path \
         FROM file_imports fi \
         JOIN files sf ON sf.id = fi.source_file_id \
         WHERE sf.path LIKE ?1 \
         ORDER BY sf.path, fi.target_path",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query(params![pattern])?;
    while let Some(row) = rows.next()? {
        results.push((row.get(0)?, row.get(1)?));
    }
    Ok(results)
}

pub fn clear_file_imports(&self) -> SqlResult<()> {
    self.conn.execute("DELETE FROM file_imports", [])?;
    Ok(())
}
```

**Step 2: Store file imports during indexing**

In `src/indexer/mod.rs`, after the symbol ref extraction phase, add a file import extraction phase. Use `extract_import_map` which already returns imports with their source crate/module info. Map each import's module path to a file path using the existing file index.

This is the most complex step. The key insight is that `ImportInfo` already has `module_path` which contains the crate/module chain. We need to resolve these to file paths.

Simpler approach: scan `use crate::...` declarations and map `crate::foo::bar` to `src/foo/bar.rs` or `src/foo/bar/mod.rs`. For workspace crates, use the crate map.

```rust
fn extract_file_imports(
    db: &Database,
    sources: &[(FileId, String, String)], // (file_id, path, content)
) -> Result<(), Box<dyn std::error::Error>> {
    db.clear_file_imports()?;
    let all_files: std::collections::HashSet<String> =
        db.get_all_file_paths()?.into_iter().collect();

    for (file_id, _path, content) in sources {
        let Ok(tree) = crate::indexer::parser::parse_source_for_imports(content) else {
            continue;
        };
        // Extract `use crate::...` paths and resolve to files
        for import in &tree {
            // Try to resolve module path to file
            let candidates = module_to_file_paths(&import.module_path);
            for candidate in candidates {
                if all_files.contains(&candidate) {
                    db.insert_file_import(*file_id, &candidate, &import.full_path)?;
                    break;
                }
            }
        }
    }
    Ok(())
}
```

**Note:** This task is the most involved and may need iteration. The exact implementation depends on how `extract_import_map` returns data and what module path resolution logic is needed. The implementer should read `extract_import_map` carefully and adapt.

**Step 3: Write handler**

```rust
use crate::db::Database;
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn handle_file_graph(
    db: &Database,
    path: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let imports = db.get_file_imports(path)?;

    let mut output = String::new();
    let _ = writeln!(output, "## File Import Graph: {path}\n");

    if imports.is_empty() {
        let _ = writeln!(output, "No file imports found under '{path}'.");
        return Ok(output);
    }

    // Group by source file
    let mut graph: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (source, target) in &imports {
        graph.entry(source).or_default().push(target);
    }

    for (source, targets) in &graph {
        let _ = writeln!(output, "### {source}\n");
        for target in targets {
            let _ = writeln!(output, "- → {target}");
        }
        let _ = writeln!(output);
    }

    // Summary
    let file_count = graph.len();
    let edge_count = imports.len();
    let _ = writeln!(
        output,
        "---\n**{file_count} files, {edge_count} import edges**"
    );

    Ok(output)
}
```

**Step 4: Wire MCP endpoint**

```rust
#[derive(Deserialize, JsonSchema)]
struct FileGraphParams {
    /// Path prefix to scope the graph (e.g. "src/server/")
    path: String,
}
```

```rust
#[tool(
    name = "file_graph",
    description = "Show the file-level import dependency graph under a path prefix. Shows which files import from which other files."
)]
async fn file_graph(
    &self,
    Parameters(params): Parameters<FileGraphParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(path = %params.path, "Tool call: file_graph");
    let _guard = crate::status::StatusGuard::new(&format!("file_graph ▸ {}", params.path));
    self.refresh()?;
    let db = self.lock_db()?;
    let result = tools::file_graph::handle_file_graph(&db, &params.path)
        .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 5: Run tests**

Run: `cargo test --lib -- file_graph::tests`
Expected: All pass.

**Step 6: Commit**

```bash
git add src/db.rs src/indexer/mod.rs src/server/tools/file_graph.rs src/server/tools/mod.rs src/server/mod.rs
git commit -m "feat: add file_graph tool for file-level import visualization"
```

---

## Post-Implementation Checklist

After all 10 tasks are done:

1. **Update CLAUDE.md** — Add new tools to the commands table and key patterns section
2. **Update `get_info()` instructions** — Mention all new tools in the MCP server info string
3. **Run full test suite:** `cargo test`
4. **Run clippy:** `cargo clippy --all-targets --all-features -- -D warnings`
5. **Run formatter:** `cargo fmt --all -- --check`
6. **Self-index test:** Build and run against the illu-rs repo itself to verify new tools work on real data
7. **Final commit with CLAUDE.md updates**

## Dependency Order

Tasks 1-9 are independent of each other and can be implemented in any order or in parallel. Task 10 (file graph) is the most complex and should be done last.

Recommended parallelization groups:
- **Group A (query enhancements):** Tasks 1, 2, 3 — all modify `query.rs` so do sequentially
- **Group B (new tools, no DB changes):** Tasks 5, 9 — independent, can parallelize
- **Group C (existing tool enhancements):** Tasks 4, 7, 8 — independent, can parallelize
- **Group D (needs new DB):** Tasks 6, 10 — Task 6 adds `get_callers_by_name`, Task 10 adds table

Sequential order within groups, parallel across groups.
