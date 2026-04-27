# Jon-Style Rust — Phase 1, Slice 3 (Performance) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 1 third slice — performance/codegen axioms.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md), [Slice 1 spec](2026-04-27-jon-style-rust-phase-1-ownership-design.md), [Slice 2 spec](2026-04-27-jon-style-rust-phase-1-types-design.md). Architecture and pipeline inherited.

## Motivation

Phases 0/1S1/1S2 added 28 axioms (errors, ownership, types). Slice 3 covers the performance area: where allocations come from, how to avoid unnecessary heap traffic, dispatch codegen tradeoffs, and memory layout. Source material: *Rust for Rustaceans* ch. 9 (Optimizing), the Rust Performance Book, and Crust-of-Rust on atomics.

**Verification discipline.** Per the Slice 2 final reviewer's recommendation: performance claims are quantitative and unforgiving. Each axiom must cite at least one of: a Godbolt permalink (codegen claims), `cargo bench` / iai-callgrind output (runtime claims), `cargo llvm-lines` / `cargo bloat` (binary size claims), or an authoritative documentation source (Rust Reference, Rust Performance Book, std docs). Source citations use the same chapter-granular placeholder format as prior slices; tighten during review if exact links are at hand.

## Goal

Land 9 new performance/codegen axioms (IDs 85–93) in `assets/rust_quality_axioms.json`, each user-reviewed and source-cited, with 3 per-batch focused-query coverage tests + 1 end-to-end demo test. Count assertion bumped 84 → 93.

## Scope

**In scope:**
- 9 new axioms across 3 thematic batches (3-3-3).
- 3 per-batch focused-query coverage tests + 1 end-to-end demo test.

**Explicit non-goals:**
- No schema changes.
- No new MCP tools or `.illu/style/` infrastructure.
- No quality-gate integration changes.
- No axioms outside performance (unsafe/FFI remains in Slice 4).
- No modification of existing performance axioms ([Performance Discipline], [Performance CI], [Benchmark Methodology], [Benchmark Coverage]) — the new axioms are tactical patterns; the existing ones are the meta-rules.

## Architecture

Unchanged. Append to `assets/rust_quality_axioms.json`; existing parse-once-cache layer ingests them.

## Schema

Per Phase 0. Each new entry uses the existing `Axiom` JSON shape.

## Candidate Axiom List

**Batch 1 — Allocation discipline** (where allocations come from and how to avoid them):
1. **Allocation in hot paths** (id 85) — preallocate `Vec`/`String` with `with_capacity`; avoid `format!` in tight loops; reuse buffers via `.clear()`.
2. **String allocation strategy** (id 86) — `&'static str` for compile-time, `&str` for borrowed, `Box<str>` for owned-no-grow (saves ~8 bytes vs `String`), `String` for owned+grow, `Cow<'a, str>` for sometimes-borrowed.
3. **Iterator semantics over indexed access** (id 87) — `iter()`/`iter_mut()` carries the "touch each element once" guarantee; reliable bounds-check elision and autovectorization.

**Batch 2 — Dispatch and codegen** (binary size, inlining, closed-set alternatives):
4. **Monomorphization budget** (id 88) — generics duplicate code per concrete `T`; profile with `cargo llvm-lines` / `cargo bloat`. Hot path = generics; cold or rarely-used = `dyn`.
5. **Inlining hints** (id 89) — `#[inline]` (cross-crate suggestion), `#[inline(always)]` (force, rarely right), `#[inline(never)]` (disable for debugging/profile attribution).
6. **Closed-set enum dispatch over `dyn Trait`** (id 90) — when the variant set is fixed, an enum dispatches faster than a vtable; subsumes `dyn` for plugin-shaped APIs that don't need open extension. Note: when variants vary wildly in size, `Vec<Box<dyn Trait>>` may use less memory — measure both.

**Batch 3 — Memory and atomics + demo** (layout, ordering, when not to allocate):
7. **Struct field layout for size** (id 91) — Rust default reorders for minimal padding; `#[repr(C)]` freezes declaration order, so order fields by descending alignment manually. Cache-line considerations (64-byte typical) for cores-shared hot structs.
8. **Atomic ordering selection** (id 92) — `Relaxed` (atomicity only), `Acquire`/`Release` (synchronizes-with pairs), `AcqRel` (RMW), `SeqCst` (total order, expensive). Pick the weakest that satisfies the actual synchronization need.
9. **Don't `Box` when stack works** (id 93) — `Box<T>` is for trait objects, recursive types, or genuinely large moves. Don't wrap small `Copy`/`Clone` types in `Box` for "indirection" — alloc + dealloc + pointer chase for nothing.

## Drafting and Review Loop

Same as prior slices. Per axiom: assistant drafts; batch of 3; user reviews; commit per accepted batch.

**Pre-flight verification standard for performance content:** every claim requires a verification source per the Slice 2 reviewer's note. The reviewer flagged that performance claims won't be caught by `cargo test` (compile-or-not is binary; perf is quantitative). For each new axiom, the implementer prompt should expect a `source` field that lists the verification reference (Godbolt link, `cargo llvm-lines` output, Rust Performance Book section, std docs).

## Verification and Exit Criteria

Phase 1 Slice 3 is complete when:
- 9 new performance axioms drafted, reviewed, merged.
- All existing tests pass.
- 3 new per-batch focused-query coverage tests + 1 demo test.
- Count assertion bumped 84 → 93.
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.

## Risks and Mitigations

- **Performance claims are quantitative and easy to get wrong.** Mitigation: cite Godbolt/cargo bench/std docs in the `source` field; pre-flight check each draft with a quick compile-and-eyeball before dispatch; engage code-quality reviewer per batch with explicit instruction to verify the quantitative claims.
- **Trigger collisions with existing performance axioms** ([Performance Discipline], [Benchmark Methodology], etc.). Mitigation: per-axiom focused queries; new categories chosen distinctly from existing.
- **Plan drift between drafts and post-fix JSON** (recurring across all prior slices). Mitigation: explicit reconciliation pass before final review.
- **Some claims are platform-dependent** (cache-line size, atomic costs). Mitigation: cite the typical x86-64 case explicitly; note when claims vary.

## Phase 1 Continuation Outline

After this slice (sketched only):
- Phase 1 Slice 4 — unsafe / FFI: invariant documentation discipline, MaybeUninit, FFI safety contracts.
- Phase 2 onward — exemplars, project context, design record, critique, cost profile.

## Open Questions for User Review

- Confirm batch grouping (3-3-3 thematic split).
- Confirm enum-dispatch axiom 90 in scope (alternative: defer if it's seen as too situational).
- Any candidate axioms to drop or merge before drafting begins.
