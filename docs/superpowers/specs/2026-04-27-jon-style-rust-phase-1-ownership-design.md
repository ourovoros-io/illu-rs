# Jon-Style Rust — Phase 1, Slice 1 (Ownership/Lifetime/Drop) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 1 first slice. Replicates Phase 0's vertical-slice methodology in the ownership/lifetime/drop area.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md) for architecture, schema, and pipeline. Phase 1 inherits all of those without modification.

## Motivation

Phase 0 landed 10 error-handling axioms (IDs 57–66) and validated the pipeline end-to-end (drafting → user review → JSON append → focused-query coverage tests → cargo gauntlet → live MCP surface after binary refresh). Phase 1 is the horizontal replication promised in the Phase 0 spec's "Phase 1+ outline" section: same pipeline, no new infrastructure, content-only enrichment of `assets/rust_quality_axioms.json` for the next style area.

This slice covers ownership, borrowing, lifetimes, and drop discipline — *Rust for Rustaceans* chapter 1 (Foundations) plus relevant Crust-of-Rust material on lifetime annotations, smart pointers, variance, and Pin.

## Goal

Land 9 new ownership/lifetime/drop axioms (IDs 67–75) in `assets/rust_quality_axioms.json`, each user-reviewed, source-cited, and surfaced via `mcp__illu__axioms` and `rust_preflight`. End state: a representative ownership query returns the new entries; the demo test confirms ≥3 of the 9 surface for a canonical broad ownership query.

## Scope

**In scope:**
- 9 new axioms across 3 batches (3-3-3).
- 3 per-batch focused-query coverage tests + 1 end-to-end demo test for the ownership slice.
- Count assertion bumped from 66 to 75.

**Explicit non-goals:**
- No schema changes to the `Axiom` struct.
- No new MCP tools or `.illu/style/` infrastructure (still deferred to Phase 2+).
- No quality-gate integration changes.
- No axioms outside the ownership/lifetime/drop area (concurrency-deep, perf, unsafe, types remain in later slices).

## Architecture

Unchanged from Phase 0. New entries append to `assets/rust_quality_axioms.json`; the existing `axioms()` parse-once-cache layer at [src/server/tools/axioms.rs:67-87](src/server/tools/axioms.rs:67) ingests them; `mcp__illu__axioms` and `rust_preflight` surface them automatically.

## Schema

Per Phase 0. Each new entry uses the existing `Axiom` JSON shape (`id`, `category`, `source`, `triggers`, `rule_summary`, `prompt_injection`, `anti_pattern`, `good_pattern`). Triggers remain the most important findability field.

Source citations use the same chapter-granular placeholder format as Phase 0 (`"Rust for Rustaceans, ch. 1 §<topic>"` or `"Crust of Rust: <video title>"`); user tightens during review if the book is at hand.

## Candidate Axiom List

Final list iterated during drafting. Some may collapse during review.

**Batch 1 — Borrows in motion** (how the borrow checker actually flows):
1. **NLL: borrows end at last use** (id 67) — non-lexical lifetimes; the borrow checker is smarter than the curly-brace scope.
2. **Reborrowing of `&mut T`** (id 68) — passing `&mut *m` (or implicit reborrow) lets you call multiple `&mut self` methods sequentially.
3. **References don't own** (id 69) — `&mut T` is a unique borrow; you can't drop it, only return it.

**Batch 2 — Variance and Drop** (type-level lifetime/drop reasoning):
4. **Variance discipline** (id 70) — covariance / invariance / contravariance via `PhantomData<&T>` vs `PhantomData<&mut T>` vs `PhantomData<fn() -> T>`.
5. **Drop order matters** (id 71) — struct fields drop in declaration order; design interdependent fields accordingly.
6. **Self-referential types need help** (id 72) — naive `struct { data: String, view: &str_into_data }` doesn't compile; use indices, arenas, `Pin`, or `ouroboros`.

**Batch 3 — Interior mutability + async-aware ownership** (controlled mutation):
7. **Interior mutability decision tree** (id 73) — `Cell` (single-threaded Copy) → `RefCell` (single-threaded compound) → atomics (thread-shared primitive) → `Mutex`/`RwLock` (thread-shared compound).
8. **`MutexGuard` across `.await` is a deadlock** (id 74) — locks must not span await points; drop before await or use `tokio::sync::Mutex`.
9. **Pin / Unpin** (id 75) — when self-referential or hand-rolled future types require pinning; why most types are `Unpin`.

## Drafting and Review Loop

Same as Phase 0:
1. Per axiom: assistant drafts category, triggers, rule_summary, prompt_injection, good_pattern, anti_pattern, source.
2. Batch: 3 axioms per batch.
3. Review: user keeps / rejects / edits / merges with existing entries.
4. Commit: one commit per accepted batch.

## Verification and Exit Criteria

Phase 1 Slice 1 is complete when **all** of the following hold:

- 9 new ownership/lifetime/drop axioms drafted, reviewed, and merged into `assets/rust_quality_axioms.json` (or fewer if some are rejected).
- All existing tests pass.
- 3 new per-batch focused-query coverage tests in `src/server/tools/axioms.rs`, all passing.
- 1 new end-to-end demo test asserts at least 3 of the new categories surface for a canonical broad ownership query.
- `test_axiom_assets_have_unique_ids_and_required_fields` count assertion bumped 66 → 75.
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.
- One end-to-end sample function exercise: design a struct with non-trivial ownership semantics via `rust_preflight`; the resulting code reflects guidance from at least one new axiom.

## Risks and Mitigations

- **Trigger collisions with existing ownership-related axioms** (Cow, Send/Sync, RefCell, lifetime elision). Mitigation: each new entry uses per-axiom focused queries that do not depend on broad-query ranking. Verified by per-batch test passing; canary in the demo test.
- **Drafted rules contradict or weaken existing axioms.** Mitigation: review-by-batch with reject as first-class outcome; the code-quality reviewer in the Phase 0 cycle caught two such issues (entry 25/66 relationship, entry 58 anti-pattern) and we apply the same diligence here.
- **Source citations imprecise.** Mitigation: same chapter-granular placeholder; user tightens during review.
- **Pin/Unpin axiom (id 75) overlaps with future Phase 1 async-deep slice.** Mitigation: this slice's Pin axiom focuses on the type-system contract (`Pin<P>` as a unique-pinning marker; most types are `Unpin`); deeper async-runtime patterns stay for the async slice.
- **Variance axiom (id 70) overlaps with the existing PhantomData axiom.** Mitigation: existing axiom covers `PhantomData<T>` for marker semantics; new axiom covers the variance choice (covariant `&T` vs invariant `&mut T` vs contravariant `fn() -> T`). Different concern; categories chosen distinctly.

## Phase 1+ Continuation Outline

After this slice (sketched only):
- Phase 1 Slice 2 — type-system: trait objects vs generics, sealed traits, marker trait discipline, orphan rule sharpening, ZST/DST distinctions.
- Phase 1 Slice 3 — performance: allocation in hot paths, monomorphization budget, dyn dispatch cost, layout/cache.
- Phase 1 Slice 4 — unsafe / FFI: invariant documentation discipline, MaybeUninit, FFI safety contracts.
- Phase 2 onward — exemplars infrastructure, project context, design record, critique, cost profile (per Phase 0 spec).

## Open Questions for User Review

- Confirm batch grouping (3-3-3 thematic split as described) or prefer different grouping.
- Confirm Pin/Unpin (id 75) is in scope for this slice rather than deferred to a future async-focused slice.
- Any candidate axioms to drop or merge before drafting begins.
