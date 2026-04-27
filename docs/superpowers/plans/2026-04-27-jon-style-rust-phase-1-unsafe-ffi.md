# Jon-Style Rust Phase 1 Slice 4 (Unsafe / FFI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Land 9 new unsafe/FFI axioms (IDs 94–102) in `assets/rust_quality_axioms.json`, with 3 per-batch coverage tests + 1 demo test. Count assertion bumped 93 → 102.

**Architecture:** Unchanged from prior phases. Pure content enrichment.

**Tech Stack:** Rust 2024, `serde_json`, `cargo test` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo fmt --all -- --check`.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-1-unsafe-ffi-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-1-unsafe-ffi-design.md)

**Existing state:** 93 axioms (Phase 0 + Slice 1 + Slice 2 + Slice 3). Test asserts `rust_quality_axiom_count == 93`. New IDs: 94–102.

**Drafts pre-approved by user.** Skip the user-review step inside batch tasks.

**Verification discipline.** Unsafe correctness is binary at compile time but soundness is decidable only against the Rust memory model. Per-batch reviewer must specifically check soundness claims against authoritative docs (Rustonomicon, Rust Reference §Unsafety, std docs), not just `cargo test` exit status. Common bugs to flag:
- Soundness claims that conflate "compiles" with "sound".
- `MaybeUninit` examples that read partially-initialized memory or use `assume_init` early.
- FFI examples that drop ownership rules, expose `&T`/`&mut T` across the boundary, or allow Rust panics to unwind into C (UB by default).

**Per-batch tests use focused queries** (Phase 0 / Slice 1-3 standard).

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `assets/rust_quality_axioms.json` | Append 9 entries | New axiom data |
| `src/server/tools/axioms.rs` | Add 4 test fns + extend 1 | Per-batch coverage tests + unsafe/FFI demo test |

---

## Task 1: Batch 1 — `unsafe` discipline (SAFETY comments, unsafe-fn contracts, smallest-unsafe-blocks)

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries before the closing `]`, after `rust_quality_93_heap_discipline`)
- Modify: `src/server/tools/axioms.rs` (add `test_unsafe_axioms_batch_1_present`; bump count assertion 93 → 96)

- [ ] **Step 1: Confirm starting ID**

```bash
grep -oE '"id": "rust_quality_[0-9]+_' assets/rust_quality_axioms.json | grep -oE '[0-9]+' | sort -n | tail -1
```
Expected: `93`. If different, bump IDs accordingly.

- [ ] **Step 2: Write the failing batch-1 coverage test**

```rust
    #[test]
    fn test_unsafe_axioms_batch_1_present() {
        let result = handle_axioms(
            "SAFETY comment unsafe block invariants undocumented_unsafe_blocks audit unsafe",
        )
        .unwrap();
        assert!(
            result.contains("Unsafe Block Discipline"),
            "Unsafe Block Discipline missing in focused query"
        );

        let result = handle_axioms(
            "unsafe fn safety preconditions # Safety rustdoc missing_safety_doc caller contract",
        )
        .unwrap();
        assert!(
            result.contains("Unsafe Fn Contract"),
            "Unsafe Fn Contract missing in focused query"
        );

        let result = handle_axioms(
            "smallest unsafe block scope minimize unsafe surface narrow unsafe block",
        )
        .unwrap();
        assert!(
            result.contains("Unsafe Block Scope"),
            "Unsafe Block Scope missing in focused query"
        );
    }
```

- [ ] **Step 3: Run the test, expect failure**

Run: `cargo test --lib -- test_unsafe_axioms_batch_1_present`. Expected: FAIL.

- [ ] **Step 4: SKIP — drafts pre-approved**

- [ ] **Step 5: Append approved drafts to JSON**

**Draft 1.1 — SAFETY comment on every unsafe block (id: rust_quality_94_unsafe_block_discipline):**

```json
{
  "id": "rust_quality_94_unsafe_block_discipline",
  "category": "Unsafe Block Discipline",
  "source": "Rust Reference §Unsafety; Rustonomicon §Working with Unsafe; clippy::undocumented_unsafe_blocks",
  "triggers": ["SAFETY comment naming invariants", "unsafe block invariants", "undocumented_unsafe_blocks", "clippy undocumented unsafe", "audit trail unsafe", "invariants from # Safety section"],
  "rule_summary": "Every `unsafe { ... }` block must be paired with a `// SAFETY: <which invariants this caller satisfies>` comment immediately above it. The comment names the obligations from the called API's `# Safety` section and explains why this caller meets them. The convention is enforced project-wide by `clippy::undocumented_unsafe_blocks`. A bare `unsafe { ... }` is a code review red flag — it asserts soundness without justifying it.",
  "prompt_injection": "MANDATORY RULE: Never write a bare `unsafe { ... }` block. Pair each one with a `// SAFETY: ...` comment that names the specific invariants you are asserting and why they hold here (e.g., `the slice is non-empty by the early return above`, `the pointer comes from Box::into_raw and was never reborrowed`). The comment is the audit trail; it is what makes the unsafe block reviewable.",
  "anti_pattern": "fn first_unchecked<T>(v: &[T]) -> &T {\n    // No SAFETY comment — reviewer cannot tell whether the caller checked emptiness.\n    unsafe { v.get_unchecked(0) }\n}",
  "good_pattern": "fn first_unchecked<T>(v: &[T]) -> &T {\n    debug_assert!(!v.is_empty());\n    // SAFETY: caller-visible debug_assert and the &[T] type guarantee no other\n    // mutable borrow exists; index 0 is in bounds because v is non-empty by\n    // the function's documented precondition.\n    unsafe { v.get_unchecked(0) }\n}"
}
```

**Draft 1.2 — `unsafe fn` is a contract (id: rust_quality_95_unsafe_fn_contract):**

```json
{
  "id": "rust_quality_95_unsafe_fn_contract",
  "category": "Unsafe Fn Contract",
  "source": "Rust Reference §unsafe-fn; Rustonomicon §Safe and Unsafe APIs; clippy::missing_safety_doc",
  "triggers": ["unsafe fn", "# Safety doc", "missing_safety_doc", "caller invariants", "unsafe fn contract", "safety preconditions"],
  "rule_summary": "Declaring `unsafe fn foo()` says \"calling this requires invariants the type system cannot check.\" Document those preconditions in a `# Safety` rustdoc section above the function; callers must, in their own `// SAFETY:` comment at each call site, explain how *they* satisfy each precondition (substituting concrete justifications, not just restating the contract). Don't mark a function `unsafe` defensively — only mark it when a caller can break memory safety by misusing it. `clippy::missing_safety_doc` flags `pub unsafe fn` without `# Safety` docs.",
  "prompt_injection": "MANDATORY RULE: When you write `unsafe fn`, write a `/// # Safety` rustdoc section listing every precondition a caller must uphold (e.g., \"`ptr` must be non-null and valid for `len * size_of::<T>()` bytes\"). If you cannot enumerate the preconditions, the function should not be `unsafe` — it is either safe (reword the API) or unsoundly designed (reshape the API).",
  "anti_pattern": "// `unsafe` with no documentation — a caller has no way to know what to verify.\npub unsafe fn copy_n<T>(src: *const T, dst: *mut T, n: usize) {\n    std::ptr::copy_nonoverlapping(src, dst, n);\n}",
  "good_pattern": "/// Copies `n` values of `T` from `src` to `dst`.\n///\n/// # Safety\n///\n/// - `src` must be valid for reads of `n * size_of::<T>()` bytes.\n/// - `dst` must be valid for writes of `n * size_of::<T>()` bytes.\n/// - The two regions must not overlap.\n/// - Both pointers must be properly aligned for `T`.\npub unsafe fn copy_n<T>(src: *const T, dst: *mut T, n: usize) {\n    // SAFETY: the four preconditions in this function's `# Safety` section\n    // are exactly those of `ptr::copy_nonoverlapping` (read-validity,\n    // write-validity, non-overlap, alignment); we forward them unchanged\n    // from the caller, who has signed our contract.\n    unsafe { std::ptr::copy_nonoverlapping(src, dst, n) }\n}"
}
```

**Draft 1.3 — Smallest-possible unsafe blocks (id: rust_quality_96_unsafe_block_scope):**

```json
{
  "id": "rust_quality_96_unsafe_block_scope",
  "category": "Unsafe Block Scope",
  "source": "Rust for Rustaceans, ch. 9 §Working with Unsafe; Rustonomicon §Working with Unsafe",
  "triggers": ["smallest unsafe block", "minimize unsafe surface", "narrow unsafe block", "unsafe block scope", "auditable unsafe"],
  "rule_summary": "Wrap only the operation that genuinely requires `unsafe`, not the surrounding logic. A 1- or 2-line `unsafe { ... }` block isolates exactly the operation under review; a multi-line block hides which operations are actually unsafe inside otherwise-safe code. Bounds checks, pointer construction, and length computation should live outside the unsafe block whenever possible.",
  "prompt_injection": "MANDATORY RULE: When you reach for `unsafe`, scope it to the exact unsafe operation. Compute pointers, lengths, and bounds checks in safe code; drop into `unsafe { ... }` only for the dereference, the `from_raw_parts`, the `assume_init`, etc. The reviewer should be able to read the unsafe block in isolation and see the single operation it covers.",
  "anti_pattern": "// One large unsafe block hides which line is the actually-unsafe one.\nunsafe fn split_at_mut_unchecked<T>(s: &mut [T], mid: usize) -> (&mut [T], &mut [T]) {\n    unsafe {\n        let len = s.len();\n        let ptr = s.as_mut_ptr();\n        // bounds reasoning, pointer arithmetic, and the actual unsafe ops all\n        // live inside the same block — reviewer must squint at every line.\n        (\n            std::slice::from_raw_parts_mut(ptr, mid),\n            std::slice::from_raw_parts_mut(ptr.add(mid), len - mid),\n        )\n    }\n}",
  "good_pattern": "/// # Safety\n/// `mid <= s.len()` must hold.\nunsafe fn split_at_mut_unchecked<T>(s: &mut [T], mid: usize) -> (&mut [T], &mut [T]) {\n    let len = s.len();\n    let ptr = s.as_mut_ptr();\n    // SAFETY: caller guarantees mid <= len. ptr came from `&mut [T]` so it is\n    // non-null, properly aligned for T, points to len consecutive initialized\n    // values, and the allocation fits within isize::MAX. The two halves are\n    // disjoint by construction so no aliasing &mut is created and no other\n    // pointer reads or writes this memory for the borrow's lifetime.\n    let left = unsafe { std::slice::from_raw_parts_mut(ptr, mid) };\n    // SAFETY: same caller guarantee and same alignment/initialization/size\n    // properties carried over from the original `&mut [T]`. `ptr.add(mid)` is\n    // in-bounds because mid <= len, and the right half is disjoint from the\n    // left because their byte ranges do not overlap.\n    let right = unsafe { std::slice::from_raw_parts_mut(ptr.add(mid), len - mid) };\n    (left, right)\n}"
}
```

After insertion, validate JSON: `python3 -c "import json; json.load(open('assets/rust_quality_axioms.json')); print('valid')"`.

- [ ] **Step 6: Bump count assertion 93 → 96**

In `src/server/tools/axioms.rs`, change `assert_eq!(rust_quality_axiom_count, 93);` to `assert_eq!(rust_quality_axiom_count, 96);`.

- [ ] **Step 7: Run the batch-1 test, expect pass**

Run: `cargo test --lib -- test_unsafe_axioms_batch_1_present`. Expected: PASS.

- [ ] **Step 8: Run the full test suite**

Run: `cargo test --lib`. Expected: all pass.

- [ ] **Step 9: Lint and format**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 10: Commit batch 1**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add unsafe batch 1 (block discipline, fn contract, block scope)

Adds three entries on the contract surrounding `unsafe`: every unsafe
block needs a // SAFETY: comment naming the invariants the caller
satisfies; every unsafe fn needs a /// # Safety rustdoc section listing
the preconditions; unsafe blocks should be scoped to the exact unsafe
operation, not the surrounding safe logic.
EOF
)"
```

---

## Task 2: Batch 2 — Memory primitives (MaybeUninit, UnsafeCell, aliasing/provenance)

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_unsafe_axioms_batch_2_present`; bump count assertion 96 → 99)

- [ ] **Step 1: Write the failing batch-2 coverage test**

```rust
    #[test]
    fn test_unsafe_axioms_batch_2_present() {
        let result = handle_axioms(
            "MaybeUninit uninitialized memory addr_of_mut assume_init partially initialized",
        )
        .unwrap();
        assert!(
            result.contains("MaybeUninit"),
            "MaybeUninit missing in focused query"
        );

        let result = handle_axioms(
            "UnsafeCell interior mutability primitive shared mutability Cell RefCell Mutex source",
        )
        .unwrap();
        assert!(
            result.contains("UnsafeCell"),
            "UnsafeCell missing in focused query"
        );

        let result = handle_axioms(
            "aliasing pointer provenance Stacked Borrows reference mut overlap raw pointer to mut",
        )
        .unwrap();
        assert!(
            result.contains("Aliasing"),
            "Aliasing missing in focused query"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

- [ ] **Step 3: SKIP — drafts pre-approved**

- [ ] **Step 4: Append approved drafts to JSON**

**Draft 2.1 — MaybeUninit for delayed initialization (id: rust_quality_97_maybe_uninit):**

```json
{
  "id": "rust_quality_97_maybe_uninit",
  "category": "MaybeUninit",
  "source": "std::mem::MaybeUninit; Rustonomicon §Working with Uninitialized Memory; std::ptr::addr_of_mut!",
  "triggers": ["MaybeUninit", "uninitialized memory", "addr_of_mut", "assume_init", "partially initialized", "mem::uninitialized"],
  "rule_summary": "`mem::uninitialized<T>()` is undefined behavior for almost every `T` and is deprecated; reach for `MaybeUninit<T>` instead. Pattern: `let mut x = MaybeUninit::<T>::uninit();` then write each field through `addr_of_mut!((*x.as_mut_ptr()).field).write(value)` (or, on Rust 1.82+, the equivalent `&raw mut (*x.as_mut_ptr()).field`); never go through a `&mut T` to the partially-initialized value. Call `assume_init()` only once *every* field is initialized. Critical: constructing a `&mut T` to a `MaybeUninit<T>` is itself undefined behavior — the `&mut` is invalid the moment it exists, regardless of whether you read from it.",
  "prompt_injection": "MANDATORY RULE: For delayed initialization use `MaybeUninit<T>`, not `mem::uninitialized` (deprecated, immediate UB for non-zero-sized non-Copy types). Write fields through raw pointers via `addr_of_mut!` (or `&raw mut`), never through a `&mut T` to the partially-initialized value. Call `assume_init()` only after every field has been written.",
  "anti_pattern": "use std::mem::MaybeUninit;\nstruct Pair { a: u32, b: String }\nlet mut p = MaybeUninit::<Pair>::uninit();\n// UB: &mut Pair is invalid because Pair is not yet initialized; the mere\n// existence of this reference is undefined behavior, even before reading.\nlet pref: &mut Pair = unsafe { &mut *p.as_mut_ptr() };\npref.a = 1;\npref.b = String::from(\"hi\");",
  "good_pattern": "use std::mem::MaybeUninit;\nuse std::ptr::addr_of_mut;\nstruct Pair { a: u32, b: String }\nlet mut p = MaybeUninit::<Pair>::uninit();\nlet ptr = p.as_mut_ptr();\n// SAFETY: addr_of_mut! takes a place expression and yields a raw pointer\n// without materializing a &mut to the partially-initialized Pair.\nunsafe {\n    addr_of_mut!((*ptr).a).write(1);\n    addr_of_mut!((*ptr).b).write(String::from(\"hi\"));\n}\n// SAFETY: every field of Pair has been initialized above.\nlet pair: Pair = unsafe { p.assume_init() };"
}
```

**Draft 2.2 — UnsafeCell as the only sound interior-mutability primitive (id: rust_quality_98_unsafe_cell):**

```json
{
  "id": "rust_quality_98_unsafe_cell",
  "category": "UnsafeCell",
  "source": "std::cell::UnsafeCell; Rust Reference §Interior Mutability; Rustonomicon §Aliasing",
  "triggers": ["UnsafeCell", "interior mutability primitive", "shared mutability", "Cell RefCell Mutex source", "shared reference to mutable conversion"],
  "rule_summary": "`UnsafeCell<T>` is the *only* type the language permits to obtain a `*mut T` from a shared reference (`&UnsafeCell<T>` → `.get()` → `*mut T`). Every safe interior-mutability type (`Cell`, `RefCell`, `Mutex`, `RwLock`, atomics, `OnceCell`) is built on top of it. Casting `&T` to `&mut T` through any other path is undefined behavior even if no one is currently reading the `&T`. Don't roll a custom `Cell` with raw pointer casts; use the existing wrappers, or build directly on `UnsafeCell` with documented invariants.",
  "prompt_injection": "MANDATORY RULE: For shared-reference mutation, pick the safest existing wrapper (`Cell` for Copy, `RefCell` for !Copy single-threaded, `Mutex`/`RwLock` for cross-thread, atomics for primitives). If none fit, build on `UnsafeCell<T>` with explicit safety documentation. Never cast `*const T` to `*mut T` to mutate behind a shared reference unless `T` is inside an `UnsafeCell` — that's UB regardless of whether anyone observes the mutation.",
  "anti_pattern": "// The cast itself is fine — pointer-type punning is well-defined. The UB is\n// the *write through* `*mut i32` to memory that is borrowed as `&i32`: the\n// shared reference guarantees the value does not change while it is live, and\n// `value` is not inside an `UnsafeCell`. The borrow checker accepts the code,\n// but the abstract machine does not.\nfn bad_set(value: &i32, new: i32) {\n    let ptr = value as *const i32 as *mut i32;\n    unsafe { *ptr = new; }\n}",
  "good_pattern": "use std::cell::Cell;\n// Sound: Cell wraps an UnsafeCell internally and exposes a safe API.\nfn good_set(slot: &Cell<i32>, new: i32) {\n    slot.set(new);\n}\n// Or, when the existing wrappers don't fit, build on UnsafeCell directly:\nuse std::cell::UnsafeCell;\nstruct MyCell<T> { inner: UnsafeCell<T> }\nimpl<T> MyCell<T> {\n    /// # Safety\n    /// Caller must ensure no other reference (shared or unique) into `inner`\n    /// is live when this method runs. Validity, alignment, and initialization\n    /// are upheld by `MyCell`'s own invariants: `UnsafeCell::get` returns a\n    /// valid, properly-aligned `*mut T` that points to an initialized `T` for\n    /// as long as `&self` is live.\n    unsafe fn replace(&self, new: T) -> T {\n        // SAFETY: ptr::replace requires (1) valid for reads and writes, (2)\n        // properly aligned, (3) points to an initialized T. (1)-(3) follow\n        // from MyCell's structural invariants. The fourth requirement — no\n        // other live reference into `inner` — is the caller's obligation.\n        unsafe { std::ptr::replace(self.inner.get(), new) }\n    }\n}"
}
```

**Draft 2.3 — Aliasing and pointer provenance (id: rust_quality_99_aliasing):**

```json
{
  "id": "rust_quality_99_aliasing",
  "category": "Aliasing",
  "source": "Rustonomicon §Aliasing; Rustonomicon §References; Stacked Borrows model (Rust unsafe code guidelines)",
  "triggers": ["aliasing", "pointer provenance", "Stacked Borrows", "&mut from raw", "reference aliasing", "raw pointer to mut"],
  "rule_summary": "Rust's reference rules apply *everywhere a reference is live*, not just where the borrow checker can see them: a `&mut T` is unique (no other reference to the same location), a `&T` is shared (any number of `&T`, but no `&mut T` to the same location). Raw pointers do not impose aliasing rules, but the moment you materialize a `&mut T` from a raw pointer, every other live reference to that location becomes invalid under the Stacked Borrows model. Keep raw pointers raw until the last moment; convert to `&T`/`&mut T` only when you can prove no other reference is live.",
  "prompt_injection": "MANDATORY RULE: Treat references as exclusive (`&mut`) or shared (`&`) globally, not lexically. When you cross between safe and unsafe code with raw pointers, do not materialize a `&mut T` from a raw pointer if any other reference to the same allocation is live — even a `&T`, even one that was created earlier and is about to be dropped. The Stacked Borrows model says the `&mut` invalidates the earlier reference at the moment the `&mut` is created.",
  "anti_pattern": "let mut x = 1u32;\nlet ptr: *mut u32 = &mut x;\n// `shared` is a live &u32 borrow rooted at `ptr`.\nlet shared: &u32 = unsafe { &*ptr };\n// UB under Stacked Borrows: materializing &mut u32 from `ptr` invalidates\n// `shared` at the moment the &mut is created, before any read happens.\n// The subsequent read through `shared` is undefined behavior.\nlet exclusive: &mut u32 = unsafe { &mut *ptr };\n*exclusive = 2;\nlet _ = *shared;",
  "good_pattern": "let mut x = 1u32;\nlet ptr: *mut u32 = &mut x;\n// Stay in raw pointers while both reads/writes are needed; raw pointers do\n// not impose aliasing, so `read`/`write` through `ptr` is sound.\nunsafe {\n    let cur = ptr.read();\n    ptr.write(cur + 1);\n}\n// SAFETY: ptr is valid for the lifetime of x, and no other reference to x\n// is live at this point (the raw-pointer ops above produced no borrow).\nlet final_ref: &u32 = unsafe { &*ptr };\nassert_eq!(*final_ref, 2);"
}
```

After insertion, validate JSON parses.

- [ ] **Step 5: Bump count assertion 96 → 99**

- [ ] **Step 6: Run the batch-2 test, expect pass**

- [ ] **Step 7: Full test suite + lint + fmt**

- [ ] **Step 8: Commit batch 2**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add unsafe batch 2 (MaybeUninit, UnsafeCell, aliasing/provenance)

Adds three entries on the raw memory primitives safe abstractions are
built on: MaybeUninit<T> for delayed init via addr_of_mut! (never
materialize &mut to partially-initialized values); UnsafeCell<T> as the
only sound primitive for &T -> &mut T conversion (every safe interior-
mutability type is built on it); aliasing and pointer provenance under
Stacked Borrows (rules apply everywhere a reference is live, not just
where the borrow checker can see).
EOF
)"
```

---

## Task 3: Batch 3 — FFI safety contracts (extern C, repr(C), CStr/CString) + demo test

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_unsafe_axioms_batch_3_present` and `test_unsafe_demo_query_returns_new_axioms`; bump count assertion 99 → 102)

- [ ] **Step 1: Write the failing batch-3 coverage test**

```rust
    #[test]
    fn test_unsafe_axioms_batch_3_present() {
        let result = handle_axioms(
            "extern C panic catch_unwind FFI boundary unwind UB generic extern reference across FFI",
        )
        .unwrap();
        assert!(
            result.contains("FFI Boundary"),
            "FFI Boundary missing in focused query"
        );

        let result = handle_axioms(
            "repr(C) FFI safe layout stable Option NonNull c_int c_uchar FFI types",
        )
        .unwrap();
        assert!(
            result.contains("FFI Layout"),
            "FFI Layout missing in focused query"
        );

        let result = handle_axioms(
            "CStr from_ptr CString into_raw FFI string ownership c_char buffer ptr len pair",
        )
        .unwrap();
        assert!(
            result.contains("FFI Strings"),
            "FFI Strings missing in focused query"
        );
    }
```

- [ ] **Step 2: Run, expect fail**

- [ ] **Step 3: SKIP — drafts pre-approved**

- [ ] **Step 4: Append approved drafts**

**Draft 3.1 — extern "C" panic and reference discipline (id: rust_quality_100_ffi_boundary):**

```json
{
  "id": "rust_quality_100_ffi_boundary",
  "category": "FFI Boundary",
  "source": "Rust Reference §Foreign Function Interface; Rust for Rustaceans, ch. 11 §FFI; std::panic::catch_unwind; RFC 2945 (extern \"C-unwind\")",
  "triggers": ["extern C panic", "catch_unwind", "FFI boundary panic", "extern C generic", "FFI references", "unwind UB"],
  "rule_summary": "Three rules at the `extern \"C\"` boundary: (1) A Rust `panic!` that tries to unwind across the default `extern \"C\"` ABI is turned into a process abort (RFC 2945, stabilized 1.71); a *foreign* unwind (e.g. C++ exception) crossing `extern \"C\"` in either direction is still undefined behavior. To recover from panics with an error code instead of aborting, wrap the body in `std::panic::catch_unwind` and map `Err` into a sentinel value. The opt-in `extern \"C-unwind\"` ABI lets panics propagate across the boundary safely. (2) Never expose Rust references (`&T`/`&mut T`) across the boundary — C cannot uphold validity or aliasing invariants. Use `*const T`/`*mut T` and document the lifetime/ownership rules. (3) A generic `extern \"C\" fn foo<T>(...)` compiles, but it has no single mangled symbol for C to link against; always write a concrete monomorphic wrapper per `T` you expose.",
  "prompt_injection": "MANDATORY RULE: Every `extern \"C\" fn` body that wants to recover (rather than abort the process) on panic wraps its work in `std::panic::catch_unwind` and converts panics into a sentinel error value. Function signatures use `*const T`/`*mut T`, never `&T`/`&mut T`, and the rustdoc above the function documents who allocates, who frees, what null/length conventions hold, and the aliasing precondition. No generic `extern \"C\"` functions — they compile but have no single symbol for C to link against, so write concrete `extern \"C\" fn parse_u32_buffer(...)` wrappers around generic helpers.",
  "anti_pattern": "// Three problems stacked: panic crossing the boundary aborts the process\n// (not what a library author wants), &mut MyType is exposed (C cannot uphold\n// aliasing/validity invariants), and the function is generic (compiles, but\n// produces no single linkable symbol for C — the no_mangle_generic_items\n// warning fires for that reason).\n#[unsafe(no_mangle)]\npub extern \"C\" fn process<T>(value: &mut T) -> i32 {\n    panic!(\"unimplemented\"); // panic across extern \"C\" -> abort, not graceful\n}",
  "good_pattern": "use std::panic::catch_unwind;\n\n#[repr(C)]\npub struct MyType { field: u32 }\n\n/// Processes a MyType value.\n///\n/// # Safety\n/// `value` must be a non-null pointer to a valid MyType for the duration of\n/// the call, and no other reference (Rust or C) may alias `*value` while this\n/// call runs. Caller retains ownership; this function does not free.\n///\n/// Returns 0 on success, -1 on caller error, -2 on internal panic.\n#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn my_process(value: *mut MyType) -> i32 {\n    if value.is_null() { return -1; }\n    catch_unwind(|| {\n        // SAFETY: caller-documented non-null, validity, and exclusive-access\n        // preconditions; the &mut lives only inside this closure.\n        let v = unsafe { &mut *value };\n        v.field = v.field.saturating_add(1);\n        0\n    })\n    .unwrap_or(-2) // recover from panic with an error code instead of abort\n}"
}
```

**Draft 3.2 — repr(C) and FFI-safe types (id: rust_quality_101_ffi_layout):**

```json
{
  "id": "rust_quality_101_ffi_layout",
  "category": "FFI Layout",
  "source": "Rust Reference §Type Layout (repr(C), repr(<int>)); std::ptr::NonNull; std::ffi::{c_int, c_uchar, c_char}",
  "triggers": ["repr(C)", "FFI safe layout", "Option NonNull", "c_int c_uchar", "FFI types", "stable layout"],
  "rule_summary": "Types crossing the FFI boundary need a stable layout. `#[repr(C)]` on structs and unions matches the C ABI; `#[repr(<int>)]` (e.g. `#[repr(u8)]`) gives enums a stable discriminant layout; default `repr(Rust)` is *not* stable and must not cross. For nullable pointers use `Option<NonNull<T>>` — niche optimization makes it the same size as `*mut T` and uses the null bit pattern as `None`. Use the concrete ABI types from `std::ffi` (`c_int`, `c_uchar`, `c_char`) rather than Rust's `i32`/`u8`/`i8` to make the C-side mapping explicit.",
  "prompt_injection": "MANDATORY RULE: Every type that appears in an `extern \"C\"` signature or as a field of one must have an FFI-safe layout. Annotate structs and unions with `#[repr(C)]`; annotate enums with `#[repr(u8)]` (or another integer); use `Option<NonNull<T>>` instead of `*mut T` when null is meaningful (you get the null check for free in pattern matches); use `c_int`/`c_char`/`c_uchar` for primitives so the C header generator and the reader both see the intended ABI mapping.",
  "anti_pattern": "// repr(Rust) — the field order can change between compiler versions,\n// breaking C callers that assumed declaration order.\npub struct Header {\n    pub kind: u8,\n    pub length: u32,\n}\n// `improper_ctypes` warns: Vec<u8> has no stable layout. The Rust side\n// compiles (extern blocks are unsafe under 2024 edition); the lint exists\n// because *calling* this from C is undefined behavior — C cannot construct\n// a valid Vec<u8>.\nunsafe extern \"C\" {\n    fn read(out: Vec<u8>) -> i32;\n}",
  "good_pattern": "use std::ffi::{c_int, c_uchar};\nuse std::ptr::NonNull;\n\n#[repr(C)]\npub struct Header {\n    pub kind: c_uchar,\n    pub length: c_int,\n    // Niche-optimized nullable pointer: same size as *mut Body, distinguishes\n    // null at the type level so callers must pattern-match.\n    pub body: Option<NonNull<Body>>,\n}\n\n#[repr(C)]\npub struct Body {\n    pub bytes: *mut c_uchar,\n    pub len: usize,\n}\n\n#[repr(u8)]\npub enum Status { Ok = 0, Err = 1 }"
}
```

**Draft 3.3 — C string and buffer ownership (id: rust_quality_102_ffi_strings):**

```json
{
  "id": "rust_quality_102_ffi_strings",
  "category": "FFI Strings",
  "source": "std::ffi::{CStr, CString}; Rust Reference §Foreign Function Interface; Rust for Rustaceans, ch. 11 §Strings",
  "triggers": ["CStr from_ptr", "CString into_raw", "FFI string ownership", "c_char buffer", "ptr len pair", "C string ownership"],
  "rule_summary": "Strings cross the FFI boundary as `*const c_char` (or `*mut`) with documented ownership: the type does not encode who frees, so the rustdoc must. `CStr::from_ptr(p)` borrows for the duration of `'a` (caller retains ownership); `CString::into_raw()` transfers ownership *out* of Rust and a matching `CString::from_raw()` reclaims it back. Buffers (non-string) cross as `(ptr: *const T, len: usize)` pairs — never raw pointer alone, because length is not recoverable on the C side.",
  "prompt_injection": "MANDATORY RULE: Every C string parameter has its ownership documented in the rustdoc: \"caller retains and frees\" (use `CStr::from_ptr` for a borrow), or \"ownership transferred to Rust, will be freed by Rust\" (use `CString::from_raw`), or \"Rust transfers ownership; C must call <free_fn>\" (return `CString::into_raw`). Buffers are always `(ptr, len)` pairs; never expose a length-less raw pointer to a buffer.",
  "anti_pattern": "use std::ffi::c_char;\n// Returns a C string with no documented ownership convention. Caller has\n// no idea whether they should call free, a Rust-provided destructor, or\n// nothing at all (which leaks).\n#[unsafe(no_mangle)]\npub extern \"C\" fn build_message() -> *const c_char {\n    let s = std::ffi::CString::new(\"hello\").unwrap();\n    s.as_ptr() // dangling: CString drops at end of scope!\n}",
  "good_pattern": "use std::ffi::{CStr, CString, c_char};\n\n/// Borrows a C string for inspection. The caller retains ownership.\n///\n/// # Safety\n/// `s` must be a non-null, NUL-terminated C string valid for the call.\n#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn message_len(s: *const c_char) -> usize {\n    if s.is_null() { return 0; }\n    // SAFETY: caller's documented non-null + NUL-terminated precondition.\n    unsafe { CStr::from_ptr(s) }.to_bytes().len()\n}\n\n/// Builds a Rust-owned C string and transfers ownership to C.\n/// C must release the value with `free_message`, not `free`.\n#[unsafe(no_mangle)]\npub extern \"C\" fn build_message() -> *mut c_char {\n    CString::new(\"hello\").map(CString::into_raw).unwrap_or(std::ptr::null_mut())\n}\n\n/// Reclaims and frees a string previously returned by `build_message`.\n#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn free_message(s: *mut c_char) {\n    if s.is_null() { return; }\n    // SAFETY: caller passes a pointer obtained from CString::into_raw\n    // and never freed by any other path.\n    drop(unsafe { CString::from_raw(s) });\n}"
}
```

After insertion, validate JSON parses.

- [ ] **Step 5: Bump count assertion 99 → 102**

- [ ] **Step 6: Run the batch-3 test, expect pass**

- [ ] **Step 7: Add the unsafe/FFI demo-query test**

```rust
    #[test]
    fn test_unsafe_demo_query_returns_new_axioms() {
        let result = handle_axioms(
            "unsafe SAFETY comment unsafe fn smallest unsafe MaybeUninit UnsafeCell aliasing extern C panic repr(C) CStr buffer ownership FFI",
        )
        .unwrap();
        let new_categories = [
            "Unsafe Block Discipline",
            "Unsafe Fn Contract",
            "Unsafe Block Scope",
            "MaybeUninit",
            "UnsafeCell",
            "Aliasing",
            "FFI Boundary",
            "FFI Layout",
            "FFI Strings",
        ];
        let surfaced = new_categories
            .iter()
            .filter(|cat| result.contains(*cat))
            .count();
        assert!(
            surfaced >= 3,
            "expected at least 3 new unsafe/FFI categories in demo query, got {surfaced}"
        );
    }
```

- [ ] **Step 8: Run the demo test, expect pass**

- [ ] **Step 9: Run the full test suite**

- [ ] **Step 10: Lint and format**

- [ ] **Step 11: Commit batch 3**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add unsafe batch 3 (FFI boundary, layout, strings) + demo test

Adds the final three unsafe/FFI entries: extern \"C\" boundary discipline
(catch_unwind to convert panics into error codes; raw pointers, never
&T/&mut T; no generic extern \"C\"); repr(C) and FFI-safe types
(Option<NonNull<T>> for nullable pointers; c_int/c_uchar from std::ffi
for primitives); C string and buffer ownership (CStr::from_ptr borrows;
CString::into_raw/from_raw transfers; (ptr, len) for buffers, never
length-less). Adds an end-to-end demo test asserting >= 3 of the 9 new
unsafe/FFI categories surface for the broad query.
EOF
)"
```

---

## Task 4: End-to-End Verification

- [ ] **Step 1: Full cargo gauntlet**

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 2: Plan-reconciliation pass before final review** — if any content fix-ups landed during execution, ensure the plan's draft sections were updated to match the post-fix JSON.

---

## Verification Summary

After all tasks: 9 new axioms; 4 new tests; cargo gauntlet clean. Phase 1 closes at 102 total axioms (+46 from the campaign).

## Risks Realized During Execution

- **Soundness claims are subtle.** Per-batch reviewer must check claims against the Rustonomicon, Rust Reference, and std docs — not just `cargo test`.
- **Trigger collisions** with existing unsafe-adjacent axioms. Per-axiom focused queries verified by per-batch tests.
- **Plan drift** between drafts and post-fix JSON (recurring). Reconcile before final review.
