# Jon-Style Rust — Phase 2 (Exemplars Infrastructure) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 2 — exemplars infrastructure: a curated, compile-checked corpus of integrated Rust patterns surfaced via a new MCP tool.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md) for overall architecture; [Slice 1](2026-04-27-jon-style-rust-phase-1-ownership-design.md), [Slice 2](2026-04-27-jon-style-rust-phase-1-types-design.md), [Slice 3](2026-04-27-jon-style-rust-phase-1-performance-design.md), [Slice 4](2026-04-27-jon-style-rust-phase-1-unsafe-ffi-design.md) for the prior content-only slices.

## Motivation

Phase 0 + Phase 1 (4 slices) added 46 axioms. Each axiom is a single rule with a small anti/good pair — enough to communicate a discipline but not enough to show what *integrated* idiomatic Rust looks like for a non-trivial problem. Phase 2 adds a parallel corpus of "exemplars": small, focused, real Rust files that demonstrate multiple axioms working together.

Source material: handwritten patterns drawn from *Rust for Rustaceans*, the standard library, the std-supporting crate ecosystem (e.g., `thiserror`, `tokio`, `crossbeam-utils`), and the Crust-of-Rust series. Each exemplar cites both the source it's drawn from and the axioms it demonstrates.

**Why exemplars complement axioms:** an axiom answers "should I do X or Y?"; an exemplar answers "what does the integrated solution look like when the axioms apply together?". A user asking the agent "design me a layered API error type" gets axioms 57–66 from the existing tool — but doesn't get a full type with `From` impls, internal cause chain, and source-line capture. The exemplars tool fills that gap.

## Goal

1. Add a new MCP tool `mcp__illu__exemplars` that scores user queries against a corpus of compile-checked Rust files and returns the most relevant ones with metadata, code body, and cross-references to the axioms each demonstrates.
2. Land 9 exemplars in a 3-3-3 batch structure spanning the five Phase-0/1 areas (errors, ownership, types, performance, unsafe/FFI).
3. Ensure exemplars compile clean (clippy + fmt) so they cannot rot silently as the rest of the workspace evolves.

## Scope

**In scope:**
- New asset directory `assets/rust_exemplars/<category>/<slug>.rs` plus `assets/rust_exemplars/manifest.json`.
- New module `src/server/tools/exemplars.rs` mirroring the shape of `axioms.rs`.
- New MCP tool `mcp__illu__exemplars` registered in `src/server/mod.rs`.
- New tests: per-batch focused-query coverage tests (3) + 1 demo test + a compile-check test that includes every exemplar.
- Cross-references: each exemplar's manifest entry declares which axioms it demonstrates (forward link only; the axiom JSON is not modified).
- 9 exemplars across 3 batches (3-3-3) covering errors, ownership, types, performance, unsafe/FFI — drawn from the existing 102-axiom corpus.

**Explicit non-goals:**
- No modification to `assets/rust_quality_axioms.json` or the existing `axioms` tool.
- No automatic surfacing of exemplars from `rust_preflight` (deferred to a follow-up if useful — keeps Phase 2 small).
- No exemplar runtime tests (compile-clean only; "does it run correctly" is out of scope until the exemplar's behavior matters).
- No project-context overrides (Phase 3).

## Architecture

```
illu-rs
├── assets/
│   ├── rust_quality_axioms.json          (existing, unchanged)
│   └── rust_exemplars/                    (new)
│       ├── manifest.json
│       ├── errors/
│       │   └── api_error.rs
│       ├── ownership/
│       │   ├── cow_string.rs
│       │   └── drop_guard.rs
│       ├── types/
│       │   ├── sealed_trait.rs
│       │   ├── typestate_builder.rs
│       │   └── extension_trait.rs
│       ├── perf/
│       │   └── closed_dispatch.rs
│       └── unsafe_ffi/
│           ├── maybe_uninit_init.rs
│           └── c_string_wrapper.rs
└── src/server/tools/
    ├── axioms.rs                          (existing)
    └── exemplars.rs                       (new)
```

**Manifest schema** (`assets/rust_exemplars/manifest.json`):

```json
{
  "exemplars": [
    {
      "slug": "errors/api_error",
      "category": "Error Design",
      "title": "Layered API error hierarchy",
      "description": "Domain error enum with thiserror, From impls at module boundaries, source chain via #[from], and stable error codes for the public surface.",
      "triggers": ["error type design", "api error hierarchy", "thiserror enum", "from impl boundary"],
      "axioms_demonstrated": ["rust_quality_57_error_strategy", "rust_quality_58_error_design", "rust_quality_64_error_chain"],
      "source": "thiserror documentation; Rust for Rustaceans, ch. 4 §Error Handling"
    }
  ]
}
```

The `code` body is **not** stored in JSON — it's read from the corresponding `.rs` file at compile time via `include_str!` (mirrors how `axioms.rs` uses `include_str!` for the JSON itself). This means:

- The exemplar files are real Rust, IDE-friendly, and refactor-able.
- `cargo clippy --all-targets` includes them (via the compile-check test below) so they cannot rot.
- The manifest carries metadata only; the code is one source of truth.

**Server tool** (`src/server/tools/exemplars.rs`): mirrors `axioms.rs`:

- Parse-once cache (`OnceLock<Vec<Exemplar>>`).
- `handle_exemplars(query: &str) -> Result<String>` with scoring: triggers (+30 exact, +10 partial), category (+20 / +5), description (+2). Same shape as `handle_axioms`.
- Result formatting: title, category, description, axioms_demonstrated (as bullet list), source citation, then the code body in a Rust fenced block.
- `MAX_EXEMPLAR_RESULTS = 4` (smaller than axioms because each result is larger; tunable later).

**Tool registration** in `src/server/mod.rs`: register `exemplars` alongside the existing `axioms` tool, with a single `query: String` parameter.

**Compile-check test** at `src/server/tools/exemplars.rs` (or a sibling integration test):

```rust
#[cfg(test)]
mod exemplar_compile_tests {
    // Each exemplar is included as its own module so unrelated identifiers
    // don't collide. dead_code and unused_imports are allowed because
    // exemplars are demonstrations, not callable from the rest of the crate.
    #![allow(dead_code, unused_imports, unused_variables, unused_mut)]

    mod errors_api_error      { include!("../../../assets/rust_exemplars/errors/api_error.rs"); }
    mod ownership_cow_string  { include!("../../../assets/rust_exemplars/ownership/cow_string.rs"); }
    // ...one per exemplar
}
```

Clippy still runs on these via `cargo clippy --all-targets`; warnings are denied per workspace policy. **An exemplar that does not pass clippy is not an exemplar.** This is a load-bearing invariant.

## Schema (Rust types)

```rust
#[derive(Debug, Deserialize)]
struct ExemplarManifest {
    exemplars: Vec<ExemplarEntry>,
}

#[derive(Debug, Deserialize)]
struct ExemplarEntry {
    slug: String,                       // e.g. "errors/api_error"
    category: String,
    title: String,
    description: String,
    triggers: Vec<String>,
    axioms_demonstrated: Vec<String>,   // axiom IDs e.g. "rust_quality_57_error_strategy"
    source: String,
}

struct Exemplar {
    entry: ExemplarEntry,
    code: &'static str,                 // from include_str! at build time
}
```

The static-code coupling between `slug` and `include_str!` path requires a build-time check that every entry in the manifest has a corresponding file. That check lives in a unit test:

```rust
#[test]
fn every_manifest_entry_has_a_file() { /* assert each slug.rs exists */ }
```

## Candidate Exemplar List

**Batch 1 — Error and ownership patterns** (foundational integrated patterns):

1. **`errors/api_error.rs`** — Layered API error hierarchy with `thiserror`, `#[from]` for source chain, internal vs public error split. Demonstrates axioms 57 (error strategy), 58 (error design), 64 (error chain).
2. **`ownership/cow_string.rs`** — `Cow<'a, str>` for sometimes-borrowed-sometimes-owned config values, with a small parser that returns `Cow` to avoid eager allocation. Demonstrates axiom 86 (string allocation strategy), and the existing `Cow` axiom from the original 56-axiom corpus.
3. **`ownership/drop_guard.rs`** — RAII drop guard pattern (a `Guard<F>` that runs `F` on drop) for cleanup. Demonstrates axiom 71 (drop order matters), and the existing RAII axiom.

**Batch 2 — Type system patterns** (trait surface and type-level state):

4. **`types/sealed_trait.rs`** — Sealed trait pattern using a private `Sealed` supertrait, with the public trait re-exported. Demonstrates axiom 76 (sealed traits), 77 (object safety considerations).
5. **`types/typestate_builder.rs`** — Builder with type-state markers for required-field tracking; misuse is a compile error. Demonstrates axiom 83 (ZST markers), 80 (associated types vs generics).
6. **`types/extension_trait.rs`** — Extension trait that adds methods to a foreign type with a sealed marker so external impls are forbidden. Demonstrates axiom 76 (sealed traits), 77 (object safety), and the existing extension-trait axiom.

**Batch 3 — Performance + unsafe + integrated patterns**:

7. **`perf/closed_dispatch.rs`** — A small command-handler that dispatches via an enum (closed-set) rather than `Box<dyn Trait>`, with a `match` that compiles to a jump table. Demonstrates axiom 90 (enum dispatch), 88 (monomorphization cost).
8. **`unsafe_ffi/maybe_uninit_init.rs`** — Sound delayed initialization of a non-`Default` struct via `MaybeUninit` + `&raw mut`, with a full `# Safety` rustdoc and per-block `// SAFETY:` comments. Demonstrates axioms 97 (MaybeUninit), 94 (block discipline), 95 (fn contract), 96 (block scope).
9. **`unsafe_ffi/c_string_wrapper.rs`** — A pair of `extern "C"` functions: one that borrows a C string (returns its length), one that builds a Rust-owned string and transfers ownership to C with a matching `free_*` function. Demonstrates axioms 100 (FFI boundary), 101 (FFI layout), 102 (FFI strings), 94 (SAFETY comments).

## Drafting and Review Loop

Same shape as Phase 0/1 slices. Per exemplar: assistant drafts file + manifest entry; batch of 3; user reviews; commit per accepted batch.

**Pre-flight verification standard for exemplars:** every exemplar must compile clean (clippy + fmt), every axiom referenced in `axioms_demonstrated` must exist, and the `slug` must map to a real file. The per-batch reviewer must:
- Verify the code is genuinely idiomatic (not a strawman that hits the axiom literally without showing real Rust).
- Verify each `axioms_demonstrated` entry is actually demonstrated by the code (not just adjacent in topic).
- Verify the example would be useful in real production code, not pedagogical fluff.

## Verification and Exit Criteria

Phase 2 is complete when:

- 9 new exemplars drafted, reviewed, merged.
- New module `src/server/tools/exemplars.rs` with `handle_exemplars` and parse-once cache.
- New MCP tool `exemplars` registered in `src/server/mod.rs`.
- New tests:
  - `test_exemplar_manifest_parses` (data-validity).
  - `test_every_exemplar_slug_has_a_file` (cross-reference).
  - `test_every_axiom_reference_resolves` (cross-reference).
  - 3 per-batch focused-query coverage tests (`test_exemplars_batch_{1,2,3}_present`).
  - 1 demo-query test (`test_exemplar_demo_query_returns_new_exemplars`).
  - 1 compile-check module that `include!`s each exemplar.
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.
- The live MCP server (after rebuild + restart) surfaces exemplars for representative queries.

## Risks and Mitigations

- **Exemplar code rot.** As the workspace and the Rust toolchain evolve, exemplars could break silently. Mitigation: compile-check test runs on every CI pass.
- **Exemplar–axiom drift.** An axiom rewrite could leave an exemplar's `axioms_demonstrated` list stale. Mitigation: cross-reference test asserts every axiom ID resolves to a real axiom; reviewer flags exemplars that no longer demonstrate the listed axioms.
- **Exemplar surface too large for tool response.** Each exemplar is 30–100 lines; 4 results × 80 lines ≈ 320 lines of code in the response. Mitigation: `MAX_EXEMPLAR_RESULTS = 4`; tune down if context bloat becomes a problem.
- **Drift between manifest drafts and post-fix files** (recurring across all prior slices). Mitigation: explicit reconciliation pass before final review, per the established pattern.
- **Subjectivity of "exemplary".** The reviewer brief explicitly asks for verification that each exemplar would be useful in production, not just pedagogically clean. We accept some judgment-call risk and rely on the per-batch reviewer to flag pedagogical-fluff candidates.

## Phase 3+ Continuation Outline

After Phase 2 (sketched only, per the original Phase 0 spec):
- Phase 3 — project context: a `.illu/style/` directory with project-specific overrides (which axioms apply, exemptions, severity).
- Phase 4 — design record: structured "we chose X over Y because Z" capture.
- Phase 5 — critique: a tool that surfaces axiom violations in a diff.
- Phase 6 — cost profile: per-axiom token-budget weight.

## Open Questions for User Review

- Confirm Architecture B (real Rust files + JSON manifest + compile-check test). The user already approved this; this question is just a checkpoint.
- Confirm 3-3-3 batching by area (errors+ownership / types / perf+unsafe-FFI) vs by complexity (foundational / mid-level / advanced).
- Confirm 9 initial exemplars vs a smaller infra-only first cut (3 exemplars to ship the tool, more in Phase 2.1).
- Any candidate exemplars to drop or substitute before drafting begins.
