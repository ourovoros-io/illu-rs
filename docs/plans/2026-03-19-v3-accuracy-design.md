# illu-rs v3 Accuracy Improvements Design

Date: 2026-03-19

Three improvements to make the symbol reference graph more complete and accurate: method-level refs, cross-crate resolution, and stale ref cleanup.

## 1. Method-level refs via impl-block awareness

**Problem**: `self.method()` inside an `impl MyStruct` block matches `method` against all symbols globally. If another type also has a `method`, the ref resolves to the wrong one.

**Approach**: Track which impl type a method belongs to, and which impl type a `self.method()` call originates from, then match them.

### Parser changes (`parser.rs`)

**`Symbol` struct**: Add `pub impl_type: Option<String>`. When `extract_symbols` recurses into an `impl_item`, pass the type name down so functions inside get `impl_type: Some("MyStruct")`. Top-level functions get `None`.

**`SymbolRef` struct**: Add `pub target_context: Option<String>`. Set when a `self.method()` call is detected inside an impl block.

**`collect_refs`**: When entering an `impl_item`, extract the type name via existing `extract_impl_type`. Pass it as `impl_type: Option<&str>` to `collect_body_refs`.

**`collect_body_refs`**: Detect `field_expression` nodes where the receiver is `self`:
- tree-sitter structure: `call_expression` → `field_expression` → `self` + `field_identifier`
- When receiver is `self` and `impl_type` is known, set `target_context: Some(impl_type)` on the `SymbolRef`
- For non-self receivers, leave `target_context: None` (current behavior)

### DB changes (`db.rs`)

**Schema**: Add `impl_type TEXT` column to `symbols` table (nullable). Migration for existing DBs.

**`store_symbols`**: Write `impl_type` when storing symbols.

**New query `get_symbol_id_in_impl(name, impl_type) -> Option<SymbolId>`**: `WHERE s.name = ?1 AND s.impl_type = ?2`.

**`store_symbol_refs`**: When `target_context` is `Some(type_name)`, try `get_symbol_id_in_impl(method, type_name)` first, fall back to `get_symbol_id_by_name`.

---

## 2. Cross-crate qualified resolution in workspaces

**Problem**: `qualified_path_to_file` only handles `crate::` prefix. `use shared::Config` in a workspace doesn't resolve to the `shared` crate's files.

**Approach**: Build a crate-name-to-path map from the `crates` table, pass it to the parser for resolution.

### Crate map

`HashMap<String, String>` mapping crate name → relative path. Example: `"shared" → "shared"`. Built once per indexing run from `crates` table.

### Parser changes (`parser.rs`)

**`extract_refs`**: Add `crate_map: &HashMap<String, String>` parameter. Flows through to `collect_refs` → `collect_body_refs`.

**`qualified_path_to_file`**: Accept the crate map. Resolution:
- `crate::foo::Bar` → `src/foo.rs` (current)
- `shared::foo::Bar` → look up `shared` in crate map → `shared/src/foo.rs`
- `serde::Serialize` → not in crate map → `None` (external, skip)

### Indexer changes (`mod.rs`)

**`extract_all_symbol_refs`**: Before the file loop, query `db.get_all_crates()` and build the map. Pass to `extract_refs`.

**`refresh_index`**: Same — build crate map before rebuilding refs for dirty files.

### DB changes (`db.rs`)

Add `get_all_crates() -> Vec<StoredCrate>` if not already present.

No schema changes needed.

---

## 3. Incremental ref cleanup (delete stale refs)

**Problem**: When `refresh_index` re-indexes a file, old symbol IDs are deleted and new ones created. `symbol_refs` rows from other files that pointed to the old IDs become dangling.

**Approach**: After re-indexing dirty files, delete `symbol_refs` rows where source or target symbol no longer exists.

### DB changes (`db.rs`)

New method `delete_stale_refs() -> SqlResult<u64>`:
```sql
DELETE FROM symbol_refs
WHERE source_symbol_id NOT IN (SELECT id FROM symbols)
   OR target_symbol_id NOT IN (SELECT id FROM symbols)
```

### Indexer changes (`mod.rs`)

In `refresh_index`, after re-indexing dirty files and rebuilding their refs, call `db.delete_stale_refs()`.

No schema changes needed.

---

## Implementation Order

1. **Stale ref cleanup** — simplest, one query + one call site, immediate correctness benefit
2. **Cross-crate resolution** — adds crate map parameter threading, no schema change
3. **Method-level refs** — schema change + parser work, most complex

Items 1 and 2 are independent. Item 3 is independent of both.
