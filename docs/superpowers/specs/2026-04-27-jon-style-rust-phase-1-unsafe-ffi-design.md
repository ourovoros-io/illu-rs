# Jon-Style Rust — Phase 1, Slice 4 (Unsafe / FFI) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 1 fourth and final slice — `unsafe` and FFI axioms.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md), [Slice 1 spec](2026-04-27-jon-style-rust-phase-1-ownership-design.md), [Slice 2 spec](2026-04-27-jon-style-rust-phase-1-types-design.md), [Slice 3 spec](2026-04-27-jon-style-rust-phase-1-performance-design.md). Architecture and pipeline inherited.

## Motivation

Phases 0/1S1/1S2/1S3 added 37 axioms (errors, ownership, types, performance). Slice 4 closes Phase 1 by covering the area where Rust's safety guarantees are *traded* rather than *enforced*: `unsafe` blocks, raw memory primitives, and FFI boundaries. Source material: *Rust for Rustaceans* ch. 9 (Unsafe Code) and ch. 11 (Foreign Function Interface), Rustonomicon (Stacked Borrows, `MaybeUninit`, drop check), std docs for `MaybeUninit`/`UnsafeCell`/`NonNull`/`CStr`/`CString`/`std::panic::catch_unwind`.

**Verification discipline.** Unsafe correctness is *binary at compile time* (the code compiles either way) but *soundness* is subtle and decidable only against the Rust memory model. Per the Slice 3 reviewer's pattern: each axiom must cite at least one of the Rustonomicon, the Rust Reference, or std-docs entries (for the API in question). Source citations use the same chapter-granular placeholder format as prior slices; the per-batch reviewer must specifically check for:

- Soundness claims that conflate "compiles" with "sound" (e.g., "this is fine because the borrow checker accepts it" when the borrow checker doesn't see through raw pointers).
- `MaybeUninit` examples that read partially-initialized memory or use `assume_init` early.
- FFI examples that drop ownership rules, expose `&T`/`&mut T` across the boundary, or allow Rust panics to unwind into C.

## Goal

Land 9 new unsafe/FFI axioms (IDs 94–102) in `assets/rust_quality_axioms.json`, each user-reviewed and source-cited, with 3 per-batch focused-query coverage tests + 1 end-to-end demo test. Count assertion bumped 93 → 102.

## Scope

**In scope:**
- 9 new axioms across 3 thematic batches (3-3-3): `unsafe` discipline / memory primitives / FFI safety contracts.
- 3 per-batch focused-query coverage tests + 1 end-to-end demo test.

**Explicit non-goals:**
- No schema changes.
- No new MCP tools or `.illu/style/` infrastructure.
- No quality-gate integration changes.
- No axioms outside `unsafe`/FFI (Phase 2+ exemplars and project-context features remain in their own slices).
- No modification of the existing Pin/Unpin axiom (id 75): that one is the type-system contract; the new axioms cover memory-model and FFI surface concerns, which are orthogonal.

## Architecture

Unchanged. Append to `assets/rust_quality_axioms.json`; existing parse-once-cache layer ingests them.

## Schema

Per Phase 0. Each new entry uses the existing `Axiom` JSON shape.

## Candidate Axiom List

**Batch 1 — `unsafe` discipline** (the contract around `unsafe` blocks and `unsafe fn`):
1. **`SAFETY:` comment on every `unsafe` block** (id 94) — pair each `unsafe { ... }` with a `// SAFETY: <which invariants this caller satisfies>` comment naming the obligations from the API. Convention enforced by `clippy::undocumented_unsafe_blocks`.
2. **`unsafe fn` is a contract, not a warning** (id 95) — declaring `unsafe fn foo()` says "calling this requires invariants the type system cannot check." Document those invariants in a `# Safety` rustdoc section; callers must repeat them in their own `SAFETY:` comment. Don't mark a function `unsafe` defensively.
3. **Smallest-possible `unsafe` blocks** (id 96) — wrap only the operation that actually needs `unsafe`, not the surrounding logic. A 1-line `unsafe { ... }` is auditable; a 50-line `unsafe { ... }` is a code smell that hides which operations actually require it.

**Batch 2 — Memory primitives** (the raw mechanisms safe abstractions are built on):
4. **`MaybeUninit<T>` for delayed initialization** (id 97) — `mem::uninitialized` is UB-by-default and removed; use `MaybeUninit<T>` with field-by-field writes through `addr_of_mut!`, then `assume_init()` only once fully initialized. Never read partially-initialized memory.
5. **`UnsafeCell<T>` is the only sound interior-mutability primitive** (id 98) — every safe interior-mutability type (`Cell`, `RefCell`, `Mutex`, atomics) is built on `UnsafeCell` because it's the only type that legally allows `&T → &mut T` aliasing. Don't roll a custom `Cell`; use one of the existing wrappers, or build directly on `UnsafeCell` with documented invariants.
6. **Aliasing and pointer provenance** (id 99) — `&T` and `&mut T` impose strict aliasing (the Stacked Borrows model: no live `&T` while a `&mut` is active, and vice versa); raw pointers can alias but you still owe the validity invariant. Never materialize a `&mut T` from a raw pointer when another reference is live to the same location.

**Batch 3 — FFI safety contracts** (the boundary with C/C++):
7. **`extern "C"` panic and reference discipline** (id 100) — never let a Rust panic unwind across the FFI boundary (use `std::panic::catch_unwind` at the boundary; unwinding into C is UB by default); never expose Rust `&T`/`&mut T` to C (raw pointers only); never expose generics through `extern "C"`.
8. **`#[repr(C)]` and FFI-safe types** (id 101) — types crossing the boundary need stable layout: `#[repr(C)]` for structs, `#[repr(C)]` or `#[repr(u8)]` for enums; use `Option<NonNull<T>>` for niche-optimized nullable pointers; primitive types (`bool`, integers) need explicit ABI mapping (`c_int`, `c_uchar` from `std::ffi`).
9. **C string and buffer ownership** (id 102) — strings cross as `*const c_char` with documented ownership: caller allocates and frees, callee may not retain past the call. Use `CStr::from_ptr` for borrows, `CString::into_raw`/`from_raw` for owned transfers. Buffers cross as `(ptr, len)` pairs, never as raw pointers alone.

## Drafting and Review Loop

Same as prior slices. Per axiom: assistant drafts; batch of 3; user reviews; commit per accepted batch.

**Pre-flight verification standard for unsafe/FFI content:** every claim requires a verification source per the discipline note above. The reviewer flagged in Slices 1, 2, and 3 that subtle areas need authoritative-doc cross-checking before dispatch. For each new axiom, the implementer prompt should expect a `source` field that lists the verification reference (Rustonomicon section, Rust Reference §Unsafety, std docs page).

## Verification and Exit Criteria

Phase 1 Slice 4 is complete when:
- 9 new unsafe/FFI axioms drafted, reviewed, merged.
- All existing tests pass.
- 3 new per-batch focused-query coverage tests + 1 demo test.
- Count assertion bumped 93 → 102.
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.

## Risks and Mitigations

- **Soundness claims are subtle and easy to get wrong.** Mitigation: cite Rustonomicon / Rust Reference / std docs in the `source` field; the per-batch reviewer must specifically check soundness claims against authoritative docs, not just "this compiles" cargo output.
- **Trigger collisions with existing unsafe-adjacent axioms.** The existing axioms reference `unsafe` in `[PhantomData]`, `[Pin/Unpin]` (id 75), `[Send/Sync]`, and the variance/object-safety entries (Slices 1 & 2). Mitigation: per-axiom focused queries; new categories chosen distinctly (`Unsafe Discipline`, `MaybeUninit`, `UnsafeCell`, `Aliasing`, `FFI ABI`, `repr(C)`, `FFI Strings`).
- **Plan drift between drafts and post-fix JSON** (recurring across all four slices now). Mitigation: explicit reconciliation pass before final review, per the established pattern.
- **Some FFI claims are platform-dependent.** Unwinding-across-boundary UB is the default but `extern "C-unwind"` was stabilized; `bool` ABI was stabilized in Rust 1.71. Mitigation: cite the platform/version explicitly when behavior depends on it; the relevant guarantees are now standard.

## Phase 1 Continuation Outline

After this slice, Phase 1 is complete. Total axiom count: 102 (10 errors + 9 ownership + 9 types + 9 performance + 9 unsafe/FFI = 46 new from this campaign).

Phase 2 onward (sketched only, per Phase 0 spec):
- Phase 2 — exemplars infrastructure: a curated `assets/rust_exemplars/` directory of "ideal" Rust patterns, surfaced via a new `mcp__illu__exemplars` tool.
- Phase 3 — project context: a `.illu/style/` directory with project-specific overrides and exceptions.
- Phase 4 — design record: structured capture of design decisions ("we chose X over Y because Z").
- Phase 5 — critique: a tool that surfaces violations of the axioms in a diff.
- Phase 6 — cost profile: per-axiom budget on token cost / quality-gate weight.

## Open Questions for User Review

- Confirm batch grouping (3-3-3 thematic split as described).
- Confirm pointer-provenance axiom 99 in scope (alternative: defer to a hypothetical Phase 2 "advanced unsafe" slice if seen as too deep). Pre-flight assessment: yes, in scope — it's the operational form of the Stacked Borrows model and orthogonal to the other axioms.
- Any candidate axioms to drop or merge before drafting begins (axiom 96 "smallest-`unsafe`-blocks" is the most editorial; could be folded into 94).
