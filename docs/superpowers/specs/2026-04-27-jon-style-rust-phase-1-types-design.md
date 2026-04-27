# Jon-Style Rust — Phase 1, Slice 2 (Type-System) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 1 second slice. Replicates the vertical-slice methodology validated by Phase 0 (errors) and Phase 1 Slice 1 (ownership) for the type-system area.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md) for architecture, schema, and pipeline. [Phase 1 Slice 1 spec](2026-04-27-jon-style-rust-phase-1-ownership-design.md) for the slice-shape template. Both inherited without modification.

## Motivation

Phase 0 added 10 error-handling axioms (IDs 57–66). Phase 1 Slice 1 added 9 ownership/lifetime/drop axioms (IDs 67–75). Phase 1 Slice 2 covers the type-system: trait design, object safety, generic/dyn dispatch tradeoffs, associated types vs generics, HRTBs, `?Sized` discipline, ZSTs, and marker/auto traits.

Source material: *Rust for Rustaceans* ch. 2 (Types) and parts of ch. 3 (Designing Interfaces); Crust-of-Rust on object safety, sealed traits, and HRTBs.

## Goal

Land 9 new type-system axioms (IDs 76–84) in `assets/rust_quality_axioms.json`, each user-reviewed and source-cited, with 3 per-batch focused-query coverage tests + 1 end-to-end demo test. Count assertion bumped 75 → 84.

## Scope

**In scope:**
- 9 new axioms across 3 thematic batches (3-3-3).
- 3 per-batch focused-query coverage tests + 1 end-to-end demo test for the type-system slice.

**Explicit non-goals:**
- No schema changes to the `Axiom` struct.
- No new MCP tools or `.illu/style/` infrastructure (still deferred to Phase 2+).
- No quality-gate integration changes.
- No axioms outside the type-system area (performance, unsafe/FFI remain in later slices).

## Architecture

Unchanged from Phase 0/1S1. New entries append to `assets/rust_quality_axioms.json`; the existing `axioms()` parse-once-cache layer at [src/server/tools/axioms.rs:67-87](src/server/tools/axioms.rs:67) ingests them.

## Schema

Per Phase 0. Each new entry uses the existing `Axiom` JSON shape. Source citations use the same chapter-granular placeholder format.

## Candidate Axiom List

**Batch 1 — Trait surface and sealing** (controlling who implements and calls):
1. **Sealed traits** (id 76) — `pub trait Foo: private::Sealed {}` pattern blocks external impls while still letting external crates call methods.
2. **Object safety / dyn compatibility** (id 77) — rules for making a trait `dyn`-compatible: methods cannot be generic; receiver must be dispatchable; `Self: Sized` escape hatch for non-dispatchable methods.
3. **Object-safe API design** (id 78) — when to deliberately preserve dyn compatibility (extension traits, plugin systems) vs give it up for monomorphization-dependent perf.

**Batch 2 — Type and trait choice** (interface design tradeoffs):
4. **Generic vs trait object** (id 79) — monomorphization vs vtable; static for hot/homogeneous, dyn for heterogeneous/binary-size.
5. **Associated types vs generic parameters** (id 80) — associated types when one impl per type (`Iterator::Item`); generics when caller picks (`Add<RHS>`); avoid mixing.
6. **HRTBs (`for<'a> Fn(&'a T) -> &'a U`)** (id 81) — when callbacks need arbitrary lifetimes; how the syntax desugars; common errors trying to use a single named lifetime.

**Batch 3 — Type-level building blocks** (subtle primitives + demo):
7. **`?Sized` discipline** (id 82) — generic params are `Sized` by default; relax with `?Sized` for DSTs (`str`, `[T]`, `dyn Trait`); cost: only methods through references work.
8. **ZST markers** (id 83) — unit structs as zero-cost compile-time witnesses (typestate phases, capability tokens); `Vec<()>` never allocates element storage.
9. **Marker traits and auto traits** (id 84) — `Send`/`Sync`/`Sized`/`Unpin` are auto traits (compiler propagates from fields); `Eq`/`Hash`/`Ord` are coherence-bound (must agree). When to derive vs hand-implement.

## Drafting and Review Loop

Same as prior slices. Per axiom: assistant drafts; batch of 3; user reviews; commit per accepted batch.

## Verification and Exit Criteria

Phase 1 Slice 2 is complete when:
- 9 new type-system axioms drafted, reviewed, merged into `assets/rust_quality_axioms.json` (or fewer if rejected).
- All existing tests pass.
- 3 new per-batch focused-query coverage tests + 1 end-to-end demo test in `src/server/tools/axioms.rs`, all passing.
- `test_axiom_assets_have_unique_ids_and_required_fields` count assertion bumped 75 → 84.
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.

## Risks and Mitigations

- **Subtle technical content.** The Phase 1 Slice 1 reviewer caught real bugs in subtle topics (variance specifications, NLL examples, `Cell` description). Object safety, HRTBs, `?Sized`, and auto-trait propagation are similarly subtle. Mitigation: pre-flight technical sanity check on each draft against the Rust Reference / Rustonomicon before dispatch.
- **Trigger collisions with existing trait-related axioms** (`[Trait Design]`, `[Trait Objects]`, `[Traits]` for orphan rule, `[Type Safety]`). Mitigation: per-axiom focused queries; verify each new entry surfaces in a query built from its own triggers.
- **Drift between plan drafts and post-fix JSON** (recurring issue in Phases 0 and 1S1). Mitigation: pre-merge plan-reconciliation step before final review, like we did at the end of Slice 1.

## Phase 1 Continuation Outline

After this slice (sketched only):
- Phase 1 Slice 3 — performance: allocation in hot paths, monomorphization budget, dyn dispatch cost, layout/cache.
- Phase 1 Slice 4 — unsafe / FFI: invariant documentation discipline, MaybeUninit, FFI safety contracts.
- Phase 2 onward — exemplars infrastructure, project context, design record, critique, cost profile (per Phase 0 spec).

## Open Questions for User Review

- Confirm batch grouping (3-3-3 thematic split as described).
- Confirm HRTBs in scope for this slice (could be deferred to a later slice if too narrow); pre-flight assessment is yes — they're a fundamental Crust-of-Rust topic and orthogonal to other type-system rules.
- Any candidate axioms to drop or merge before drafting begins.
