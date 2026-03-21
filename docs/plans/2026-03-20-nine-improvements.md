# Nine illu-rs Improvements Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add 9 features to illu-rs: attribute/signature filters on query, impl_type in StoredSymbol, smarter noisy symbol filtering, and 5 new tools (freshness, callpath, batch_context, unused, crate_graph).

**Architecture:** Changes span three layers: data model (StoredSymbol gains impl_type), query enhancements (attribute + signature filters on existing query tool), and 5 new MCP tool handlers following the existing pattern. Each new tool gets a handler file in `src/server/tools/`, a params struct in `src/server/mod.rs`, and registration in the `#[tool_router]` impl block.

**Tech Stack:** Rust, rusqlite, tree-sitter, rmcp (MCP server framework), schemars (JSON schema for tool params)

---

### Task 1: Add `impl_type` to `StoredSymbol`

**Files:**
- Modify: `src/db.rs:51-65` (row_to_stored_symbol)
- Modify: `src/db.rs:1425-1437` (StoredSymbol struct)
- Modify: `src/server/tools/context.rs:64-85` (render_symbol_header)

**Step 1: Add field to StoredSymbol struct**

In `src/db.rs`, add `impl_type` field to `StoredSymbol`:

```rust
pub struct StoredSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub body: Option<String>,
    pub details: Option<String>,
    pub attributes: Option<String>,
    pub impl_type: Option<String>,  // NEW
}
```

**Step 2: Update `row_to_stored_symbol` to read column 11**

```rust
fn row_to_stored_symbol(row: &rusqlite::Row) -> SqlResult<StoredSymbol> {
    Ok(StoredSymbol {
        name: row.get(0)?,
        kind: parse_kind(&row.get::<_, String>(1)?)?,
        visibility: parse_visibility(&row.get::<_, String>(2)?)?,
        file_path: row.get(3)?,
        line_start: row.get(4)?,
        line_end: row.get(5)?,
        signature: row.get(6)?,
        doc_comment: row.get(7)?,
        body: row.get(8)?,
        details: row.get(9)?,
        attributes: row.get(10)?,
        impl_type: row.get(11)?,  // NEW
    })
}
```

**Step 3: Update ALL SQL SELECTs that feed `row_to_stored_symbol`**

Every query that selects `s.name, s.kind, s.visibility, f.path, s.line_start, s.line_end, s.signature, s.doc_comment, s.body, s.details, s.attributes` must add `, s.impl_type` at the end:

- `search_symbols` (two SQL branches — FTS-safe with trigram, FTS-safe without trigram, and FTS-unsafe)
- `search_symbols_by_impl`
- `search_symbols_by_attribute`
- `get_symbols_at_lines`
- `get_symbols_by_path_prefix_filtered`

Find all with: `rg "s\.attributes" src/db.rs` — every SELECT that ends with `s.attributes` needs `, s.impl_type` appended.

**Step 4: Update `render_symbol_header` in context.rs**

After the attributes line, add impl_type display:

```rust
if let Some(impl_type) = &sym.impl_type {
    let _ = writeln!(output, "- **Impl:** {impl_type}");
}
```

**Step 5: Run tests**

Run: `cargo test --lib -- db::tests`
Run: `cargo test --test self_index`
Expected: all pass

**Step 6: Commit**

```
feat: add impl_type field to StoredSymbol for method context
```

---

### Task 2: Shrink `NOISY_SYMBOL_NAMES`

**Files:**
- Modify: `src/indexer/parser.rs:660-685` (NOISY_SYMBOL_NAMES const)

**Step 1: Replace the NOISY_SYMBOL_NAMES list**

Keep only derive/trait plumbing that are almost never user-written:

```rust
const NOISY_SYMBOL_NAMES: &[&str] = &[
    "fmt",
    "eq",
    "ne",
    "partial_cmp",
    "cmp",
    "hash",
    "drop",
    "deref",
    "deref_mut",
    "as_ref",
    "as_mut",
    "borrow",
    "borrow_mut",
    "to_string",
    "to_owned",
    "try_from",
    "try_into",
];
```

Removed from filter: `new`, `from`, `into`, `clone`, `default`, `build`, `init`. These are user-written constructors/conversions that matter for impact analysis.

**Step 2: Run tests**

Run: `cargo test`
Expected: all pass. Some tests may now find more refs than before — that's correct behavior.

**Step 3: Commit**

```
feat: track new/from/into/clone/default in symbol refs

These are user-written constructors and conversions that matter for
impact analysis. The impl_type-aware ref resolution disambiguates
Database::new from Config::new.
```

---

### Task 3: Add attribute and signature filters to `query` tool

**Files:**
- Modify: `src/server/mod.rs:74-79` (QueryParams struct)
- Modify: `src/server/mod.rs:131-156` (query tool handler)
- Modify: `src/server/tools/query.rs` (handle_query + format_symbols)
- Modify: `src/db.rs` (add `search_symbols_by_signature`)

**Step 1: Add `search_symbols_by_signature` to Database**

In `src/db.rs`, after `search_symbols_by_attribute`:

```rust
pub fn search_symbols_by_signature(
    &self,
    pattern: &str,
) -> SqlResult<Vec<StoredSymbol>> {
    let like_pattern = format!("%{}%", escape_like(pattern));
    let mut stmt = self.conn.prepare(
        "SELECT s.name, s.kind, s.visibility, f.path, \
                s.line_start, s.line_end, s.signature, \
                s.doc_comment, s.body, s.details, s.attributes, \
                s.impl_type \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         WHERE s.signature LIKE ?1 ESCAPE '\\' \
         ORDER BY s.name \
         LIMIT 50",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query(params![like_pattern])?;
    while let Some(row) = rows.next()? {
        results.push(row_to_stored_symbol(row)?);
    }
    Ok(results)
}
```

**Step 2: Add params to QueryParams**

In `src/server/mod.rs`:

```rust
#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    query: String,
    scope: Option<String>,
    kind: Option<String>,
    /// Filter by attribute/derive (e.g. "test", "derive(Serialize)")
    attribute: Option<String>,
    /// Filter by signature pattern (e.g. "&Database", "-> Result")
    signature: Option<String>,
}
```

**Step 3: Pass new params through the MCP handler**

In `src/server/mod.rs`, update the query tool handler to pass new params:

```rust
let result = tools::query::handle_query(
    &db,
    &params.query,
    params.scope.as_deref(),
    params.kind.as_deref(),
    params.attribute.as_deref(),
    params.signature.as_deref(),
)
.map_err(to_mcp_err)?;
```

**Step 4: Update `handle_query` signature and `format_symbols`**

In `src/server/tools/query.rs`:

```rust
pub fn handle_query(
    db: &Database,
    query: &str,
    scope: Option<&str>,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let scope = scope.unwrap_or("all");
    let mut output = String::new();

    match scope {
        "symbols" => format_symbols(db, query, kind, attribute, signature, &mut output)?,
        "docs" => format_docs(db, query, &mut output)?,
        "files" => format_files(db, query, &mut output)?,
        "all" => {
            format_symbols(db, query, kind, attribute, signature, &mut output)?;
            format_docs(db, query, &mut output)?;
        }
        other => {
            return Err(
                format!("Unknown scope: '{other}'. Valid: symbols, docs, files, all").into(),
            );
        }
    }

    if output.is_empty() {
        let _ = write!(output, "No results found for '{query}'.");
    }

    Ok(output)
}
```

Update `format_symbols` to support attribute and signature filtering:

```rust
fn format_symbols(
    db: &Database,
    query: &str,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    output: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let all_symbols = if let Some(attr) = attribute {
        // Attribute search mode: use attribute filter, then
        // additionally filter by query name if non-empty
        let mut syms = db.search_symbols_by_attribute(attr)?;
        if !query.is_empty() {
            let q_lower = query.to_lowercase();
            syms.retain(|s| s.name.to_lowercase().contains(&q_lower));
        }
        syms
    } else if let Some(sig) = signature {
        let mut syms = db.search_symbols_by_signature(sig)?;
        if !query.is_empty() {
            let q_lower = query.to_lowercase();
            syms.retain(|s| s.name.to_lowercase().contains(&q_lower));
        }
        syms
    } else {
        db.search_symbols(query)?
    };

    let symbols: Vec<_> = if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        all_symbols
            .into_iter()
            .filter(|s| s.kind.to_string().to_lowercase() == k_lower)
            .collect()
    } else {
        all_symbols
            .into_iter()
            .filter(|s| {
                s.kind != crate::indexer::parser::SymbolKind::Use
                    && s.kind != crate::indexer::parser::SymbolKind::Mod
                    && s.kind != crate::indexer::parser::SymbolKind::EnumVariant
            })
            .collect()
    };

    if !symbols.is_empty() {
        output.push_str("## Symbols\n\n");
        for sym in &symbols {
            let _ = writeln!(
                output,
                "- **{}** ({}) at {}:{}-{}\n  `{}`",
                sym.name, sym.kind, sym.file_path,
                sym.line_start, sym.line_end, sym.signature,
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

**Step 5: Update CLI command and main.rs call site**

In `src/main.rs`, update `handle_query` call to pass `None` for new params:

```rust
let result = handle_query(&db, &search, Some(&scope), kind.as_deref(), None, None)?;
```

**Step 6: Update existing tests**

All existing test calls to `handle_query` need two extra `None` args:

```rust
handle_query(&db, "parse", Some("symbols"), None, None, None)
```

**Step 7: Run tests**

Run: `cargo test --lib -- query::tests`
Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: all pass

**Step 8: Commit**

```
feat: add attribute and signature filters to query tool
```

---

### Task 4: `freshness` tool

**Files:**
- Create: `src/server/tools/freshness.rs`
- Modify: `src/server/tools/mod.rs` (add module)
- Modify: `src/server/mod.rs` (add FreshnessParams + tool handler)

**Step 1: Create handler**

Create `src/server/tools/freshness.rs`:

```rust
use crate::db::Database;
use std::fmt::Write;
use std::path::Path;
use std::process::Command;

pub fn handle_freshness(
    db: &Database,
    repo_path: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();
    let _ = writeln!(output, "## Index Freshness\n");

    // Get indexed commit hash
    let repo_str = repo_path.to_string_lossy();
    let indexed_hash = db.get_commit_hash(&repo_str)?;

    // Get current HEAD
    let head_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    let current_head = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();

    let indexed = indexed_hash.as_deref().unwrap_or("(none)");
    let _ = writeln!(output, "- **Indexed commit:** `{indexed}`");
    let _ = writeln!(output, "- **Current HEAD:** `{current_head}`");

    let is_current = indexed_hash.as_deref() == Some(current_head.as_str());
    let _ = writeln!(
        output,
        "- **Status:** {}",
        if is_current { "up to date" } else { "**STALE**" }
    );

    // Show changed files if stale
    if !is_current {
        if let Some(hash) = &indexed_hash {
            let diff_output = Command::new("git")
                .args(["diff", "--name-only", hash, "HEAD"])
                .current_dir(repo_path)
                .output()?;
            let changed = String::from_utf8_lossy(&diff_output.stdout);
            let files: Vec<&str> = changed
                .lines()
                .filter(|l| !l.is_empty())
                .collect();
            if !files.is_empty() {
                let _ = writeln!(output, "\n### Changed since index ({} files)\n", files.len());
                for f in &files {
                    let _ = writeln!(output, "- {f}");
                }
            }
        }

        // Also check unstaged changes
        let unstaged = Command::new("git")
            .args(["diff", "--name-only"])
            .current_dir(repo_path)
            .output()?;
        let unstaged_files = String::from_utf8_lossy(&unstaged.stdout);
        let ufiles: Vec<&str> = unstaged_files
            .lines()
            .filter(|l| !l.is_empty())
            .collect();
        if !ufiles.is_empty() {
            let _ = writeln!(output, "\n### Unstaged changes ({} files)\n", ufiles.len());
            for f in &ufiles {
                let _ = writeln!(output, "- {f}");
            }
        }
    }

    Ok(output)
}
```

**Step 2: Register module in tools/mod.rs**

Add `pub mod freshness;` to `src/server/tools/mod.rs`.

**Step 3: Add params struct and tool handler in server/mod.rs**

Add empty params struct (no params needed but rmcp requires one):

```rust
#[derive(Deserialize, JsonSchema)]
struct FreshnessParams {}
```

Add tool handler in the `#[tool_router] impl IlluServer` block:

```rust
#[tool(
    name = "freshness",
    description = "Check if the index is up to date with the current git HEAD. Shows indexed commit, current HEAD, and any changed files."
)]
async fn freshness(
    &self,
    Parameters(_params): Parameters<FreshnessParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!("Tool call: freshness");
    let _guard = crate::status::StatusGuard::new("freshness");
    let db = self.lock_db()?;
    let repo_path = &self.config.repo_path;
    let result = tools::freshness::handle_freshness(&db, repo_path)
        .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

Note: `freshness` intentionally does NOT call `self.refresh()` — it reports the current state, not updates it.

**Step 4: Run tests**

Run: `cargo build`
Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: compiles clean

**Step 5: Commit**

```
feat: add freshness tool to check index staleness
```

---

### Task 5: `callpath` tool

**Files:**
- Create: `src/server/tools/callpath.rs`
- Modify: `src/server/tools/mod.rs` (add module)
- Modify: `src/server/mod.rs` (add CallpathParams + tool handler)
- Modify: `src/db.rs` (add `get_callees_by_name` for BFS)

**Step 1: Add `get_callees_by_name` to Database**

This method returns callees without requiring a source file — needed for BFS traversal where we discover symbols by name:

```rust
pub fn get_callees_by_name(
    &self,
    symbol_name: &str,
) -> SqlResult<Vec<(String, String)>> {
    let mut stmt = self.conn.prepare(
        "SELECT DISTINCT ts.name, f.path \
         FROM symbol_refs sr \
         JOIN symbols ss ON ss.id = sr.source_symbol_id \
         JOIN symbols ts ON ts.id = sr.target_symbol_id \
         JOIN files f ON f.id = ts.file_id \
         WHERE ss.name = ?1",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query(params![symbol_name])?;
    while let Some(row) = rows.next()? {
        results.push((row.get(0)?, row.get(1)?));
    }
    Ok(results)
}
```

**Step 2: Create handler**

Create `src/server/tools/callpath.rs`:

```rust
use crate::db::Database;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write;

pub fn handle_callpath(
    db: &Database,
    from: &str,
    to: &str,
    max_depth: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = max_depth.unwrap_or(10) as usize;

    // Verify both symbols exist
    let from_syms = db.search_symbols(from)?;
    if from_syms.is_empty() {
        return Ok(format!("Source symbol '{from}' not found."));
    }
    let to_syms = db.search_symbols(to)?;
    if to_syms.is_empty() {
        return Ok(format!("Target symbol '{to}' not found."));
    }

    // BFS from `from` following callee edges
    let mut visited: HashSet<String> = HashSet::new();
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    visited.insert(from.to_string());
    queue.push_back((from.to_string(), 0));

    let mut found = false;
    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let callees = db.get_callees_by_name(&current)?;
        for (callee_name, _file) in callees {
            if visited.contains(&callee_name) {
                continue;
            }
            visited.insert(callee_name.clone());
            parent.insert(callee_name.clone(), current.clone());

            if callee_name == to {
                found = true;
                break;
            }
            queue.push_back((callee_name, depth + 1));
        }
        if found {
            break;
        }
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Call Path: {from} → {to}\n");

    if !found {
        let _ = writeln!(
            output,
            "No call path found from `{from}` to `{to}` within depth {max_depth}."
        );
        return Ok(output);
    }

    // Reconstruct path
    let mut path = vec![to.to_string()];
    let mut current = to.to_string();
    while let Some(prev) = parent.get(&current) {
        path.push(prev.clone());
        current = prev.clone();
    }
    path.reverse();

    let _ = writeln!(output, "**Path ({} hops):**\n", path.len() - 1);
    let _ = writeln!(output, "`{}`", path.join(" → "));

    // Show file locations for each symbol in the path
    let _ = writeln!(output, "\n**Locations:**\n");
    for name in &path {
        let syms = db.search_symbols(name)?;
        if let Some(sym) = syms.first() {
            let _ = writeln!(
                output,
                "- **{name}** ({}:{}-{})",
                sym.file_path, sym.line_start, sym.line_end
            );
        }
    }

    Ok(output)
}
```

**Step 3: Register module, add params, add handler**

In `src/server/tools/mod.rs`, add `pub mod callpath;`.

In `src/server/mod.rs`, add params:

```rust
#[derive(Deserialize, JsonSchema)]
struct CallpathParams {
    /// Source symbol name
    from: String,
    /// Target symbol name
    to: String,
    /// Max search depth (default: 10)
    max_depth: Option<i64>,
}
```

Add tool handler:

```rust
#[tool(
    name = "callpath",
    description = "Find the shortest call path between two symbols. Shows how function A reaches function B through the call graph."
)]
async fn callpath(
    &self,
    Parameters(params): Parameters<CallpathParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(from = %params.from, to = %params.to, "Tool call: callpath");
    let _guard = crate::status::StatusGuard::new(
        &format!("callpath ▸ {} → {}", params.from, params.to)
    );
    self.refresh()?;
    let db = self.lock_db()?;
    let result = tools::callpath::handle_callpath(
        &db, &params.from, &params.to, params.max_depth,
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 4: Run tests**

Run: `cargo build`
Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: compiles clean

**Step 5: Commit**

```
feat: add callpath tool for shortest call chain between symbols
```

---

### Task 6: `batch_context` tool

**Files:**
- Create: `src/server/tools/batch_context.rs`
- Modify: `src/server/tools/mod.rs` (add module)
- Modify: `src/server/mod.rs` (add BatchContextParams + tool handler)

**Step 1: Create handler**

Create `src/server/tools/batch_context.rs`:

```rust
use crate::db::Database;
use crate::server::tools::context;

pub fn handle_batch_context(
    db: &Database,
    symbols: &[String],
    full_body: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if symbols.is_empty() {
        return Ok("No symbols provided.".to_string());
    }

    let mut output = String::new();
    for (i, symbol) in symbols.iter().enumerate() {
        if i > 0 {
            output.push_str("\n---\n\n");
        }
        let result = context::handle_context(db, symbol, full_body, None)?;
        output.push_str(&result);
    }

    Ok(output)
}
```

**Step 2: Register module, add params, add handler**

In `src/server/tools/mod.rs`, add `pub mod batch_context;`.

In `src/server/mod.rs`, add params:

```rust
#[derive(Deserialize, JsonSchema)]
struct BatchContextParams {
    /// List of symbol names to get context for
    symbols: Vec<String>,
    /// Return full untruncated source bodies (default: false)
    full_body: Option<bool>,
}
```

Add tool handler:

```rust
#[tool(
    name = "batch_context",
    description = "Get full context for multiple symbols in one call. Returns definition, signature, callers, callees, and docs for each symbol."
)]
async fn batch_context(
    &self,
    Parameters(params): Parameters<BatchContextParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(symbols = ?params.symbols, "Tool call: batch_context");
    let _guard = crate::status::StatusGuard::new("batch_context");
    self.refresh()?;
    let db = self.lock_db()?;
    let full_body = params.full_body.unwrap_or(false);
    let result = tools::batch_context::handle_batch_context(
        &db, &params.symbols, full_body,
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 3: Run tests**

Run: `cargo build`
Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: compiles clean

**Step 4: Commit**

```
feat: add batch_context tool for multi-symbol lookup
```

---

### Task 7: `unused` tool

**Files:**
- Create: `src/server/tools/unused.rs`
- Modify: `src/server/tools/mod.rs` (add module)
- Modify: `src/server/mod.rs` (add UnusedParams + tool handler)
- Modify: `src/db.rs` (add `get_unreferenced_symbols`)

**Step 1: Add `get_unreferenced_symbols` to Database**

In `src/db.rs`:

```rust
pub fn get_unreferenced_symbols(
    &self,
    path_prefix: Option<&str>,
    include_private: bool,
) -> SqlResult<Vec<StoredSymbol>> {
    let prefix = path_prefix.unwrap_or("");
    let like_pattern = format!("{prefix}%");
    let mut stmt = self.conn.prepare(
        "SELECT s.name, s.kind, s.visibility, f.path, \
                s.line_start, s.line_end, s.signature, \
                s.doc_comment, s.body, s.details, s.attributes, \
                s.impl_type \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         LEFT JOIN symbol_refs sr ON sr.target_symbol_id = s.id \
         WHERE sr.id IS NULL \
           AND f.path LIKE ?1 \
           AND s.kind NOT IN ('use', 'mod', 'impl') \
         ORDER BY f.path, s.line_start",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query(params![like_pattern])?;
    while let Some(row) = rows.next()? {
        let sym = row_to_stored_symbol(row)?;
        if !include_private
            && sym.visibility != Visibility::Public
            && sym.visibility != Visibility::PubCrate
        {
            continue;
        }
        results.push(sym);
    }
    Ok(results)
}
```

**Step 2: Create handler**

Create `src/server/tools/unused.rs`:

```rust
use crate::db::Database;
use std::fmt::Write;

pub fn handle_unused(
    db: &Database,
    path: Option<&str>,
    kind: Option<&str>,
    include_private: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut symbols = db.get_unreferenced_symbols(path, include_private)?;

    // Filter by kind if specified
    if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        symbols.retain(|s| s.kind.to_string().to_lowercase() == k_lower);
    }

    // Exclude entry points: main, #[test], #[tokio::test]
    symbols.retain(|s| {
        if s.name == "main" {
            return false;
        }
        if let Some(attrs) = &s.attributes {
            if attrs.contains("test") {
                return false;
            }
        }
        true
    });

    // Exclude enum variants (referenced through their parent enum)
    symbols.retain(|s| {
        s.kind != crate::indexer::parser::SymbolKind::EnumVariant
    });

    let mut output = String::new();
    let _ = writeln!(output, "## Potentially Unused Symbols\n");

    if symbols.is_empty() {
        let _ = writeln!(output, "No unreferenced symbols found.");
        return Ok(output);
    }

    let _ = writeln!(output, "Found {} symbols with no incoming references:\n", symbols.len());

    let mut current_file = String::new();
    for sym in &symbols {
        if sym.file_path != current_file {
            current_file.clone_from(&sym.file_path);
            let _ = writeln!(output, "### {current_file}\n");
        }
        let _ = writeln!(
            output,
            "- **{}** ({}, {}, line {}-{})",
            sym.name, sym.kind, sym.visibility,
            sym.line_start, sym.line_end
        );
    }

    let _ = writeln!(
        output,
        "\n*Note: entry points (main, #[test]) are excluded. \
         Symbols used via macros, dynamic dispatch, or external \
         crates may appear as false positives.*"
    );

    Ok(output)
}
```

**Step 3: Register module, add params, add handler**

In `src/server/tools/mod.rs`, add `pub mod unused;`.

In `src/server/mod.rs`, add params:

```rust
#[derive(Deserialize, JsonSchema)]
struct UnusedParams {
    /// Filter to files under this path prefix (e.g. "src/server/")
    path: Option<String>,
    /// Filter by symbol kind: function, struct, enum, trait, etc.
    kind: Option<String>,
    /// Include private symbols (default: false, shows only pub/pub(crate))
    include_private: Option<bool>,
}
```

Add tool handler:

```rust
#[tool(
    name = "unused",
    description = "Find potentially unused symbols (no incoming references). Excludes entry points like main and #[test]. Useful for dead code detection."
)]
async fn unused(
    &self,
    Parameters(params): Parameters<UnusedParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(path = ?params.path, kind = ?params.kind, "Tool call: unused");
    let _guard = crate::status::StatusGuard::new("unused");
    self.refresh()?;
    let db = self.lock_db()?;
    let result = tools::unused::handle_unused(
        &db,
        params.path.as_deref(),
        params.kind.as_deref(),
        params.include_private.unwrap_or(false),
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 4: Run tests**

Run: `cargo build`
Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: compiles clean

**Step 5: Commit**

```
feat: add unused tool for dead code detection
```

---

### Task 8: `crate_graph` tool

**Files:**
- Create: `src/server/tools/crate_graph.rs`
- Modify: `src/server/tools/mod.rs` (add module)
- Modify: `src/server/mod.rs` (add CrateGraphParams + tool handler)
- Modify: `src/db.rs` (add `get_all_crate_deps`)

**Step 1: Add `get_all_crate_deps` to Database**

In `src/db.rs`:

```rust
pub fn get_all_crate_deps(&self) -> SqlResult<Vec<(String, String)>> {
    let mut stmt = self.conn.prepare(
        "SELECT sc.name, tc.name \
         FROM crate_deps cd \
         JOIN crates sc ON sc.id = cd.source_crate_id \
         JOIN crates tc ON tc.id = cd.target_crate_id \
         ORDER BY sc.name, tc.name",
    )?;
    let mut results = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        results.push((row.get(0)?, row.get(1)?));
    }
    Ok(results)
}
```

**Step 2: Create handler**

Create `src/server/tools/crate_graph.rs`:

```rust
use crate::db::Database;
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn handle_crate_graph(
    db: &Database,
) -> Result<String, Box<dyn std::error::Error>> {
    let crate_count = db.get_crate_count()?;
    if crate_count <= 1 {
        return Ok(
            "Single-crate project — no crate dependency graph.".to_string()
        );
    }

    let crates = db.get_all_crates()?;
    let deps = db.get_all_crate_deps()?;

    let mut output = String::new();
    let _ = writeln!(output, "## Crate Dependency Graph\n");
    let _ = writeln!(output, "**{} crates:**\n", crates.len());

    for c in &crates {
        let _ = writeln!(output, "- **{}** (`{}`)", c.name, c.path);
    }

    // Build adjacency list: source → [targets] (source depends on targets)
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (source, target) in &deps {
        adj.entry(source.clone()).or_default().push(target.clone());
    }

    if adj.is_empty() {
        let _ = writeln!(output, "\nNo inter-crate dependencies.");
        return Ok(output);
    }

    let _ = writeln!(output, "\n### Dependencies\n");
    for (source, targets) in &adj {
        let targets_str = targets.join(", ");
        let _ = writeln!(output, "- **{source}** → {targets_str}");
    }

    // Show leaf crates (no dependencies) and root crates (no dependents)
    let all_names: std::collections::BTreeSet<&str> =
        crates.iter().map(|c| c.name.as_str()).collect();
    let sources: std::collections::BTreeSet<&str> =
        deps.iter().map(|(s, _)| s.as_str()).collect();
    let targets: std::collections::BTreeSet<&str> =
        deps.iter().map(|(_, t)| t.as_str()).collect();

    let leaves: Vec<&&str> = all_names.iter().filter(|n| !sources.contains(**n)).collect();
    let roots: Vec<&&str> = all_names.iter().filter(|n| !targets.contains(**n)).collect();

    if !roots.is_empty() {
        let _ = writeln!(
            output,
            "\n**Root crates** (not depended on): {}",
            roots.iter().map(|n| format!("**{}**", n)).collect::<Vec<_>>().join(", ")
        );
    }
    if !leaves.is_empty() {
        let _ = writeln!(
            output,
            "**Leaf crates** (no deps): {}",
            leaves.iter().map(|n| format!("**{}**", n)).collect::<Vec<_>>().join(", ")
        );
    }

    Ok(output)
}
```

**Step 3: Register module, add params, add handler**

In `src/server/tools/mod.rs`, add `pub mod crate_graph;`.

In `src/server/mod.rs`, add params:

```rust
#[derive(Deserialize, JsonSchema)]
struct CrateGraphParams {}
```

Add tool handler:

```rust
#[tool(
    name = "crate_graph",
    description = "Show the workspace crate dependency graph. Lists all crates and their inter-crate dependencies."
)]
async fn crate_graph(
    &self,
    Parameters(_params): Parameters<CrateGraphParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!("Tool call: crate_graph");
    let _guard = crate::status::StatusGuard::new("crate_graph");
    self.refresh()?;
    let db = self.lock_db()?;
    let result = tools::crate_graph::handle_crate_graph(&db)
        .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 4: Run tests**

Run: `cargo build`
Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: compiles clean

**Step 5: Commit**

```
feat: add crate_graph tool for workspace dependency visualization
```

---

### Task 9: Update server instructions and CLAUDE.md

**Files:**
- Modify: `src/server/mod.rs:273-292` (ServerInfo instructions)
- Modify: `src/main.rs` (illu_agent_section function for CLAUDE.md/GEMINI.md generation)

**Step 1: Update server instructions**

Update the `get_info()` instructions string to mention the new tools.

**Step 2: Update `illu_agent_section` in main.rs**

Add the new tools to the command table in the generated CLAUDE.md section.

**Step 3: Final verification**

Run: `cargo test`
Run: `cargo clippy --all-targets --all-features -- -D warnings`
Run: `cargo fmt --all -- --check`
Expected: all pass, no warnings

**Step 4: Commit**

```
docs: update server instructions and agent section for new tools
```
