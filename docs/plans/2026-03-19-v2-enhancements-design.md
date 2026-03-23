# illu-rs v2 Enhancements Design

Date: 2026-03-19

Four improvements to make illu-rs more accurate and useful: qualified path resolution, change-set impact analysis, trait impl surfacing, and docs improvements.

## 1. Qualified Path Reference Resolution

**Problem**: `extract_refs` matches identifiers against a global `HashSet<String>` of all symbol names. If two crates both define `Config`, the ref resolves to whichever `get_symbol_id_by_name` returns first. This produces false positives in impact analysis.

**Approach**: Parse `use` statements to build an import map per file, then resolve references against it before falling back to name-based matching. Glob imports (`use foo::*`) are not expanded — they degrade to current behavior.

### Parser changes (`parser.rs`)

New function `extract_import_map(root_node, source) -> HashMap<String, ImportInfo>`:
- Walks `use_declaration` AST nodes
- Builds `short_name -> ImportInfo { qualified_path, source_file: Option<String> }` mapping
- Handles direct imports, aliases (`as`), nested groups (`use foo::{Bar, Baz}`)
- Skips glob imports — identifiers matching a glob path fall through to name-based matching

Examples:
- `use crate::config::Config` → `Config -> ImportInfo { path: "crate::config::Config", file: Some("src/config.rs") }`
- `use std::collections::HashMap` → `HashMap -> ImportInfo { path: "std::collections::HashMap", file: None }` (external)
- `use anyhow::Result as AnyResult` → `AnyResult -> ImportInfo { path: "anyhow::Result", file: None, alias: true }`

### `SymbolRef` changes

Add `target_file: Option<String>` field. When the import map resolves a reference to a known file in the index, this is populated.

### `collect_body_refs` changes

Resolution priority order:
1. **Import map** — look up identifier, if it maps to a file in the index, set `target_file`
2. **Same-file symbol** — check if identifier matches a symbol defined in the current file
3. **Same-crate symbol** — check symbols in files belonging to the same crate (new DB query: `get_symbol_id_in_crate`)
4. **Global name match** — current fallback via `get_symbol_id_by_name`

### `store_symbol_refs` changes

When `target_file` is `Some`, use `get_symbol_id(name, file)` (exact match). When `None`, fall back to `get_symbol_id_by_name(name)` (current behavior).

### No schema changes needed

The `symbol_refs` table stores `(source_symbol_id, target_symbol_id, kind)` — the resolution improvement happens at insert time, not query time.

---

## 2. Change-Set Aware Impact (`diff_impact` tool)

**Problem**: Impact analysis currently works one symbol at a time. When reviewing a set of changes (a PR, recent commits), I have to query each modified symbol individually and mentally merge the results.

**Approach**: New MCP tool `diff_impact` that parses `git diff` output, maps changed lines to symbols, and runs batch impact analysis.

### Tool interface

New tool `diff_impact` with params:
- `git_ref: Option<String>` — git ref range (`HEAD~3..HEAD`), branch name, or commit SHA. Omit for unstaged changes. Single ref like `main` becomes `main..HEAD`.

### Implementation flow

1. **Parse diff**: Run `git diff <ref>` in repo directory. Parse unified diff to extract `Vec<(file_path, Vec<(start_line, end_line)>)>`.
2. **Map lines to symbols**: For each changed file + line range, query `symbols` table: `SELECT name FROM symbols WHERE file_id = ? AND line_start <= ? AND line_end >= ?`.
3. **Batch impact**: Run existing `impact_dependents` CTE for each affected symbol. Deduplicate results across symbols.
4. **Format output**: Group by changed file, show which symbols were modified and their downstream dependents.

### Output format

```
## Changed Symbols

### src/db.rs
- **search_symbols** (line 450, modified)
- **insert_file** (line 320, modified)

### Downstream Impact

#### search_symbols
- handle_query (src/server/tools/query.rs) — depth 1
- handle_context (src/server/tools/context.rs) — depth 1

#### insert_file
- index_repo (src/indexer/mod.rs) — depth 1
```

### File layout

- `src/server/tools/diff_impact.rs` — tool handler, owns git diff parsing and line-to-symbol mapping
- DB layer unchanged — reuses `impact_dependents`

---

## 3. Trait Impl Mapping in Context Output

**Problem**: illu already parses and stores trait impls (`trait_impls` table, `get_trait_impls_for_type`, `get_trait_impls_for_trait`), but this data isn't exposed through any MCP tool.

**Approach**: Embed trait impl information in the `context` tool output based on symbol kind.

### Changes to `handle_context` (`src/server/tools/context.rs`)

After the existing source body and callees sections, add:

- **If symbol is a struct or enum**: Call `get_trait_impls_for_type(symbol_name)`, render as:
  ```
  ### Trait Implementations
  - Display (src/model.rs:45-52)
  - Serialize (src/model.rs:54-60)
  - From<RawConfig> (src/model.rs:62-78)
  ```

- **If symbol is a trait**: Call `get_trait_impls_for_trait(symbol_name)`, render as:
  ```
  ### Implementors
  - Config (src/config.rs:30-45)
  - Settings (src/settings.rs:12-28)
  ```

### No other changes needed

Parser (`extract_trait_impl_info`) and DB (`insert_trait_impl`, queries) already work. This is purely a wiring change in `context.rs`.

---

## 4. Docs Improvements

### 4a: Preserve docs across re-indexes

**Problem**: `clear_index` deletes `docs`, `dependencies`, and all code tables. Every re-index throws away cached docs and re-fetches them, even when dependency versions haven't changed.

**Fix**: Split `clear_index` into two operations:
- `clear_code_index()` — deletes FTS tables, `symbol_refs`, `trait_impls`, `symbols`, `files`, `crate_deps`, `crates`, `metadata`. Called during normal re-indexing.
- `clear_all()` — calls `clear_code_index()` plus deletes `docs`, `docs_fts`, `dependencies`. Only for full reset.

Re-indexing calls `clear_code_index()`. Dependency docs persist.

**Version change handling**: After storing new dependencies, delete `docs` rows whose `dependency_id` points to a dependency whose version changed compared to what was previously stored. This handles `cargo update` scenarios.

### 4b: Two-tier doc storage (summary + per-module detail)

**Problem**: All docs truncated to 8000 chars. Large crates like `tokio` lose most of their API surface.

**Schema change**: Add `module TEXT DEFAULT ''` column to `docs` table. Empty string = crate-level summary. Non-empty = module path (e.g., `sync`, `fs`).

**Indexing change** (`cargo_doc.rs`): `parse_rustdoc_json` produces multiple rows:
- Summary row (module = `""`, ~2k chars): module listing with top-level items and one-line descriptions.
- Per-module detail rows (module = `"sync"`, `"fs"`, etc.): full item listings for each module.

**`handle_docs` change**:
- `topic` empty → return summary row (module index).
- `topic` provided → match against `module` column, return that module's detail. Fall back to FTS search if no exact match.

**Network fallback docs** (README, docs.rs HTML) remain single rows with `module = ""` since they can't be meaningfully split.

### Keep raw JSON parsing

No switch to `rustdoc-types` crate. Current ~150 lines of `serde_json::Value` parsing is small, contained, and has a graceful network fallback if the format changes. Adding `rustdoc-types` would create a version-coupling problem (which nightly matches which crate version).

---

## Implementation Order

1. **Trait impl in context** — smallest change, immediate value, no migrations
2. **Docs: preserve across re-indexes** — `clear_index` split, no new features
3. **Docs: two-tier storage** — schema migration, `cargo_doc.rs` refactor
4. **Qualified path resolution** — parser work, import map, resolution priority chain
5. **`diff_impact` tool** — new tool, git diff parsing, batch impact

Items 1-2 are independent and can be parallelized. Items 3-5 are independent of each other but each builds on 1-2 being stable.
