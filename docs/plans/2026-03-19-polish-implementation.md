# Polish: Error Messages, Performance, Syntax Coverage

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Polish illu-rs with better error messages, faster indexing, and broader Rust syntax support.

**Architecture:** Three independent workstreams: (1) improve error/empty-result messages across all tool handlers, (2) eliminate double-parsing and N+1 queries in the indexing pipeline, (3) add union and extern block support to the parser.

**Tech Stack:** Rust, tree-sitter, rusqlite

---

### Task 1: Better error messages across all tool handlers

Improve every "not found" / "empty result" message to be actionable. The consumer is an AI (Claude), so messages should explain why and suggest what to try next.

**Files:**
- Modify: `src/server/tools/context.rs`
- Modify: `src/server/tools/impact.rs`
- Modify: `src/server/tools/query.rs`
- Modify: `src/server/tools/docs.rs`
- Modify: `src/server/tools/diff_impact.rs`
- Modify: `src/server/tools/overview.rs`
- Modify: `src/server/tools/tree.rs`

**Changes:**

**context.rs** — symbol not found (~line 13):
```rust
// Before:
return Ok(format!("No symbol found matching '{symbol_name}'."));
// After:
return Ok(format!(
    "No symbol found matching '{symbol_name}'.\n\
    Try a partial name or use `query` to search."
));
```

**impact.rs** — symbol not found (~line 10):
```rust
// Before:
return Ok(format!("No symbol found matching '{symbol_name}'."));
// After:
return Ok(format!(
    "No symbol found matching '{symbol_name}'.\n\
    Verify the symbol exists with `query`."
));
```

**impact.rs** — no dependents (~line 52-57):
```rust
// Before:
output.push_str("No dependents found.\n");
output.push_str("Note: Symbol references are populated during indexing.\n");
// After:
output.push_str("No dependents found.\n");
output.push_str(
    "This symbol may be a leaf (not used by other code), \
     or only used in ways the indexer cannot detect \
     (e.g., macro-generated calls, dynamic dispatch).\n",
);
```

**query.rs** — no results (~line 27):
```rust
// Before:
output.push_str("No results found.");
// After:
output.push_str(&format!("No results found for '{query}'."));
```

Note: The `query` variable isn't in scope at line 27. You'll need to pass it or capture it. Check the actual function signature — `handle_query` takes `query: &str`. Thread it to where the empty check happens.

**query.rs** — unknown scope (~line 22):
```rust
// Before:
other => return Err(format!("Unknown scope: {other}").into()),
// After:
other => return Err(
    format!("Unknown scope: '{other}'. Valid: symbols, docs, files, all").into()
),
```

**docs.rs** — known dep but no docs (in the no-topic path):
```rust
// Before:
"'{dep_name}' is a known dependency but no docs were fetched. It may not be published on docs.rs."
// After:
"'{dep_name}' is a known dependency but no docs were fetched. \
 The crate may not be on docs.rs, or doc fetching may have been skipped."
```

**docs.rs** — topic not found, no modules available (in handle_docs_with_topic):
After the existing modules listing, if modules is empty, add:
```rust
if modules.is_empty() {
    msg.push_str("\n\nNo module-level docs available for this dependency.");
}
```

**diff_impact.rs** — no changes (~line 96):
```rust
// Before:
return Ok("No changes detected in the diff.".to_string());
// After:
return Ok(
    "No changes detected. Check the git ref \
     (e.g., git_ref: \"HEAD~1..HEAD\" for last commit, \
     or omit for unstaged changes)."
    .to_string()
);
```

**diff_impact.rs** — no symbols at changed lines (~line 126):
```rust
// Before:
"No indexed symbols found at the changed lines."
// After:
"No indexed symbols overlap the changed lines. \
 Changes may be in comments, whitespace, or between function definitions."
```

**overview.rs** — no symbols (~line 8):
```rust
// Before:
return Ok(format!("No public symbols found under '{path}'."));
// After:
return Ok(format!(
    "No public symbols found under '{path}'. \
     Try a broader prefix like 'src/'."
));
```

**tree.rs** — no files (~line 8):
```rust
// Before:
return Ok(format!("No files found under '{path}'."));
// After:
return Ok(format!(
    "No files found under '{path}'. \
     Try 'src/' for standard Rust layout."
));
```

**After all changes:**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings. The existing tests check for substrings like "No symbol found" — verify those still match after the message changes. Update test assertions if needed.

Commit: `polish: improve error messages across all tool handlers`

---

### Task 2: Performance — eliminate double parsing and N+1 lookups

Two optimizations: (A) build an in-memory symbol ID lookup table to replace per-ref SQL queries, (B) merge symbol extraction and ref extraction into a single parse pass.

**Files:**
- Modify: `src/db.rs` (add `build_symbol_id_map`)
- Modify: `src/indexer/mod.rs` (merge phases, use lookup map)
- Modify: `src/indexer/parser.rs` (add combined parse function)
- Modify: `src/indexer/store.rs` (accept lookup map)

#### Part A: In-memory symbol ID lookup table

**Problem:** `store_symbol_refs` does 2-7 individual SQL queries per ref to resolve symbol IDs. For 5000 refs that's 10,000-35,000 queries.

**Fix:** After storing all symbols, build a lookup table in memory. Use it instead of per-ref queries.

Add to `src/db.rs`:

```rust
pub struct SymbolIdMap {
    by_name_and_file: std::collections::HashMap<(String, String), SymbolId>,
    by_name_and_impl: std::collections::HashMap<(String, String), SymbolId>,
    by_name: std::collections::HashMap<String, SymbolId>,
}

impl SymbolIdMap {
    pub fn resolve(
        &self,
        name: &str,
        target_file: Option<&str>,
        target_context: Option<&str>,
    ) -> Option<SymbolId> {
        // Try impl-qualified first
        if let Some(ctx) = target_context {
            if let Some(id) = self.by_name_and_impl.get(&(name.to_string(), ctx.to_string())) {
                return Some(*id);
            }
        }
        // Try file-qualified
        if let Some(file) = target_file {
            if let Some(id) = self.by_name_and_file.get(&(name.to_string(), file.to_string())) {
                return Some(*id);
            }
        }
        // Fall back to name-only
        self.by_name.get(name).copied()
    }
}

pub fn build_symbol_id_map(&self) -> SqlResult<SymbolIdMap> {
    let mut by_name_and_file = std::collections::HashMap::new();
    let mut by_name_and_impl = std::collections::HashMap::new();
    let mut by_name = std::collections::HashMap::new();

    let mut stmt = self.conn.prepare(
        "SELECT s.id, s.name, f.path, s.impl_type \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: SymbolId = row.get(0)?;
        let name: String = row.get(1)?;
        let path: String = row.get(2)?;
        let impl_type: Option<String> = row.get(3)?;

        by_name_and_file
            .entry((name.clone(), path))
            .or_insert(id);
        if let Some(it) = impl_type {
            by_name_and_impl
                .entry((name.clone(), it))
                .or_insert(id);
        }
        by_name.entry(name).or_insert(id);
    }

    Ok(SymbolIdMap {
        by_name_and_file,
        by_name_and_impl,
        by_name,
    })
}
```

Add a new `store_symbol_refs_fast` that uses the map:

```rust
pub fn store_symbol_refs_fast(
    &self,
    refs: &[crate::indexer::parser::SymbolRef],
    map: &SymbolIdMap,
) -> SqlResult<u64> {
    let mut count = 0;
    for r in refs {
        let source_id = map.resolve(&r.source_name, Some(&r.source_file), None);
        let target_id = map.resolve(
            &r.target_name,
            r.target_file.as_deref(),
            r.target_context.as_deref(),
        );
        if let (Some(sid), Some(tid)) = (source_id, target_id) {
            self.insert_symbol_ref(sid, tid, &r.kind.to_string())?;
            count += 1;
        }
    }
    Ok(count)
}
```

Update `extract_all_symbol_refs` and `rebuild_refs_for_files` in `src/indexer/mod.rs` to:
1. Build the symbol map once: `let symbol_map = db.build_symbol_id_map()?;`
2. Use `db.store_symbol_refs_fast(&refs, &symbol_map)?` instead of `db.store_symbol_refs(&refs)?`

#### Part B: Single-pass parsing (merge symbol extraction and ref extraction)

**Problem:** Each file is parsed twice — once for symbols, once for refs.

**Fix:** Add a combined function that parses once and extracts both:

In `src/indexer/parser.rs`, add:

```rust
pub struct ParseResult {
    pub symbols: Vec<Symbol>,
    pub trait_impls: Vec<TraitImpl>,
    pub refs: Vec<SymbolRef>,
}

/// Parse source once, extract symbols, trait impls, and refs in a single pass.
pub fn parse_and_extract_refs<S: std::hash::BuildHasher, S2: std::hash::BuildHasher>(
    source: &str,
    file_path: &str,
    known_symbols: &std::collections::HashSet<String, S>,
    crate_map: &std::collections::HashMap<String, String, S2>,
) -> Result<ParseResult, String> {
    let tree = parse_source(source)?;
    let root = tree.root_node();

    let mut symbols = Vec::new();
    let mut trait_impls = Vec::new();
    extract_symbols(&root, source, file_path, &mut symbols, &mut trait_impls, None);

    let import_map = extract_import_map(&root, source);
    let mut refs = Vec::new();
    collect_refs(&root, source, file_path, known_symbols, &import_map, crate_map, None, &mut refs);

    Ok(ParseResult {
        symbols,
        trait_impls,
        refs,
    })
}
```

Then update `extract_all_symbol_refs` in `src/indexer/mod.rs` to use the combined function. This is the bigger refactor — instead of two separate loops (one for symbols, one for refs), do a single loop. However, there's a chicken-and-egg problem: refs need `known_symbols`, which comes from the symbols already stored.

**The practical fix:** Keep the two-phase approach but eliminate the second file read + parse. Cache the parsed trees or source content from phase 1 and reuse in phase 2. The simplest approach:

In `extract_all_symbol_refs`, the source is already being read from disk. The parse happens inside `extract_refs`. Since we already read the source, we're only saving the tree-sitter parse (not the disk read). The parse is ~50-200μs per file for typical Rust files, so for 1000 files that's 50-200ms saved.

**Decision:** Implement Part A (lookup table) which has the bigger impact. For Part B, just make `extract_refs` reuse the parsed tree if possible — or skip it for now since the double-parse overhead is much smaller than the N+1 query overhead.

**After all changes:**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings.

Commit: `perf: replace N+1 symbol lookups with in-memory hash map`

---

### Task 3: Broader Rust syntax coverage — union and extern blocks

Add support for `union` types and `extern` blocks in the parser.

**Files:**
- Modify: `src/indexer/parser.rs` (add match arms in `extract_symbols`)
- Test: `src/indexer/parser.rs` test module

#### Part A: Union support

In `src/indexer/parser.rs`, `extract_symbols` (~line 196), add a match arm for `union_item`:

```rust
"union_item" => {
    if let Some(sym) = extract_named_item(&child, source, file_path, SymbolKind::Struct) {
        // Reuse Struct kind — unions are structurally similar
        symbols.push(sym);
    }
}
```

Wait — `SymbolKind` doesn't have a `Union` variant. Two options:
1. Add `Union` to `SymbolKind` enum (cleanest)
2. Reuse `Struct` (simpler, but loses info)

**Go with option 1:** Add `Union` to the enum.

In `src/indexer/parser.rs`:
- Add `Union` variant to `SymbolKind` enum
- Add `"union" => Ok(Self::Union)` to `FromStr` impl
- Add `Self::Union => "union"` to `Display` impl

In `extract_symbols`:
```rust
"union_item" => {
    if let Some(sym) = extract_named_item(
        &child, source, file_path, SymbolKind::Union,
    ) {
        symbols.push(sym);
    }
}
```

Add test:
```rust
#[test]
fn test_extract_union() {
    let source = r#"
pub union MyUnion {
    pub i: i32,
    pub f: f32,
}
"#;
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let u = symbols.iter().find(|s| s.name == "MyUnion").unwrap();
    assert_eq!(u.kind, SymbolKind::Union);
    assert!(u.details.as_ref().is_some_and(|d| d.contains("i: i32")));
}
```

#### Part B: Extern block support

`extern "C" { fn c_func(); }` — tree-sitter parses this as `foreign_mod_item` containing `foreign_mod_body` with `function_signature_item` children.

In `extract_symbols`, add:
```rust
"foreign_mod_item" => {
    // extern "C" { ... } blocks — extract function signatures inside
    extract_symbols(&child, source, file_path, symbols, trait_impls, impl_type_name);
}
```

This works because `extract_symbols` already handles `declaration_list` recursion and `function_signature_item` is already matched via the trait item path. But we need to verify — the `foreign_mod_item` contains a child of kind `declaration_list` or `foreign_mod_body`.

Actually, check the tree-sitter Rust grammar. The structure is:
```
(foreign_mod_item
  (extern_modifier "C")
  (declaration_list
    (function_signature_item ...)))
```

Since `extract_symbols` already recurses into `declaration_list`, just adding `"foreign_mod_item"` to recurse should work:

```rust
"foreign_mod_item" => {
    extract_symbols(&child, source, file_path, symbols, trait_impls, impl_type_name);
}
```

Add test:
```rust
#[test]
fn test_extract_extern_block_functions() {
    let source = r#"
extern "C" {
    pub fn c_function(x: i32) -> i32;
    pub fn another_c_fn();
}
"#;
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let c_fn = symbols.iter().find(|s| s.name == "c_function");
    assert!(c_fn.is_some(), "extern C functions should be extracted: {symbols:?}");
}
```

Note: `function_signature_item` may not be in `extract_symbols`'s match. If not, it's handled via the `declaration_list` → `trait_item` path, which may not work for extern blocks. Debug by printing the AST with `tree.root_node().to_sexp()` if the test fails.

If `function_signature_item` isn't matched, add it:
```rust
"function_signature_item" => {
    if let Some(sym) = extract_function(&child, source, file_path) {
        let mut s = sym;
        s.impl_type = impl_type_name.map(String::from);
        symbols.push(s);
    }
}
```

#### Part C: Add const fn test

`const fn` should already work but isn't tested. Add:
```rust
#[test]
fn test_extract_const_fn() {
    let source = "pub const fn compute(x: u32) -> u32 { x + 1 }\n";
    let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
    let f = symbols.iter().find(|s| s.name == "compute").unwrap();
    assert_eq!(f.kind, SymbolKind::Function);
    assert!(f.signature.contains("const fn"));
}
```

**After all changes:**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass, no warnings.

Commit: `feat: add union and extern block support, test const fn`
