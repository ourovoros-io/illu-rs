# v2 Enhancements Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make illu-rs more accurate (qualified path refs), more useful (diff-based impact, better docs), and surface existing data (trait impls already done).

**Architecture:** Four independent changes: (1) split `clear_index` to preserve docs across re-indexes, (2) add `module` column to docs for two-tier storage, (3) add import map parsing for qualified ref resolution, (4) new `diff_impact` MCP tool.

**Tech Stack:** Rust, tree-sitter, rusqlite, rmcp, git CLI

**Note:** Trait impl mapping in context output is already implemented (`render_trait_info` in `src/server/tools/context.rs:116-152`). Skipped from this plan.

---

### Task 1: Preserve docs across re-indexes

Split `clear_index` so re-indexing keeps cached dependency docs.

**Files:**
- Modify: `src/db.rs:282-297` (split `clear_index`)
- Modify: `src/indexer/mod.rs:17` (call `clear_code_index` instead of `clear_index`)
- Test: `src/db.rs` (existing test module) and `src/indexer/mod.rs` (existing test module)

**Step 1: Write failing test for `clear_code_index`**

Add to `src/db.rs` test module:

```rust
#[test]
fn test_clear_code_index_preserves_docs() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[Symbol {
            name: "foo".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub fn foo()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
        }],
    )
    .unwrap();
    let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
    db.store_doc(dep_id, "cargo_doc", "Serde docs content")
        .unwrap();

    // Clear code index only
    db.clear_code_index().unwrap();

    // Symbols should be gone
    let syms = db.search_symbols("foo").unwrap();
    assert!(syms.is_empty());

    // Docs should survive
    let docs = db.get_docs_for_dependency("serde").unwrap();
    assert_eq!(docs.len(), 1);
    assert!(docs[0].content.contains("Serde docs"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- db::tests::test_clear_code_index_preserves_docs`
Expected: FAIL — `clear_code_index` method doesn't exist yet.

**Step 3: Implement `clear_code_index` and rename old method**

In `src/db.rs`, replace the existing `clear_index` (lines 282-297) with two methods:

```rust
pub fn clear_code_index(&self) -> SqlResult<()> {
    self.conn.execute_batch(
        "DELETE FROM docs_fts;
         DELETE FROM symbols_fts;
         DELETE FROM symbols_trigram;
         DELETE FROM symbol_refs;
         DELETE FROM trait_impls;
         DELETE FROM symbols;
         DELETE FROM files;
         DELETE FROM crate_deps;
         DELETE FROM crates;
         DELETE FROM metadata;",
    )
}

pub fn clear_all(&self) -> SqlResult<()> {
    self.clear_code_index()?;
    self.conn.execute_batch(
        "DELETE FROM docs;
         DELETE FROM dependencies;",
    )
}
```

**Step 4: Update `index_repo` to call `clear_code_index`**

In `src/indexer/mod.rs:17`, change:
```rust
// Before:
db.clear_index()?;
// After:
db.clear_code_index()?;
```

**Step 5: Run tests to verify**

Run: `cargo test --lib -- db::tests::test_clear_code_index_preserves_docs`
Expected: PASS

Run: `cargo test --lib` to check no regressions.
Expected: All pass. If any test calls `clear_index`, update it to `clear_all` or `clear_code_index` as appropriate.

**Step 6: Add test for version-change cleanup**

Add to `src/db.rs` test module:

```rust
#[test]
fn test_clear_code_index_then_reinsert_deps_preserves_matching_docs() {
    let db = Database::open_in_memory().unwrap();
    let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
    db.store_doc(dep_id, "cargo_doc", "Old serde docs").unwrap();

    db.clear_code_index().unwrap();

    // Same version re-inserted — docs should still be accessible
    let docs = db.get_docs_for_dependency("serde").unwrap();
    assert_eq!(docs.len(), 1);
}
```

**Step 7: Run and verify**

Run: `cargo test --lib -- db::tests::test_clear_code_index_then_reinsert`
Expected: PASS

**Step 8: Run full test suite and lint**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings.

**Step 9: Commit**

```bash
git add src/db.rs src/indexer/mod.rs
git commit -m "feat: split clear_index to preserve docs across re-indexes

clear_code_index() deletes symbols/refs/files/crates but keeps
docs and dependencies. clear_all() does a full reset.
index_repo now uses clear_code_index()."
```

---

### Task 2: Two-tier doc storage (summary + per-module detail)

Add `module` column to docs table, produce per-module docs from rustdoc JSON, update `handle_docs` to serve summary vs detail.

**Files:**
- Modify: `src/db.rs` (schema migration, `store_doc`, `get_docs_for_dependency`, new `get_doc_by_module`)
- Modify: `src/indexer/cargo_doc.rs` (produce per-module output from `parse_rustdoc_json`)
- Modify: `src/server/tools/docs.rs` (summary vs detail routing)
- Test: all three files' test modules

**Step 1: Write failing test for `store_doc` with module**

Add to `src/db.rs` test module:

```rust
#[test]
fn test_store_doc_with_module() {
    let db = Database::open_in_memory().unwrap();
    let dep_id = db.insert_dependency("tokio", "1.0", true, None).unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "summary content", "")
        .unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "sync module docs", "sync")
        .unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "fs module docs", "fs")
        .unwrap();

    // get_docs_for_dependency returns all rows
    let all = db.get_docs_for_dependency("tokio").unwrap();
    assert_eq!(all.len(), 3);

    // get_doc_by_module returns specific module
    let sync = db.get_doc_by_module("tokio", "sync").unwrap();
    assert!(sync.is_some());
    assert!(sync.unwrap().content.contains("sync module"));

    // Empty module returns summary
    let summary = db.get_doc_by_module("tokio", "").unwrap();
    assert!(summary.is_some());
    assert!(summary.unwrap().content.contains("summary"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- db::tests::test_store_doc_with_module`
Expected: FAIL — `store_doc_with_module` and `get_doc_by_module` don't exist.

**Step 3: Add `module` column to schema and new DB methods**

In `src/db.rs` migrate method, update the `docs` table:

```sql
CREATE TABLE IF NOT EXISTS docs (
    id INTEGER PRIMARY KEY,
    dependency_id INTEGER NOT NULL REFERENCES dependencies(id),
    source TEXT NOT NULL,
    content TEXT NOT NULL,
    module TEXT NOT NULL DEFAULT ''
);
```

Add new methods to `Database`:

```rust
pub fn store_doc_with_module(
    &self,
    dep_id: DepId,
    source: &str,
    content: &str,
    module: &str,
) -> SqlResult<()> {
    self.conn.execute(
        "INSERT INTO docs (dependency_id, source, content, module) \
         VALUES (?1, ?2, ?3, ?4)",
        params![dep_id, source, content, module],
    )?;
    let rowid = self.conn.last_insert_rowid();
    self.conn.execute(
        "INSERT INTO docs_fts (rowid, content) \
         VALUES (?1, ?2)",
        params![rowid, content],
    )?;
    Ok(())
}

pub fn get_doc_by_module(
    &self,
    dep_name: &str,
    module: &str,
) -> SqlResult<Option<DocResult>> {
    let mut stmt = self.conn.prepare(
        "SELECT d.content, d.source, dep.name, dep.version \
         FROM docs d \
         JOIN dependencies dep ON dep.id = d.dependency_id \
         WHERE dep.name = ?1 AND d.module = ?2 \
         LIMIT 1",
    )?;
    let mut rows = stmt.query(params![dep_name, module])?;
    match rows.next()? {
        Some(row) => Ok(Some(row_to_doc_result(row)?)),
        None => Ok(None),
    }
}
```

Update existing `store_doc` to delegate:

```rust
pub fn store_doc(&self, dep_id: DepId, source: &str, content: &str) -> SqlResult<()> {
    self.store_doc_with_module(dep_id, source, content, "")
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib -- db::tests::test_store_doc_with_module`
Expected: PASS

**Step 5: Run all DB tests**

Run: `cargo test --lib -- db::tests`
Expected: All pass.

**Step 6: Commit DB changes**

```bash
git add src/db.rs
git commit -m "feat: add module column to docs table for two-tier storage

store_doc_with_module() stores docs tagged by module path.
get_doc_by_module() retrieves docs for a specific module.
Existing store_doc() defaults to empty module (crate summary)."
```

**Step 7: Write failing test for per-module rustdoc parsing**

Add to `src/indexer/cargo_doc.rs` test module:

```rust
#[test]
fn test_parse_rustdoc_json_with_modules() {
    let json = serde_json::json!({
        "root": "0:0",
        "crate_version": "1.0.0",
        "index": {
            "0:0": {
                "id": "0:0", "crate_id": 0, "name": "mylib",
                "visibility": "public",
                "docs": "A library with modules.",
                "inner": {
                    "module": {
                        "is_crate": true,
                        "items": ["0:1", "0:2"]
                    }
                }
            },
            "0:1": {
                "id": "0:1", "crate_id": 0, "name": "sync",
                "visibility": "public",
                "docs": "Synchronization primitives.",
                "inner": {
                    "module": {
                        "is_crate": false,
                        "items": ["0:3"]
                    }
                }
            },
            "0:2": {
                "id": "0:2", "crate_id": 0, "name": "Config",
                "visibility": "public",
                "docs": "Top-level config.",
                "inner": {
                    "struct": {
                        "kind": { "plain": { "fields": [], "has_stripped_fields": false } },
                        "generics": { "params": [], "where_predicates": [] },
                        "impls": []
                    }
                }
            },
            "0:3": {
                "id": "0:3", "crate_id": 0, "name": "Mutex",
                "visibility": "public",
                "docs": "A mutual exclusion lock.",
                "inner": {
                    "struct": {
                        "kind": { "plain": { "fields": [], "has_stripped_fields": false } },
                        "generics": { "params": [{"name": "T"}], "where_predicates": [] },
                        "impls": []
                    }
                }
            }
        },
        "paths": {},
        "external_crates": {},
        "format_version": 39
    });

    let results = parse_rustdoc_json_modules(&json.to_string(), "mylib").unwrap();

    // Should have summary (module="") and module detail (module="sync")
    assert!(results.len() >= 2);

    let summary = results.iter().find(|r| r.module.is_empty()).unwrap();
    assert!(summary.content.contains("mylib"));
    assert!(summary.content.contains("sync"));
    assert!(summary.content.contains("Config"));

    let sync_mod = results.iter().find(|r| r.module == "sync").unwrap();
    assert!(sync_mod.content.contains("Mutex"));
}
```

**Step 8: Run test to verify it fails**

Run: `cargo test --lib -- cargo_doc::tests::test_parse_rustdoc_json_with_modules`
Expected: FAIL — `parse_rustdoc_json_modules` doesn't exist.

**Step 9: Implement `parse_rustdoc_json_modules`**

In `src/indexer/cargo_doc.rs`, add a new struct and function:

```rust
pub struct ModuleDoc {
    pub module: String,
    pub content: String,
}

/// Parse rustdoc JSON into a summary + per-module detail docs.
pub fn parse_rustdoc_json_modules(
    json_str: &str,
    crate_name: &str,
) -> Result<Vec<ModuleDoc>, Box<dyn std::error::Error>> {
    let doc: serde_json::Value = serde_json::from_str(json_str)?;
    let version = doc
        .get("crate_version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let index = doc
        .get("index")
        .and_then(|v| v.as_object())
        .ok_or("missing index")?;

    let mut results = Vec::new();

    // Collect modules and top-level items
    let mut modules: Vec<(&str, Vec<String>)> = Vec::new();
    let mut top_level_items = CollectedItems {
        traits: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        functions: Vec::new(),
        macros: Vec::new(),
    };

    // Find root module and its direct children
    let root_id = doc.get("root")
        .map(|r| r.to_string().replace('"', ""))
        .unwrap_or_default();
    let root_items: Vec<String> = index
        .get(&root_id)
        .and_then(|r| r.get("inner"))
        .and_then(|i| i.get("module"))
        .and_then(|m| m.get("items"))
        .and_then(|items| items.as_array())
        .map(|arr| arr.iter().filter_map(|v| {
            Some(v.to_string().replace('"', ""))
        }).collect())
        .unwrap_or_default();

    for item_id in &root_items {
        let Some(item) = index.get(item_id) else { continue };
        let Some(name) = item.get("name").and_then(|n| n.as_str()) else { continue };
        let Some(inner) = item.get("inner").and_then(|i| i.as_object()) else { continue };

        if inner.contains_key("module") {
            // This is a submodule — collect its item IDs
            let sub_items: Vec<String> = inner
                .get("module")
                .and_then(|m| m.get("items"))
                .and_then(|items| items.as_array())
                .map(|arr| arr.iter().filter_map(|v| {
                    Some(v.to_string().replace('"', ""))
                }).collect())
                .unwrap_or_default();
            modules.push((name, sub_items));
        } else {
            // Top-level item
            classify_item(item, &mut top_level_items);
        }
    }

    // Build summary: crate docs + module listing + top-level items
    let mut summary = String::new();
    let _ = writeln!(summary, "# {crate_name} {version}\n");

    // Crate-level docs
    if let Some(root_item) = index.get(&root_id)
        && let Some(docs) = root_item.get("docs").and_then(|d| d.as_str())
    {
        let truncated = truncate_doc(docs, 500);
        let _ = writeln!(summary, "{truncated}\n");
    }

    // Module listing
    if !modules.is_empty() {
        let _ = writeln!(summary, "## Modules\n");
        for (mod_name, items) in &modules {
            // Get the module's doc comment
            let mod_doc = root_items.iter()
                .find_map(|id| {
                    let item = index.get(id)?;
                    let n = item.get("name")?.as_str()?;
                    if n == *mod_name {
                        item.get("docs").and_then(|d| d.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("");
            let first = first_doc_line(mod_doc);
            if first.is_empty() {
                let _ = writeln!(summary, "- **{mod_name}** ({} items)", items.len());
            } else {
                let _ = writeln!(summary, "- **{mod_name}** — {first}");
            }
        }
        let _ = writeln!(summary);
    }

    // Top-level items in summary
    render_section(&mut summary, "Traits", &top_level_items.traits);
    render_section(&mut summary, "Structs", &top_level_items.structs);
    render_section(&mut summary, "Enums", &top_level_items.enums);
    render_section(&mut summary, "Functions", &top_level_items.functions);
    render_section(&mut summary, "Macros", &top_level_items.macros);

    results.push(ModuleDoc {
        module: String::new(),
        content: truncate_doc(&summary, 4000),
    });

    // Per-module detail docs
    for (mod_name, item_ids) in &modules {
        let mut mod_items = CollectedItems {
            traits: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            functions: Vec::new(),
            macros: Vec::new(),
        };

        for item_id in item_ids {
            if let Some(item) = index.get(item_id) {
                classify_item(item, &mut mod_items);
            }
        }

        let mut detail = String::new();
        let _ = writeln!(detail, "# {crate_name}::{mod_name}\n");

        render_section(&mut detail, "Traits", &mod_items.traits);
        render_section(&mut detail, "Structs", &mod_items.structs);
        render_section(&mut detail, "Enums", &mod_items.enums);
        render_section(&mut detail, "Functions", &mod_items.functions);
        render_section(&mut detail, "Macros", &mod_items.macros);

        if !detail.trim().is_empty() {
            results.push(ModuleDoc {
                module: mod_name.to_string(),
                content: truncate_doc(&detail, 8000),
            });
        }
    }

    Ok(results)
}
```

Also add a helper `classify_item` that slots an item into `CollectedItems`:

```rust
fn classify_item<'a>(
    item: &'a serde_json::Value,
    items: &mut CollectedItems<'a>,
) {
    let Some(name) = item.get("name").and_then(|n| n.as_str()) else { return };
    if item.get("visibility").and_then(|v| v.as_str()) != Some("public") {
        return;
    }
    let Some(inner) = item.get("inner").and_then(|i| i.as_object()) else { return };
    let docs = item.get("docs").and_then(|d| d.as_str()).unwrap_or("");

    let kind = if inner.contains_key("trait_") { "trait_" }
        else if inner.contains_key("struct") { "struct" }
        else if inner.contains_key("enum") { "enum" }
        else if inner.contains_key("function") { "function" }
        else if inner.contains_key("macro") { "macro" }
        else { return };

    let entry = ItemEntry { name, docs, inner, kind };
    match kind {
        "trait_" => items.traits.push(entry),
        "struct" => items.structs.push(entry),
        "enum" => items.enums.push(entry),
        "function" => items.functions.push(entry),
        "macro" => items.macros.push(entry),
        _ => {}
    }
}
```

**Step 10: Run test to verify it passes**

Run: `cargo test --lib -- cargo_doc::tests::test_parse_rustdoc_json_with_modules`
Expected: PASS

**Step 11: Wire `parse_rustdoc_json_modules` into `generate_cargo_docs`**

In `src/indexer/cargo_doc.rs`, update `generate_cargo_docs` return type from `Vec<(String, String)>` to `Vec<(String, Vec<ModuleDoc>)>`. Update the caller in `src/indexer/docs.rs` `fetch_docs` to store each `ModuleDoc` as a separate doc row using `store_doc_with_module`.

In `generate_cargo_docs` (line ~80), change:
```rust
// Before:
let formatted = match parse_rustdoc_json(&content, dep_name) { ... };
if !formatted.is_empty() {
    results.push((dep_name.clone(), formatted));
}

// After:
let modules = match parse_rustdoc_json_modules(&content, dep_name) { ... };
if !modules.is_empty() {
    results.push((dep_name.clone(), modules));
}
```

In `src/indexer/docs.rs` `fetch_docs` (around line 188-198), update the cargo_doc result handling:

```rust
// Before:
for (name, content) in docs {
    if let Some(p) = pending.iter().find(|p| p.name == name) {
        results.push(FetchedDoc {
            dep_id: p.dep_id,
            source: "cargo_doc",
            content: truncate_content(&content),
        });
    }
}

// After:
for (name, module_docs) in docs {
    if let Some(p) = pending.iter().find(|p| p.name == name) {
        for md in module_docs {
            results.push(FetchedDoc {
                dep_id: p.dep_id,
                source: "cargo_doc",
                content: truncate_content(&md.content),
                module: md.module,
            });
        }
    }
}
```

Add `module: String` field to `FetchedDoc`. Update `store_fetched_docs` to use `store_doc_with_module`. Update network-fetched docs to set `module: String::new()`.

**Step 12: Update `handle_docs` for summary vs detail**

In `src/server/tools/docs.rs`, update the no-topic path:

```rust
// No topic — return summary (module="") for this dependency
let summary = db.get_doc_by_module(dep_name, "")?;
if let Some(doc) = summary {
    let mut output = String::new();
    let _ = writeln!(output, "## Documentation: {dep_name}\n");
    let _ = writeln!(output, "{}\n", doc.content);

    // List available modules
    let all_docs = db.get_docs_for_dependency(dep_name)?;
    let modules: Vec<_> = all_docs.iter()
        .filter(|d| !d.module.is_empty())
        .collect();
    if !modules.is_empty() {
        let _ = writeln!(output, "### Available Modules\n");
        let _ = writeln!(
            output,
            "Use `topic` param to get details for a specific module:\n"
        );
        for m in &modules {
            let _ = writeln!(output, "- {}", m.module);
        }
    }
    return Ok(output);
}
```

Update topic path to check module column first:

```rust
if let Some(topic) = topic {
    // Try exact module match first
    if let Some(doc) = db.get_doc_by_module(dep_name, topic)? {
        let mut output = String::new();
        let _ = writeln!(output, "## {dep_name}::{topic}\n");
        let _ = writeln!(output, "{}\n", doc.content);
        return Ok(output);
    }
    // Fall back to FTS search
    // ... existing FTS code ...
}
```

Note: This requires adding `module` field to `DocResult`. Update the struct and `row_to_doc_result` in `src/db.rs`, and update the SELECT queries to include `d.module`.

**Step 13: Run full test suite and lint**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings.

**Step 14: Commit**

```bash
git add src/db.rs src/indexer/cargo_doc.rs src/indexer/docs.rs src/server/tools/docs.rs
git commit -m "feat: two-tier doc storage with summary and per-module detail

Rustdoc JSON now parsed into crate summary + per-module docs.
handle_docs returns summary by default, module detail when topic
matches a module name, FTS search as fallback."
```

---

### Task 3: Qualified path reference resolution

Parse `use` statements to build import maps, then resolve refs against them instead of global name matching.

**Files:**
- Modify: `src/indexer/parser.rs` (add `extract_import_map`, update `collect_body_refs`, add `target_file` to `SymbolRef`)
- Modify: `src/db.rs` (add `get_symbol_id_in_crate`)
- Modify: `src/indexer/mod.rs` (pass crate file info to `extract_refs`)
- Test: `src/indexer/parser.rs` test module

**Step 1: Write failing test for `extract_import_map`**

Add to `src/indexer/parser.rs` test module:

```rust
#[test]
fn test_extract_import_map_direct() {
    let source = r#"
use crate::config::Config;
use crate::db::Database;

fn main() {}
"#;
    let map = extract_import_map_from_source(source).unwrap();
    assert_eq!(map.get("Config").unwrap().qualified_path, "crate::config::Config");
    assert_eq!(map.get("Database").unwrap().qualified_path, "crate::db::Database");
}

#[test]
fn test_extract_import_map_alias() {
    let source = r#"
use anyhow::Result as AnyResult;

fn main() {}
"#;
    let map = extract_import_map_from_source(source).unwrap();
    assert_eq!(map.get("AnyResult").unwrap().qualified_path, "anyhow::Result");
}

#[test]
fn test_extract_import_map_nested() {
    let source = r#"
use std::collections::{HashMap, HashSet};

fn main() {}
"#;
    let map = extract_import_map_from_source(source).unwrap();
    assert_eq!(map.get("HashMap").unwrap().qualified_path, "std::collections::HashMap");
    assert_eq!(map.get("HashSet").unwrap().qualified_path, "std::collections::HashSet");
}

#[test]
fn test_extract_import_map_skips_glob() {
    let source = r#"
use std::collections::*;

fn main() {}
"#;
    let map = extract_import_map_from_source(source).unwrap();
    // Glob imports should not create entries
    assert!(map.is_empty());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib -- parser::tests::test_extract_import_map`
Expected: FAIL — function doesn't exist.

**Step 3: Implement `ImportInfo` and `extract_import_map`**

Add to `src/indexer/parser.rs`:

```rust
#[derive(Debug, Clone)]
pub struct ImportInfo {
    pub qualified_path: String,
}

/// Parse `use` declarations and build a map from short name to qualified path.
/// Skips glob imports.
pub fn extract_import_map(root: &Node, source: &str) -> HashMap<String, ImportInfo> {
    let mut map = HashMap::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() != "use_declaration" {
            continue;
        }
        // The use_declaration has a child that is the use tree
        collect_use_paths(&child, source, "", &mut map);
    }
    map
}

fn collect_use_paths(
    node: &Node,
    source: &str,
    prefix: &str,
    map: &mut HashMap<String, ImportInfo>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" | "scoped_use_list" => {
                // Build the path prefix from the scope part
                let mut scope_cursor = child.walk();
                let children: Vec<_> = child.children(&mut scope_cursor).collect();

                if child.kind() == "scoped_use_list" {
                    // e.g., std::collections::{HashMap, HashSet}
                    let path_part = children.iter()
                        .take_while(|c| c.kind() != "use_list")
                        .map(|c| node_text(c, source))
                        .collect::<Vec<_>>()
                        .join("");
                    // Remove trailing ::
                    let path = if let Some(stripped) = path_part.strip_suffix("::") {
                        format!("{prefix}{stripped}")
                    } else {
                        format!("{prefix}{path_part}")
                    };
                    if let Some(list) = children.iter().find(|c| c.kind() == "use_list") {
                        collect_use_list(list, source, &path, map);
                    }
                } else {
                    // e.g., crate::config::Config
                    let full = node_text(&child, source);
                    let name = full.rsplit("::").next().unwrap_or(&full);
                    map.insert(name.to_string(), ImportInfo {
                        qualified_path: format!("{prefix}{full}"),
                    });
                }
            }
            "use_as_clause" => {
                // e.g., anyhow::Result as AnyResult
                let mut clause_cursor = child.walk();
                let clause_children: Vec<_> = child.children(&mut clause_cursor).collect();
                // First child is the path, last child after "as" is the alias
                let path_node = clause_children.first();
                let alias_node = clause_children.last();
                if let (Some(path_n), Some(alias_n)) = (path_node, alias_node) {
                    if path_n.id() != alias_n.id() {
                        let path = node_text(path_n, source);
                        let alias = node_text(alias_n, source);
                        map.insert(alias, ImportInfo {
                            qualified_path: format!("{prefix}{path}"),
                        });
                    }
                }
            }
            "use_wildcard" => {
                // Skip glob imports
            }
            "identifier" if node.kind() == "use_declaration" => {
                // Simple: `use Foo;` (rare)
                let name = node_text(&child, source);
                map.insert(name.clone(), ImportInfo {
                    qualified_path: format!("{prefix}{name}"),
                });
            }
            _ => {
                collect_use_paths(&child, source, prefix, map);
            }
        }
    }
}

fn collect_use_list(
    node: &Node,
    source: &str,
    prefix: &str,
    map: &mut HashMap<String, ImportInfo>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "type_identifier" => {
                let name = node_text(&child, source);
                map.insert(name.clone(), ImportInfo {
                    qualified_path: format!("{prefix}::{name}"),
                });
            }
            "use_as_clause" => {
                let mut clause_cursor = child.walk();
                let children: Vec<_> = child.children(&mut clause_cursor).collect();
                let path_node = children.first();
                let alias_node = children.last();
                if let (Some(p), Some(a)) = (path_node, alias_node) {
                    if p.id() != a.id() {
                        let path = node_text(p, source);
                        let alias = node_text(a, source);
                        map.insert(alias, ImportInfo {
                            qualified_path: format!("{prefix}::{path}"),
                        });
                    }
                }
            }
            "scoped_use_list" => {
                collect_use_paths(&child, source, prefix, map);
            }
            "self" => {
                // `use foo::{self}` imports `foo` itself
                let name = prefix.rsplit("::").next().unwrap_or(prefix);
                map.insert(name.to_string(), ImportInfo {
                    qualified_path: prefix.to_string(),
                });
            }
            _ => {}
        }
    }
}

/// Convenience wrapper for testing — parses source and extracts import map.
pub fn extract_import_map_from_source(
    source: &str,
) -> Result<HashMap<String, ImportInfo>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {e}"))?;
    let tree = parser.parse(source, None).ok_or("Failed to parse")?;
    Ok(extract_import_map(&tree.root_node(), source))
}
```

**Step 4: Run import map tests**

Run: `cargo test --lib -- parser::tests::test_extract_import_map`
Expected: All PASS. If any fail, debug the tree-sitter AST node kinds — use `cargo test` output to see which assertion fails, then adjust the node traversal.

**Step 5: Commit import map parsing**

```bash
git add src/indexer/parser.rs
git commit -m "feat: parse use statements into import map for qualified path resolution"
```

**Step 6: Add `target_file` to `SymbolRef`**

Update `SymbolRef` in `src/indexer/parser.rs`:

```rust
pub struct SymbolRef {
    pub source_name: String,
    pub source_file: String,
    pub target_name: String,
    pub target_file: Option<String>,
    pub kind: RefKind,
}
```

Update all places that construct `SymbolRef` (in `collect_body_refs`) to add `target_file: None` initially.

**Step 7: Write failing test for qualified ref resolution**

Add to `src/indexer/parser.rs` test module:

```rust
#[test]
fn test_extract_refs_qualified() {
    let source = r#"
use crate::config::Config;

pub fn setup() -> Config {
    Config::new()
}
"#;
    let mut known = std::collections::HashSet::new();
    known.insert("Config".to_string());
    known.insert("new".to_string());

    let refs = extract_refs(source, "src/main.rs", &known).unwrap();
    let config_ref = refs.iter().find(|r| r.target_name == "Config").unwrap();
    // Should have resolved via import map — no target_file yet
    // (target_file resolution needs file mapping, but the ref should exist)
    assert_eq!(config_ref.source_name, "setup");
}

#[test]
fn test_extract_refs_does_not_match_local_shadowing() {
    let source = r#"
pub fn process() {
    let config = 42;
    println!("{}", config);
}
"#;
    let mut known = std::collections::HashSet::new();
    known.insert("config".to_string());

    let refs = extract_refs(source, "src/main.rs", &known).unwrap();
    // "config" is a local variable — should NOT create a ref
    let config_refs: Vec<_> = refs.iter().filter(|r| r.target_name == "config").collect();
    assert!(config_refs.is_empty());
}
```

**Step 8: Run tests**

Run: `cargo test --lib -- parser::tests::test_extract_refs_qualified && cargo test --lib -- parser::tests::test_extract_refs_does_not_match_local_shadowing`

The second test should already pass (locals are filtered by `collect_locals`). The first should pass too since it's testing basic behavior. Verify both pass.

**Step 9: Update `collect_body_refs` to use import map**

Modify `extract_refs` to build the import map and pass it to `collect_refs`:

```rust
pub fn extract_refs<S: std::hash::BuildHasher>(
    source: &str,
    file_path: &str,
    known_symbols: &std::collections::HashSet<String, S>,
) -> Result<Vec<SymbolRef>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {e}"))?;
    let tree = parser.parse(source, None).ok_or("Failed to parse source")?;
    let root = tree.root_node();
    let import_map = extract_import_map(&root, source);
    let mut refs = Vec::new();
    collect_refs(&root, source, file_path, known_symbols, &import_map, &mut refs);
    Ok(refs)
}
```

Update `collect_refs` and `collect_body_refs` signatures to accept `import_map: &HashMap<String, ImportInfo>`. In `collect_body_refs`, after the import map lookup, try to resolve `target_file` from the qualified path:

```rust
// In collect_body_refs, when building a SymbolRef:
let target_file = import_map.get(&name).and_then(|info| {
    // Convert crate::foo::bar::Baz to src/foo/bar.rs (heuristic)
    qualified_path_to_file(&info.qualified_path)
});

refs.push(SymbolRef {
    source_name: fn_name.to_string(),
    source_file: file_path.to_string(),
    target_name: name,
    target_file,
    kind: ref_kind,
});
```

Add the path heuristic helper:

```rust
/// Convert a qualified crate path like `crate::config::Config` to a file path
/// like `src/config.rs`. Returns None for external crate paths.
fn qualified_path_to_file(qualified_path: &str) -> Option<String> {
    let path = qualified_path.strip_prefix("crate::")?;
    // Split into segments, drop the last one (it's the item name)
    let segments: Vec<&str> = path.split("::").collect();
    if segments.len() < 2 {
        return None;
    }
    let module_segments = &segments[..segments.len() - 1];
    // Try src/foo/bar.rs or src/foo/bar/mod.rs
    Some(format!("src/{}.rs", module_segments.join("/")))
}
```

**Step 10: Update `store_symbol_refs` to use `target_file`**

In `src/db.rs`, update `store_symbol_refs`:

```rust
pub fn store_symbol_refs(&self, refs: &[crate::indexer::parser::SymbolRef]) -> SqlResult<u64> {
    let mut count = 0;
    for r in refs {
        let source_id = self.get_symbol_id(&r.source_name, &r.source_file)?;
        let target_id = if let Some(target_file) = &r.target_file {
            // Precise: use file-qualified lookup
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

**Step 11: Run full test suite and lint**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings. Fix any issues.

**Step 12: Commit**

```bash
git add src/indexer/parser.rs src/db.rs src/indexer/mod.rs
git commit -m "feat: qualified path reference resolution via use statement parsing

extract_refs now builds an import map from use declarations and
resolves target_file when possible. store_symbol_refs uses file-
qualified lookup before falling back to global name matching."
```

---

### Task 4: `diff_impact` tool

New MCP tool that parses git diff output, maps changed lines to symbols, and runs batch impact analysis.

**Files:**
- Create: `src/server/tools/diff_impact.rs`
- Modify: `src/server/tools/mod.rs` (add module)
- Modify: `src/server/mod.rs` (register tool)
- Modify: `src/db.rs` (add `get_symbols_at_lines`)
- Test: `src/server/tools/diff_impact.rs`, `src/db.rs`

**Step 1: Write failing test for `get_symbols_at_lines`**

Add to `src/db.rs` test module:

```rust
#[test]
fn test_get_symbols_at_lines() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
    store_symbols(
        &db,
        file_id,
        &[
            Symbol {
                name: "foo".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn foo()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
            },
            Symbol {
                name: "bar".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 12,
                line_end: 20,
                signature: "pub fn bar()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
            },
        ],
    )
    .unwrap();

    // Line 5 falls within foo (1-10)
    let syms = db.get_symbols_at_lines("src/lib.rs", &[(5, 5)]).unwrap();
    assert_eq!(syms.len(), 1);
    assert_eq!(syms[0].name, "foo");

    // Line 15 falls within bar (12-20)
    let syms = db.get_symbols_at_lines("src/lib.rs", &[(15, 15)]).unwrap();
    assert_eq!(syms.len(), 1);
    assert_eq!(syms[0].name, "bar");

    // Range spanning both
    let syms = db.get_symbols_at_lines("src/lib.rs", &[(1, 20)]).unwrap();
    assert_eq!(syms.len(), 2);

    // Line outside any symbol
    let syms = db.get_symbols_at_lines("src/lib.rs", &[(11, 11)]).unwrap();
    assert!(syms.is_empty());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib -- db::tests::test_get_symbols_at_lines`
Expected: FAIL — method doesn't exist.

**Step 3: Implement `get_symbols_at_lines`**

Add to `src/db.rs`:

```rust
/// Find symbols whose line range overlaps any of the given (start, end) ranges.
pub fn get_symbols_at_lines(
    &self,
    file_path: &str,
    line_ranges: &[(i64, i64)],
) -> SqlResult<Vec<StoredSymbol>> {
    if line_ranges.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = self.conn.prepare(
        "SELECT s.name, s.kind, s.visibility, f.path, \
                s.line_start, s.line_end, s.signature, \
                s.doc_comment, s.body, s.details, s.attributes \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         WHERE f.path = ?1 AND s.line_start <= ?3 AND s.line_end >= ?2",
    )?;

    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (start, end) in line_ranges {
        let mut rows = stmt.query(params![file_path, start, end])?;
        while let Some(row) = rows.next()? {
            let sym = row_to_stored_symbol(row)?;
            if seen.insert((sym.name.clone(), sym.line_start)) {
                results.push(sym);
            }
        }
    }
    Ok(results)
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib -- db::tests::test_get_symbols_at_lines`
Expected: PASS

**Step 5: Commit**

```bash
git add src/db.rs
git commit -m "feat: add get_symbols_at_lines for mapping diff hunks to symbols"
```

**Step 6: Write failing test for diff parsing**

Create `src/server/tools/diff_impact.rs`:

```rust
use crate::db::Database;
use std::fmt::Write;

/// A changed region in a file.
#[derive(Debug, Clone)]
struct DiffHunk {
    file_path: String,
    start_line: i64,
    end_line: i64,
}

/// Parse unified diff output into file/line-range pairs.
fn parse_diff(diff_output: &str) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_file: Option<String> = None;

    for line in diff_output.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            current_file = Some(rest.to_string());
        } else if line.starts_with("@@ ") {
            // Parse @@ -old_start,old_count +new_start,new_count @@
            let Some(file) = &current_file else { continue };
            let Some(plus_part) = line.split('+').nth(1) else { continue };
            let nums = plus_part.split(' ').next().unwrap_or("");
            let parts: Vec<&str> = nums.split(',').collect();
            let start: i64 = parts.first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            let count: i64 = parts.get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            let end = start + count.saturating_sub(1);
            hunks.push(DiffHunk {
                file_path: file.clone(),
                start_line: start,
                end_line: end,
            });
        }
    }
    hunks
}

pub fn handle_diff_impact(
    db: &Database,
    repo_path: &std::path::Path,
    git_ref: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    // Build git diff command
    let diff_output = run_git_diff(repo_path, git_ref)?;
    if diff_output.trim().is_empty() {
        return Ok("No changes detected.".to_string());
    }

    let hunks = parse_diff(&diff_output);
    if hunks.is_empty() {
        return Ok("No file changes found in diff.".to_string());
    }

    // Group hunks by file
    let mut by_file: std::collections::BTreeMap<&str, Vec<(i64, i64)>> =
        std::collections::BTreeMap::new();
    for hunk in &hunks {
        by_file
            .entry(&hunk.file_path)
            .or_default()
            .push((hunk.start_line, hunk.end_line));
    }

    // Map changed lines to symbols
    let mut changed_symbols: Vec<(String, String)> = Vec::new(); // (name, file)
    let mut output = String::new();
    let _ = writeln!(output, "## Changed Symbols\n");

    for (file, ranges) in &by_file {
        // Only look at .rs files
        if !file.ends_with(".rs") {
            continue;
        }
        let symbols = db.get_symbols_at_lines(file, ranges)?;
        if symbols.is_empty() {
            continue;
        }
        let _ = writeln!(output, "### {file}\n");
        for sym in &symbols {
            let _ = writeln!(
                output,
                "- **{}** ({}, line {}-{})",
                sym.name, sym.kind, sym.line_start, sym.line_end
            );
            changed_symbols.push((sym.name.clone(), sym.file_path.clone()));
        }
        let _ = writeln!(output);
    }

    if changed_symbols.is_empty() {
        return Ok(
            "Changes detected but no indexed symbols overlap the changed lines."
                .to_string(),
        );
    }

    // Batch impact analysis
    let _ = writeln!(output, "## Downstream Impact\n");
    let mut any_impact = false;

    for (sym_name, _sym_file) in &changed_symbols {
        let dependents = db.impact_dependents(sym_name)?;
        // Filter out self-references and other changed symbols
        let external: Vec<_> = dependents
            .iter()
            .filter(|d| !changed_symbols.iter().any(|(n, _)| n == &d.name))
            .collect();
        if external.is_empty() {
            continue;
        }
        any_impact = true;
        let _ = writeln!(output, "### {sym_name}\n");
        for dep in &external {
            if dep.via.is_empty() {
                let _ = writeln!(
                    output,
                    "- **{}** ({}) — depth {}",
                    dep.name, dep.file_path, dep.depth
                );
            } else {
                let _ = writeln!(
                    output,
                    "- **{}** ({}) — depth {}, via {}",
                    dep.name, dep.file_path, dep.depth, dep.via
                );
            }
        }
        let _ = writeln!(output);
    }

    if !any_impact {
        let _ = writeln!(output, "No downstream dependents found for changed symbols.");
    }

    Ok(output)
}

fn run_git_diff(
    repo_path: &std::path::Path,
    git_ref: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("diff");
    if let Some(r) = git_ref {
        // If single ref (no ".."), compare ref..HEAD
        if r.contains("..") {
            cmd.arg(r);
        } else {
            cmd.arg(format!("{r}..HEAD"));
        }
    }
    cmd.arg("--unified=0"); // minimal context for precise line mapping
    cmd.current_dir(repo_path);

    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff failed: {stderr}").into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_parse_diff_basic() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
index abc123..def456 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,3 +10,5 @@ fn existing() {
+    new_line();
+    another_line();
@@ -30,0 +32,2 @@ fn other() {
+    added();
+    more();
";
        let hunks = parse_diff(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].file_path, "src/lib.rs");
        assert_eq!(hunks[0].start_line, 10);
        assert_eq!(hunks[0].end_line, 14);
        assert_eq!(hunks[1].start_line, 32);
        assert_eq!(hunks[1].end_line, 33);
    }

    #[test]
    fn test_parse_diff_multiple_files() {
        let diff = "\
diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
@@ -5,2 +5,3 @@
 context
diff --git a/src/b.rs b/src/b.rs
--- a/src/b.rs
+++ b/src/b.rs
@@ -1,1 +1,2 @@
 context
";
        let hunks = parse_diff(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].file_path, "src/a.rs");
        assert_eq!(hunks[1].file_path, "src/b.rs");
    }

    #[test]
    fn test_parse_diff_empty() {
        let hunks = parse_diff("");
        assert!(hunks.is_empty());
    }

    #[test]
    fn test_handle_diff_impact_with_symbols() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        crate::indexer::store::store_symbols(
            &db,
            file_id,
            &[
                crate::indexer::parser::Symbol {
                    name: "changed_fn".into(),
                    kind: crate::indexer::parser::SymbolKind::Function,
                    visibility: crate::indexer::parser::Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 5,
                    line_end: 15,
                    signature: "pub fn changed_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                },
                crate::indexer::parser::Symbol {
                    name: "caller_fn".into(),
                    kind: crate::indexer::parser::SymbolKind::Function,
                    visibility: crate::indexer::parser::Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 20,
                    line_end: 30,
                    signature: "pub fn caller_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                },
            ],
        )
        .unwrap();

        // caller_fn depends on changed_fn
        let changed_id = db.get_symbol_id("changed_fn", "src/lib.rs").unwrap().unwrap();
        let caller_id = db.get_symbol_id("caller_fn", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(caller_id, changed_id, "call").unwrap();

        // Simulate a diff that touches lines 8-12 (within changed_fn)
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -8,2 +8,3 @@
+    new_code();
";
        let hunks = parse_diff(diff);
        assert_eq!(hunks.len(), 1);

        let syms = db.get_symbols_at_lines("src/lib.rs", &[(8, 10)]).unwrap();
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "changed_fn");

        // Verify impact chain works
        let deps = db.impact_dependents("changed_fn").unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "caller_fn");
    }
}
```

**Step 7: Run tests to verify they pass**

Run: `cargo test --lib -- diff_impact::tests`
Expected: PASS

**Step 8: Register the tool in the MCP server**

Add to `src/server/tools/mod.rs`:

```rust
pub mod diff_impact;
```

In `src/server/mod.rs`, add the tool registration. Add a new param struct:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct DiffImpactParams {
    /// Git ref range (e.g. "HEAD~3..HEAD", "main"). Omit for unstaged changes.
    git_ref: Option<String>,
}
```

Add the tool method to the `#[tool_router]` impl block:

```rust
#[tool(
    name = "diff_impact",
    description = "Analyze impact of code changes from a git diff. Shows which symbols were modified and their downstream dependents. Provide a git ref range like 'HEAD~3..HEAD' or 'main', or omit for unstaged changes."
)]
async fn diff_impact(
    &self,
    Parameters(params): Parameters<DiffImpactParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(git_ref = ?params.git_ref, "Tool call: diff_impact");
    let _guard = crate::status::StatusGuard::new("diff_impact");
    self.refresh()?;
    let db = self.lock_db()?;
    let repo_path = &self.config.repo_path;
    let result = tools::diff_impact::handle_diff_impact(
        &db,
        repo_path,
        params.git_ref.as_deref(),
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

Update `ServerInfo` instructions to mention `diff_impact`.

**Step 9: Update skill file generator**

In `src/indexer/mod.rs` `generate_claude_skill`, add:

```rust
let _ = writeln!(
    out,
    "- **diff_impact** — Analyze impact of git changes. \
     Shows modified symbols and their downstream dependents."
);
```

**Step 10: Run full test suite and lint**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings.

**Step 11: Commit**

```bash
git add src/server/tools/diff_impact.rs src/server/tools/mod.rs src/server/mod.rs src/db.rs src/indexer/mod.rs
git commit -m "feat: add diff_impact tool for change-set aware impact analysis

New MCP tool parses git diff output, maps changed lines to symbols,
and runs batch impact analysis. Accepts git ref range or defaults
to unstaged changes."
```

---

## Verification Checklist

After all tasks are complete:

1. `cargo test` — all tests pass
2. `cargo clippy --all-targets --all-features -- -D warnings` — no warnings
3. `cargo fmt --all -- --check` — properly formatted
4. Test manually: `RUST_LOG=info cargo run -- /path/to/test/repo` and call each tool via MCP
5. Verify `diff_impact` works: make a change, call the tool, check output
6. Verify docs persist: index, re-index, check docs still exist
7. Verify qualified refs: check that cross-crate `new()` no longer creates false refs
