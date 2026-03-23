# Enhanced Code Intelligence for illu-rs

Date: 2026-03-16

## Overview

Six enhancements to make illu-rs tools return enough information that AI assistants rarely need follow-up file reads. Grouped into two phases: Phase 1 enriches the data model, Phase 2 adds new query capabilities on top.

## Phase 1: Data Enrichment

### 1.1 Source Code in Context Results

**Problem:** `context` returns file:line and signature but not the actual code. The AI must Read the file separately every time.

**Solution:** Store the full source body of each symbol in a `body TEXT` column on `symbols`. Captured at parse time from `node.start_byte()..node.end_byte()`. Capped at 100 lines; longer bodies are truncated with `// ... truncated`. The `context` tool renders the body as a fenced Rust code block.

### 1.2 Doc Comments

**Problem:** `///` and `/** */` doc comments are not captured. They convey intent that signatures alone cannot.

**Solution:** New `doc_comment TEXT` column on `symbols`. The parser walks backward through preceding siblings of each declaration node, collecting `line_comment` nodes starting with `///` and `block_comment` nodes starting with `/**`. Non-comment, non-attribute siblings (`attribute_item` nodes like `#[derive(...)]`) are skipped during the walk; the walk stops at the first sibling that is neither a comment nor an attribute. Prefixes are stripped, lines joined with newlines. Added to `symbols_fts` so doc comments are searchable via the `query` tool. The `context` tool renders them as blockquotes. The `query` tool shows the first line as a snippet.

### 1.3 Struct Fields and Enum Variants

**Problem:** Knowing a type exists at a line number isn't enough. The AI needs field names, types, and variants to write correct code.

**Solution:** New `details TEXT` column on `symbols`. Contents vary by kind:
- **Structs:** `field_declaration_list` children parsed into `name: Type` per line.
- **Enums:** `enum_variant_list` children parsed into `VariantName`, `VariantName(Type)`, or `VariantName { field: Type }` per line.
- **Traits:** `declaration_list` children parsed into method signatures per line.
- **Functions/Use/Mod:** NULL.

The `context` tool renders details as a code block.

### 1.4 Trait-Type Relationships

**Problem:** "What traits does this type implement?" and "What types implement this trait?" are hard to answer from raw source.

**Solution:** New `trait_impls` table:

```sql
CREATE TABLE IF NOT EXISTS trait_impls (
    id INTEGER PRIMARY KEY,
    type_name TEXT NOT NULL,
    trait_name TEXT NOT NULL,
    file_id INTEGER NOT NULL REFERENCES files(id),
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    UNIQUE(type_name, trait_name, file_id)
);
```

The `UNIQUE` constraint on `(type_name, trait_name, file_id)` prevents duplicate rows on re-indexing. The table references `file_id` but not `crate_id` directly; workspace-level queries join through `files.crate_id` when needed.

The parser detects trait impls in `impl_item` nodes by looking for a `type_identifier` or `scoped_type_identifier` child (the trait) appearing before a `for` keyword, followed by another `type_identifier` or `generic_type` child (the implementing type). Inherent impls (no `for` keyword) are ignored. Returns a new `TraitImpl` struct alongside symbols.

Two new DB query methods:
- `get_trait_impls_for_type(type_name)` — "what traits does X implement?"
- `get_trait_impls_for_trait(trait_name)` — "what types implement Y?"

The `context` tool includes trait impl information when the symbol is a struct/enum (shows implemented traits) or a trait (shows implementing types).

## Phase 2: New Query Capabilities

### 2.1 Callees View in Context

**Problem:** `impact` shows "who depends on me?" but not "what do I depend on?" Understanding a function's behavior requires knowing what it calls.

**Solution:** New DB method `get_callees(symbol_name)` that walks the forward direction of `symbol_refs`: given a symbol as `source_symbol_id`, finds its `target_symbol_id` entries — i.e., the symbols it references. This is the natural direction of the data (`source` calls/uses `target`), whereas `impact` walks the reverse direction (finding all sources that reference a given target). Added as a "Callees" section in the `context` tool output, grouped by ref kind (calls vs type refs). No new tool — this is natural context for a symbol.

### 2.2 Overview Tool

**Problem:** No way to get a structural map of a crate or directory without globbing and reading many files.

**Solution:** New 5th MCP tool `overview` with params:

```rust
struct OverviewParams {
    path: String,
}
```

Calls `get_symbols_by_path_prefix(path)` which returns public and `pub(crate)` symbols under a file path prefix (filtered by `WHERE visibility IN ('public', 'pub(crate)')`), ordered by file then line. `pub(crate)` symbols are included because in a workspace context they are visible to sibling crates and relevant for understanding crate APIs. Output is grouped by file, then by kind within each file. For each symbol: name, kind, signature, first line of doc comment (if any). No source bodies — this is a map, not a deep dive.

## Schema Changes Summary

**`symbols` table — 3 new columns:**
- `doc_comment TEXT` — nullable
- `body TEXT` — nullable
- `details TEXT` — nullable

**New table:** `trait_impls` (id, type_name, trait_name, file_id, line_start, line_end) with `UNIQUE(type_name, trait_name, file_id)`

**`symbols_fts` migration:** The existing FTS table (`name, signature`) must be rebuilt to include the `doc_comment` column. Since `CREATE VIRTUAL TABLE IF NOT EXISTS` will not alter an existing table, the `migrate()` function must detect whether `doc_comment` is already in the FTS schema. If not: drop `symbols_fts`, recreate with `(name, signature, doc_comment)`, and repopulate from `symbols`. Use a `metadata` row (e.g., key `schema_version`) to track migration state and avoid re-running on every startup.

No changes to `symbol_refs`, `dependencies`, `docs`, `crates`, `crate_deps`, `files`, or `metadata` (beyond adding `schema_version`).

## Parser Changes Summary

**`Symbol` struct gains 3 fields:**
- `doc_comment: Option<String>`
- `body: Option<String>`
- `details: Option<String>`

**New `TraitImpl` struct:**
- `type_name: String`
- `trait_name: String`
- `file_path: String`
- `line_start: usize`
- `line_end: usize`

`file_path` on `TraitImpl` is for the parser's benefit (matching the existing `Symbol` pattern). When storing, the caller passes `file_id` separately; `file_path` is not used by `store_trait_impls`.

**`parse_rust_source` return type** changes from `Vec<Symbol>` to `(Vec<Symbol>, Vec<TraitImpl>)`.

**New extraction logic:**
- Doc comments: backward sibling walk for `///` and `/**` nodes, skipping `attribute_item` nodes, stopping at the first non-comment/non-attribute sibling.
- Body: full text span from `node.start_byte()..node.end_byte()`, 100-line cap.
- Details: struct fields, enum variants, trait method signatures.
- Trait impls: detect `impl Trait for Type` pattern in `impl_item` nodes via child node inspection.

## Store Changes Summary

- `store_symbols` updated to write `doc_comment`, `body`, `details`.
- New `store_trait_impls(db, file_id, &[TraitImpl])` function. Uses `file_id` parameter for the FK; ignores `TraitImpl.file_path`.
- `symbols_fts` insert includes `doc_comment`.

## Existing Struct Updates

- **`StoredSymbol`** (`db.rs`): Add `doc_comment: Option<String>`, `body: Option<String>`, `details: Option<String>` fields. Update `search_symbols` and all other queries that SELECT from `symbols` to include the new columns.

## Tool Changes Summary

| Tool | Change |
|------|--------|
| `context` | Renders doc comment (blockquote), details (code block), body (fenced Rust), callees (grouped list), trait impls (for structs/enums/traits) |
| `query` | Symbol results include first line of doc comment as snippet |
| `impact` | No changes |
| `docs` | No changes |
| `overview` | New tool — path prefix, public + pub(crate) symbols grouped by file and kind |

## Indexer Pipeline Changes

- **`index_crate_sources`** (`src/indexer/mod.rs`): receives `(Vec<Symbol>, Vec<TraitImpl>)` from `parse_rust_source`, calls `store_symbols` then `store_trait_impls`.
- **`refresh_index`** (`src/indexer/mod.rs`): The existing `db.delete_file_data(path)` method (`src/db.rs`) must be extended to also `DELETE FROM trait_impls WHERE file_id = ?` alongside its existing cleanup of `symbol_refs`, `symbols_fts`, `symbols`, and `files`.
- **`generate_claude_skill`** (`src/indexer/mod.rs`): Updated to document the `overview` tool and the enriched `context` output.

## Testing Strategy

**Parser tests:**
- Doc comment extraction (single `///`, multi-line, `/** */`, absent, with interleaved `#[derive]` attributes)
- Body extraction (normal, >100 lines truncation)
- Struct field extraction (named, tuple, unit)
- Enum variant extraction (unit, tuple, struct variants)
- Trait method signature extraction
- Trait impl detection (inherent vs trait impl, generics, scoped trait paths)

**DB tests:**
- New columns round-trip (insert with doc_comment/body/details, read back)
- `trait_impls` CRUD and both query directions
- `trait_impls` UNIQUE constraint (duplicate insert doesn't error with INSERT OR IGNORE)
- `get_symbols_by_path_prefix` with various prefixes
- `get_callees` returns correct forward references
- FTS searches doc comments
- Schema migration: FTS rebuild on upgrade from old schema

**Tool tests:**
- `context` output includes all new sections
- `query` shows doc comment snippets
- `overview` groups by file/kind, public + pub(crate) filtering, path prefix matching

**Integration tests:**
- Extend end-to-end test with richer fixture containing doc comments, struct fields, trait impls, cross-references. Verify full pipeline from indexing through tool output.
