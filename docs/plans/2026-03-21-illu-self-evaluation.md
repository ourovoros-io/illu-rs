# illu-rs Self-Evaluation: AI Coding Assistant Perspective

**Date:** 2026-03-21
**Method:** Used illu's MCP tools against its own codebase, evaluating correctness,
usefulness, and gaps from the perspective of an AI coding assistant's real workflow.

**Note:** Index was STALE during testing (16 files changed since last index).
Some source bodies reflected old code.

---

## Summary

illu is already highly useful for the core workflow: **find → understand → assess impact → navigate**.
The query, context, overview, boundary, and callpath tools are excellent.
Three bugs and a few design issues significantly reduce reliability.

### Verdict by Use Case

| Use Case | Rating | Primary Tool(s) |
|----------|--------|-----------------|
| Find a symbol | A | query |
| Understand what it does | A | context |
| Assess blast radius before edit | A- | impact, references |
| Navigate from error/line | A | symbols_at |
| Trace call chains | A | callpath |
| Understand module structure | A | overview, tree, boundary |
| Find tests that cover a symbol | C | test_impact (broken refs) |
| Explore call graph around a symbol | D | neighborhood (upstream broken) |
| Understand what changed in a diff | F | diff_impact (output too large) |
| Find dead code | C | unused (false positives from ref bug) |
| Find similar code | C+ | similar (scoring too coarse) |

---

## Bugs Found

### BUG 1: Neighborhood upstream traversal is broken (HIGH)

**Symptom:** `neighborhood direction="up"` returns "No connections found" for symbols
that have many known callers (e.g., `resolve_symbol` has 13 callers per context tool).

**Reproduced with:** `resolve_symbol`, `handle_query` — both show zero upstream.
`format_symbols` works (finds `handle_query` as caller).

**Root cause hypothesis:** `get_callers_by_name` uses a different matching strategy
than `get_callers` (ID-based). Name-based lookup likely fails when the ref was stored
with a different name format or when the symbol is in a different crate/module.

**Impact:** The neighborhood tool is the only way to get a bidirectional call graph
view. With upstream broken, it's effectively just `callees` — making it redundant
with the context tool's callees section.

### BUG 2: Ref extraction misses test-to-code calls (HIGH)

**Symptom:** `Database::open_in_memory` shows zero callers despite being called in
virtually every test function. Body search confirms `Database::open_in_memory().unwrap()`
appears throughout tests.

**Root cause hypothesis:** When extracting refs from test function bodies,
`Database::open_in_memory()` may not resolve because:
- The `Database::` qualifier needs impl_type resolution in test context
- Or `open_in_memory` isn't in the `known_symbols` set during test body scanning
- Or test module extraction skips/mishandles method calls on types

**Impact:** This poisons multiple tools:
- `unused` reports false positives (says symbols are unused when tests use them)
- `test_impact` misses test coverage
- `orphaned` gives false "no orphaned" results
- `impact` related tests section is incomplete

### BUG 3: diff_impact produces unbounded output (MEDIUM)

**Symptom:** `diff_impact git_ref="HEAD~3..HEAD"` produced 119,452 characters,
exceeding the MCP token limit and failing to deliver results.

**Root cause:** A 3-commit diff touching 16 files generates impact analysis for
every changed symbol, with no output cap. For any meaningful PR, this will blow up.

**Impact:** The most common use case — "what changed and what's affected?" — fails
for non-trivial diffs. The `changes_only=true` flag helps but loses the most
valuable part (downstream impact).

---

## Design Issues (Not Bugs)

### 1. Related (impl) section is unbounded in context output

`Database::open` context shows **70+ related methods** in the "Related (impl Database)"
section. For a struct with many methods, this dominates the output and wastes context
window. Most of the time I don't need to see all 70 sibling methods.

**Suggestion:** Cap related section at ~10 items with a "(N more)" note,
or make it opt-in via sections filter. Or: only show related symbols that share
callers/callees with the target (actually related, not just co-located).

### 2. Similar tool scoring is too coarse

`handle_impact` similar results: top match (score 6) is genuinely similar.
But scores 2-10 just share `Database` param type and `new` callee — that's true
of almost every function in the codebase. The scoring over-weights common types.

**Suggestion:** Penalize ubiquitous types (`Database`, `String`, `Option`).
Weight parameter position and arity. Consider name similarity (edit distance).

### 3. No output size control for verbose tools

Several tools can produce unbounded output: diff_impact, references (StoredSymbol
returned 30 call sites + 34 signature usages), overview. There's no global
"budget" or truncation mechanism.

**Suggestion:** Add a `max_chars` or `budget` parameter that truncates with
a summary note. MCP tool output should always fit in ~4K tokens.

### 4. Health tool noise sources could be more actionable

Health correctly identifies `new` (124 low-confidence refs) as a noise source.
But it doesn't tell me what to DO about it. Are these false refs? Should I ignore them?

**Suggestion:** Add a recommendation: "These are common names that cause
false-positive refs. Impact/callers for these symbols may be inflated."

---

## What Works Excellently

### 1. Query with filters (A)
Exact match, kind, signature, path, attribute filters all compose cleanly.
Results include full signatures and file:line. This replaces grep for 90% of
my symbol-finding needs.

### 2. Context with sections filter (A)
The sections parameter is crucial for efficiency. I can ask for just source,
just callers, or just callees. The `callers_path` filter is excellent for
excluding test callers when I only care about production code.

### 3. Boundary analysis (A+)
The single most useful tool for refactoring. "9 public API symbols vs 30 internal"
instantly tells me what I can safely change. Every codebase should have this.

### 4. Callpath (A)
`index_repo → index_single_crate → index_crate_sources → parse_rust_source`
with file:line for each hop. Clean, correct, instantly useful.

### 5. Symbols_at for error navigation (A)
`src/db.rs:150` → shows both the enclosing `impl Database` and the specific
`migrate` function. Perfect for compiler errors and stack traces.

### 6. Impact with depth control (A)
Depth-by-depth breakdown with call chain paths (`via search_symbols → resolve_symbol`).
The Related Tests section with `cargo test` command is exactly right.

### 7. References for types (A)
`StoredSymbol` references showing 30 call sites + 34 signature usages gives me
complete visibility into how a type flows through the codebase.

---

## Missing Features (Ranked by Impact on AI Workflow)

### 1. Semantic search / fuzzy concept matching (HIGH)
When I don't know the exact name, I need "find functions related to parsing imports"
or "find error handling patterns." Query FTS helps but isn't semantic.

### 2. Change suggestion preview (MEDIUM)
"If I rename `StoredSymbol` to `IndexedSymbol`, show me every file I need to edit."
`rename_plan` exists but I didn't test it — this is the right direction.

### 3. Dependency doc search scoped to usage (MEDIUM)
`docs` tool fetches crate docs, but what I really want is "how does THIS codebase
use rusqlite?" — show me the patterns, not the upstream docs.

### 4. Cross-file data flow (LOW)
"Trace how a `SymbolRef` struct flows from `extract_refs` through `store_symbol_refs`
to `get_callers`." This is deeper than callpath — it's data flow, not call flow.

---

## Recommendations (Priority Order)

1. **Fix ref extraction for test bodies** — This is the highest-leverage fix.
   It improves unused, test_impact, orphaned, and impact tools simultaneously.

2. **Fix neighborhood upstream** — Investigate `get_callers_by_name` vs `get_callers`
   discrepancy. Neighborhood should use ID-based traversal like context does.

3. **Add output budgeting** — Cap diff_impact and other verbose tools. A tool that
   blows up the token limit provides zero value.

4. **Cap related section in context** — 70+ siblings is noise. Show 10 + count.

5. **Improve similar scoring** — Penalize ubiquitous types, weight arity/position.

6. **Re-index after code changes** — The freshness tool correctly reports staleness,
   but there's no auto-refresh. Consider auto-refresh on MCP connection start.
