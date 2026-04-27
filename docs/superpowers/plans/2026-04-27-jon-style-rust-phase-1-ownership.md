# Jon-Style Rust Phase 1 Slice 1 (Ownership) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land 9 new ownership/lifetime/drop axioms (IDs 67–75) in `assets/rust_quality_axioms.json`, each user-reviewed and source-cited, with 3 per-batch focused-query coverage tests + 1 end-to-end demo test. Count assertion bumped 66 → 75.

**Architecture:** Unchanged from Phase 0. Pure content enrichment of an existing pipeline. No new files, no new MCP tools, no schema changes. Append entries to JSON; existing parse-once-cache layer at [src/server/tools/axioms.rs:67-87](src/server/tools/axioms.rs:67) ingests them.

**Tech Stack:** Rust 2024, `serde_json` for axiom parsing, `cargo test` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo fmt --all -- --check` for verification, `mcp__illu__axioms` and `mcp__illu__rust_preflight` for end-to-end demo.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-1-ownership-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-1-ownership-design.md)

**Existing state:** 66 axioms in `assets/rust_quality_axioms.json` (IDs `rust_quality_01_*` through `rust_quality_66_*`). The test `test_axiom_assets_have_unique_ids_and_required_fields` currently asserts `rust_quality_axiom_count == 66`. New IDs: 67–75.

**Drafts pre-approved:** The user reviewed the 9 candidate axioms during the Phase 1 brainstorming compression step before this plan was written and approved them as-is. Skip the "User reviews drafted axiom" step inside each batch task; integrate drafts verbatim. Per-batch user-review checkpoints between dispatches still happen at the controller (assistant) level.

**Per-batch tests use focused queries**, same as Phase 0 (refactored mid-Phase-0 from broad-query to per-axiom focused-query for robustness against trigger crowding). Each new entry must rank in the top `MAX_AXIOM_RESULTS = 16` for a query built from its own triggers.

**Commit policy per CLAUDE.md:** No `Co-Authored-By` trailer. Use HEREDOC for the commit message body. User identity only.

**Quality gate:** `mcp__illu__quality_gate` will likely return BLOCKED on the documented heuristic false-positive (test-scope `unwrap()` covered by module-level `#[expect(clippy::unwrap_used, reason = "tests")]` at [src/server/tools/axioms.rs:179](src/server/tools/axioms.rs:179)). This is acceptable when `cargo clippy -D warnings` exits 0.

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `assets/rust_quality_axioms.json` | Append 9 entries | New axiom data; same JSON-array shape |
| `src/server/tools/axioms.rs` | Add 4 test fns + extend 1 | Per-batch coverage tests + ownership demo test |

No other files touched.

---

## Task 1: Batch 1 — Borrows in motion (NLL, Reborrowing, References don't own)

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries before the closing `]`, after `rust_quality_66_test_failure_variant`)
- Modify: `src/server/tools/axioms.rs` (add `test_ownership_axioms_batch_1_present` inside `mod tests`; bump count assertion from 66 to 69)

- [ ] **Step 1: Confirm starting ID**

```bash
grep -oE '"id": "rust_quality_[0-9]+_' assets/rust_quality_axioms.json | grep -oE '[0-9]+' | sort -n | tail -1
```
Expected: `66`. If different, update IDs in the drafts below to start at `<max>+1` and adjust subsequent batches.

- [ ] **Step 2: Write the failing batch-1 coverage test**

Add inside `mod tests` in `src/server/tools/axioms.rs`, alongside the existing batch tests:

```rust
    #[test]
    fn test_ownership_axioms_batch_1_present() {
        let result = handle_axioms(
            "NLL non-lexical lifetimes borrow scope last use borrow flow",
        )
        .unwrap();
        assert!(
            result.contains("Borrow Scope"),
            "Borrow Scope missing in focused query"
        );

        let result = handle_axioms(
            "reborrow reborrowing &mut T compiler reborrow ergonomic API mut self chain",
        )
        .unwrap();
        assert!(
            result.contains("Reborrowing"),
            "Reborrowing missing in focused query"
        );

        let result = handle_axioms(
            "references do not own &mut not ownership consume vs borrow API confusion",
        )
        .unwrap();
        assert!(
            result.contains("Reference Semantics"),
            "Reference Semantics missing in focused query"
        );
    }
```

- [ ] **Step 3: Run the new test, expect failure**

Run: `cargo test --lib -- test_ownership_axioms_batch_1_present`
Expected: FAIL — three categories not yet in JSON.

- [ ] **Step 4: SKIP — drafts pre-approved by user**

- [ ] **Step 5: Append approved drafts to JSON**

The file is a top-level JSON array. Locate the closing `]` at the end. Insert the three drafts immediately before it, with a comma after the previous last entry (`rust_quality_66_test_failure_variant`). Use the `Edit` tool: `old_string` is the last entry's closing `}` followed by the `]`; `new_string` is the same `}` followed by `,\n  <new entries joined by commas>\n]`.

**Draft 1.1 — NLL: borrows end at last use (id: rust_quality_67_nll_borrow_scope):**

```json
{
  "id": "rust_quality_67_nll_borrow_scope",
  "category": "Borrow Scope",
  "source": "Rust for Rustaceans, ch. 1 §Lifetimes; Crust of Rust: Lifetime Annotations",
  "triggers": ["NLL", "non-lexical lifetimes", "borrow scope", "last use", "borrow flow", "borrow checker scope"],
  "rule_summary": "A borrow ends at its last use, not at the end of the curly-brace scope. When the borrow checker rejects code, restructure (move the conflicting use earlier or use the value sooner) before reaching for `.clone()` or owned types.",
  "prompt_injection": "MANDATORY RULE: When the borrow checker rejects code that conflicts with a live borrow, first ask whether the borrow's last use can move earlier. NLL means a borrow's effective scope is its live range, not its lexical block. `.clone()` is the last resort, not the first.",
  "anti_pattern": "let mut map: HashMap<String, i32> = HashMap::new();\nlet key = String::from(\"foo\");\nlet value = match map.get(&key) {\n    Some(v) => *v,\n    None => 0,\n};\n// Defensive clone: assumes map.get's borrow of key extends past the match.\nmap.insert(key.clone(), value + 1);",
  "good_pattern": "let mut map: HashMap<String, i32> = HashMap::new();\nlet key = String::from(\"foo\");\nlet value = match map.get(&key) {\n    Some(v) => *v,\n    None => 0,\n};\n// NLL: the borrow of key from map.get ends at the end of the match;\n// key can be moved into insert without cloning.\nmap.insert(key, value + 1);"
}
```

**Draft 1.2 — Reborrowing of `&mut T` (id: rust_quality_68_reborrow_mut):**

```json
{
  "id": "rust_quality_68_reborrow_mut",
  "category": "Reborrowing",
  "source": "Rust for Rustaceans, ch. 1 §Borrowing; Crust of Rust: Lifetime Annotations",
  "triggers": ["reborrow", "reborrowing", "&mut T", "compiler reborrow", "ergonomic API", "mut self chain"],
  "rule_summary": "Passing a `&mut T` to a function does not move the reference; the compiler reborrows it as `&mut *m` for the call's duration, after which the original is usable again. Lean on this rather than working around perceived double-borrows.",
  "prompt_injection": "MANDATORY RULE: When chaining calls on a `&mut T`, trust the compiler's reborrowing. `helper(buf); buf.push(x);` works because `helper` got a reborrow `&mut *buf`, not the original. Manual `std::mem::take` / `std::mem::replace` gymnastics are a code smell when reborrowing would suffice.",
  "anti_pattern": "fn process(data: &mut Vec<i32>) {\n    // Unnecessary ownership swap to avoid perceived double-borrow.\n    let mut owned = std::mem::take(data);\n    use_helper(&mut owned);\n    *data = owned;\n}\nfn use_helper(d: &mut Vec<i32>) { d.push(0); }",
  "good_pattern": "fn process(data: &mut Vec<i32>) {\n    use_helper(data); // implicit reborrow: helper gets &mut *data\n    data.push(0);     // data is usable again after the call returns\n}\nfn use_helper(d: &mut Vec<i32>) { d.push(0); }"
}
```

**Draft 1.3 — References don't own (id: rust_quality_69_references_dont_own):**

```json
{
  "id": "rust_quality_69_references_dont_own",
  "category": "Reference Semantics",
  "source": "Rust for Rustaceans, ch. 1 §Ownership and Borrowing",
  "triggers": ["references do not own", "&mut not ownership", "consume vs borrow", "API confusion", "drop reference", "take vs borrow"],
  "rule_summary": "`&mut T` is a unique borrow, not ownership. You cannot drop, replace, or destructure the borrowed value through it; you can only mutate in place. APIs that need to consume should take `T` (owned); APIs that need to mutate-and-return-control should take `&mut T`.",
  "prompt_injection": "MANDATORY RULE: Choose between `&mut T` and `T` by what the API does to the value: mutate-in-place takes `&mut T`; consume / replace / extract takes `T`. `drop(buf)` on a `buf: &mut T` only drops the reference, not the value — usually a sign the wrong signature was chosen.",
  "anti_pattern": "// Function pretends to consume but only takes a reference.\nfn finalize(buf: &mut Vec<u8>) -> Vec<u8> {\n    // Caller still owns *buf afterwards; the function cannot replace or destroy it.\n    buf.clone() // paid an allocation to return ownership we never had\n}",
  "good_pattern": "// Take T when the function consumes; the caller transfers ownership.\nfn finalize(buf: Vec<u8>) -> Vec<u8> {\n    buf // moved through; can drop, mutate, replace, return\n}\n// Take &mut T when the function mutates in place.\nfn append_one(buf: &mut Vec<u8>, byte: u8) {\n    buf.push(byte);\n}"
}
```

After insertion, validate JSON parses:
```bash
python3 -c "import json; json.load(open('assets/rust_quality_axioms.json')); print('valid')"
```

- [ ] **Step 6: Bump the count assertion in `axioms.rs`**

Find `test_axiom_assets_have_unique_ids_and_required_fields`. Its last line currently reads:
```rust
assert_eq!(rust_quality_axiom_count, 66);
```
Change `66` to `69` (3 drafts accepted).

- [ ] **Step 7: Run the batch-1 test, expect pass**

Run: `cargo test --lib -- test_ownership_axioms_batch_1_present`
Expected: PASS. **If any focused query fails to surface its target axiom, STOP and report BLOCKED with the failing query.**

- [ ] **Step 8: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 9: Lint and format checks**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
Expected: clean.

- [ ] **Step 10: Commit batch 1**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add ownership batch 1 (NLL, reborrow, references don't own)

Adds three entries from Rust for Rustaceans ch. 1 covering non-lexical
lifetimes (borrows end at last use, not at scope end), reborrowing of
&mut T (the compiler's implicit reborrow makes ergonomic chaining work),
and the distinction between &mut T as a unique borrow vs T as ownership.
EOF
)"
```

---

## Task 2: Batch 2 — Variance + Drop (Variance discipline, Drop order, Self-referential types)

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_ownership_axioms_batch_2_present`; bump count assertion from 69 to 72)

- [ ] **Step 1: Write the failing batch-2 coverage test**

Add inside `mod tests`:

```rust
    #[test]
    fn test_ownership_axioms_batch_2_present() {
        let result = handle_axioms(
            "variance covariance contravariance invariant PhantomData lifetime variance",
        )
        .unwrap();
        assert!(
            result.contains("Variance"),
            "Variance missing in focused query"
        );

        let result = handle_axioms(
            "drop order field declaration order destructor sequence struct drop",
        )
        .unwrap();
        assert!(
            result.contains("Drop Order"),
            "Drop Order missing in focused query"
        );

        let result = handle_axioms(
            "self-referential struct ouroboros self_cell pinned fields owning_ref dangling self-ref",
        )
        .unwrap();
        assert!(
            result.contains("Self-Referential Types"),
            "Self-Referential Types missing in focused query"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

Run: `cargo test --lib -- test_ownership_axioms_batch_2_present`
Expected: FAIL.

- [ ] **Step 3: SKIP — drafts pre-approved by user**

- [ ] **Step 4: Append approved drafts to JSON**

**Draft 2.1 — Variance discipline (id: rust_quality_70_variance_discipline):**

```json
{
  "id": "rust_quality_70_variance_discipline",
  "category": "Variance",
  "source": "Rust for Rustaceans, ch. 1 §Variance and Subtyping; Rustonomicon §Variance; Crust of Rust: Subtyping and Variance",
  "triggers": ["variance", "covariance", "contravariance", "invariant", "PhantomData lifetime", "variance marker"],
  "rule_summary": "Pick the `PhantomData<...>` marker that captures what your type semantically holds, because variance determines soundness. Per the Rustonomicon variance table: `&'a T` is covariant in `'a` and `T`; `&'a mut T` is covariant in `'a` and invariant in `T`; `fn(T)` is contravariant in `T`; `fn() -> T` is covariant in `T`; `*const T` is covariant in `T`; `*mut T` is invariant in `T`. `PhantomData<X>` mirrors `X`.",
  "prompt_injection": "MANDATORY RULE: When defining a struct that conceptually holds a borrow, raw pointer, or function involving a generic parameter it does not store as a field, choose the `PhantomData<...>` marker by variance, not by convenience. The marker dictates whether `'short` lifetimes can subtype `'long` and whether `T` may be substituted for a subtype, which determines whether your unsafe abstraction is sound.",
  "anti_pattern": "use std::marker::PhantomData;\nstruct Slice<'a, T> {\n    ptr: *const T,\n    len: usize,\n    _marker: PhantomData<T>, // Wrong: ignores 'a; treats T as owned, not borrowed\n}",
  "good_pattern": "use std::marker::PhantomData;\n// Behaves like &'a [T]: covariant in 'a and T.\nstruct Slice<'a, T> {\n    ptr: *const T,\n    len: usize,\n    _marker: PhantomData<&'a T>,\n}\n// Behaves like &'a mut [T]: covariant in 'a, invariant in T.\nstruct SliceMut<'a, T> {\n    ptr: *mut T,\n    len: usize,\n    _marker: PhantomData<&'a mut T>,\n}\n// Consumer-like callback: contravariant in T (input position).\nstruct Sink<T> {\n    f: fn(T),\n    _marker: PhantomData<fn(T)>,\n}"
}
```

**Draft 2.2 — Drop order matters (id: rust_quality_71_drop_order):**

```json
{
  "id": "rust_quality_71_drop_order",
  "category": "Drop Order",
  "source": "Rust for Rustaceans, ch. 1 §Drop Check; Rust Reference §Destructors",
  "triggers": ["drop order", "field declaration order", "destructor sequence", "struct drop", "drop fields", "RAII order"],
  "rule_summary": "Struct fields are dropped in declaration order (top to bottom). Design the field order so that fields holding borrows or callbacks into other fields are declared *before* the fields they depend on — dependents drop first, dependencies drop last.",
  "prompt_injection": "MANDATORY RULE: Order struct fields by drop dependency: a field that uses another field in its `Drop` impl must be declared first. If `logger.flush()` writes to `writer`, declare `logger` before `writer` so logger's destructor runs while writer is still alive.",
  "anti_pattern": "// writer dropped first, but logger's Drop tries to flush via writer — undefined access.\nstruct LogWriter {\n    writer: BufWriter<File>, // declared first → dropped first\n    logger: FlushOnDropLogger,\n}",
  "good_pattern": "// logger dropped first while writer is still alive; writer drops last and flushes.\nstruct LogWriter {\n    logger: FlushOnDropLogger, // declared first → dropped first\n    writer: BufWriter<File>,   // declared second → dropped last\n}"
}
```

**Draft 2.3 — Self-referential types need help (id: rust_quality_72_self_referential):**

```json
{
  "id": "rust_quality_72_self_referential",
  "category": "Self-Referential Types",
  "source": "Rust for Rustaceans, ch. 1 §Self-Referential Types; Crust of Rust: Pin",
  "triggers": ["self-referential", "self-ref struct", "ouroboros", "self_cell", "pinned fields", "owning_ref", "dangling self-ref"],
  "rule_summary": "Naive self-referential structs (`struct { data: String, view: &str_into_data }`) do not compile, and faking `'static` is unsound the moment `data` moves. Use indices into the owning data, an arena (`bumpalo`, `typed-arena`), or a Pin-based crate (`ouroboros`, `self_cell`) instead.",
  "prompt_injection": "MANDATORY RULE: If your struct conceptually holds a reference into a field of itself, the design is wrong before any code compiles. Replace the inner reference with an index, restructure ownership through an arena, or pull in `ouroboros`/`self_cell`. Never `'static`-cast the inner reference; moving the outer struct will dangle it.",
  "anti_pattern": "// 'static is a lie. If the Doc moves, every &'static str dangles.\nstruct Doc {\n    text: String,\n    parsed: Vec<&'static str>,\n}",
  "good_pattern": "use std::ops::Range;\n// Index into the owning data; safe under any move of Doc.\nstruct Doc {\n    text: String,\n    spans: Vec<Range<usize>>,\n}\nimpl Doc {\n    fn segment(&self, i: usize) -> &str { &self.text[self.spans[i].clone()] }\n}\n// Alternative when zero-copy borrowing is essential: ouroboros / self_cell."
}
```

After insertion, validate JSON parses.

- [ ] **Step 5: Bump the count assertion**

Update `assert_eq!(rust_quality_axiom_count, 69);` to `assert_eq!(rust_quality_axiom_count, 72);`.

- [ ] **Step 6: Run the batch-2 test, expect pass**

Run: `cargo test --lib -- test_ownership_axioms_batch_2_present`
Expected: PASS.

- [ ] **Step 7: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 8: Lint and format**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 9: Commit batch 2**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add ownership batch 2 (variance, drop order, self-referential types)

Adds three entries on the variance contract enforced by PhantomData<...>
choice, struct field declaration order determining destructor sequence,
and the canonical alternatives to naive self-referential structs (indices,
arenas, ouroboros / self_cell) instead of unsound 'static casts.
EOF
)"
```

---

## Task 3: Batch 3 — Interior mutability + async-aware ownership (IM decision tree, MutexGuard/.await, Pin/Unpin) + demo test

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_ownership_axioms_batch_3_present` and `test_ownership_demo_query_returns_new_axioms`; bump count assertion from 72 to 75)

- [ ] **Step 1: Write the failing batch-3 coverage test**

Add inside `mod tests`:

```rust
    #[test]
    fn test_ownership_axioms_batch_3_present() {
        let result = handle_axioms(
            "Cell RefCell atomic Mutex RwLock interior mutability decision tree thread shared",
        )
        .unwrap();
        assert!(
            result.contains("Interior Mutability Selection"),
            "Interior Mutability Selection missing in focused query"
        );

        let result = handle_axioms(
            "MutexGuard await deadlock async lock std sync Mutex tokio sync Mutex",
        )
        .unwrap();
        assert!(
            result.contains("Async Lock Hygiene"),
            "Async Lock Hygiene missing in focused query"
        );

        let result = handle_axioms(
            "Pin Unpin self-pin self-referential future poll Pin<&mut Self> unpin auto trait",
        )
        .unwrap();
        assert!(
            result.contains("Pin Discipline"),
            "Pin Discipline missing in focused query"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

Run: `cargo test --lib -- test_ownership_axioms_batch_3_present`
Expected: FAIL.

- [ ] **Step 3: SKIP — drafts pre-approved by user**

- [ ] **Step 4: Append approved drafts to JSON**

**Draft 3.1 — Interior mutability decision tree (id: rust_quality_73_interior_mutability_selection):**

```json
{
  "id": "rust_quality_73_interior_mutability_selection",
  "category": "Interior Mutability Selection",
  "source": "Rust for Rustaceans, ch. 1 §Interior Mutability; Crust of Rust: Smart Pointers",
  "triggers": ["Cell RefCell atomic Mutex RwLock", "interior mutability decision tree", "thread shared", "lock-free", "single-threaded mutation", "IM primitive"],
  "rule_summary": "Pick the lightest interior-mutability primitive that satisfies the sharing need: `Cell<T>` (single-threaded, `Copy` or whole-value get/set, no runtime borrow check), `RefCell<T>` (single-threaded, compound types, runtime borrow check), atomics (`AtomicU64` etc., thread-shared primitives, lock-free), `Mutex<T>` / `RwLock<T>` (thread-shared compound types, blocking).",
  "prompt_injection": "MANDATORY RULE: Do not reach for `Mutex<T>` reflexively when the sharing pattern is single-threaded or primitive. `Cell<T>` and `RefCell<T>` are single-thread and free of locking; atomics are thread-shared and lock-free; `Mutex`/`RwLock` are thread-shared and lock-bearing. Picking a heavier primitive than required is a wasted-cost smell.",
  "anti_pattern": "// Single-threaded counter wrapped in Mutex — overkill, lock-bearing, panic-prone.\nstruct Counter { inner: std::sync::Mutex<u64> }\nimpl Counter {\n    fn increment(&self) { *self.inner.lock().unwrap() += 1; }\n}",
  "good_pattern": "use std::sync::atomic::{AtomicU64, Ordering};\n// Thread-shared primitive: atomic, lock-free.\nstruct Counter { inner: AtomicU64 }\nimpl Counter {\n    fn increment(&self) { self.inner.fetch_add(1, Ordering::Relaxed); }\n}\n// Single-thread Copy alternative: Cell<u64> (no atomics needed).\nstruct LocalCounter { inner: std::cell::Cell<u64> }\nimpl LocalCounter {\n    fn increment(&self) { self.inner.set(self.inner.get() + 1); }\n}"
}
```

**Draft 3.2 — `MutexGuard` across `.await` is a deadlock (id: rust_quality_74_mutex_guard_await):**

```json
{
  "id": "rust_quality_74_mutex_guard_await",
  "category": "Async Lock Hygiene",
  "source": "Rust for Rustaceans, ch. 5 §Async; Tokio docs §Shared State; Crust of Rust: Channels",
  "triggers": ["MutexGuard await deadlock", "async lock", "std sync Mutex async", "tokio sync Mutex", "lock across await", "blocking lock async"],
  "rule_summary": "A `std::sync::MutexGuard` held across an `.await` point is a deadlock or correctness hazard: the executor may move the future to another thread, yet the guard's `Send` bounds and the lock's thread-affinity will conflict. Drop the guard before awaiting, or use `tokio::sync::Mutex` (whose guard is `Send` and integrates with the runtime).",
  "prompt_injection": "MANDATORY RULE: Never hold a `std::sync::Mutex` (or `RwLock`) guard across an `.await`. Either narrow the locked region so the guard drops before the await, or replace the lock with `tokio::sync::Mutex` for genuine async-aware locking. The compiler will not always catch this; write the code so the guard's scope is local.",
  "anti_pattern": "use std::sync::Mutex;\nasync fn process(state: &Mutex<State>) -> Result<()> {\n    let guard = state.lock().unwrap();\n    fetch_data().await?; // guard is still held; future may move; deadlock risk\n    apply_change(&*guard);\n    Ok(())\n}",
  "good_pattern": "use std::sync::Mutex;\nasync fn process(state: &Mutex<State>) -> Result<()> {\n    // Take what we need from the lock, then drop the guard before awaiting.\n    let snapshot = {\n        let guard = state.lock().unwrap();\n        guard.snapshot()\n    };\n    let data = fetch_data().await?;\n    {\n        let mut guard = state.lock().unwrap();\n        guard.update_from(&snapshot, &data);\n    }\n    Ok(())\n}\n// If the lock genuinely must span an await, switch to tokio::sync::Mutex."
}
```

**Draft 3.3 — Pin / Unpin (id: rust_quality_75_pin_discipline):**

```json
{
  "id": "rust_quality_75_pin_discipline",
  "category": "Pin Discipline",
  "source": "Rust for Rustaceans, ch. 1 §Pinning and ch. 5 §Async; Crust of Rust: Pin",
  "triggers": ["Pin Unpin", "self-pin", "self-referential future", "Pin<&mut Self>", "unpin auto trait", "pin projection"],
  "rule_summary": "Most types are `Unpin` (the auto trait); you only need to think about `Pin` when designing a type that is self-referential or implementing `Future::poll` directly. `Pin<&mut Self>` in `poll` means the future's memory address is stable so any internal self-references stay valid; respect that contract or use `pin_project`.",
  "prompt_injection": "MANDATORY RULE: When implementing `Future::poll` directly or designing a self-referential type, take the `Pin<&mut Self>` contract seriously: do not move out of `self`, do not return a `&mut` to a self-referential field without `pin_project`, and do not assume the type is `Unpin` unless you have an explicit `impl Unpin for ...` with documented soundness. For everything else, normal `&mut self` methods are fine because most types are auto-`Unpin`.",
  "anti_pattern": "use std::future::Future;\nuse std::pin::Pin;\nuse std::task::{Context, Poll};\nstruct MyFuture { /* ... */ }\nimpl Future for MyFuture {\n    type Output = ();\n    // Wrong signature: Future::poll takes Pin<&mut Self>, not &mut Self.\n    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<()> { Poll::Ready(()) }\n}",
  "good_pattern": "use std::future::Future;\nuse std::pin::Pin;\nuse std::task::{Context, Poll};\nstruct MyFuture { state: u32 }\nimpl Future for MyFuture {\n    type Output = ();\n    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {\n        // MyFuture has no self-references, so Pin is just a marker here.\n        // For self-referential futures use pin-project to get safe field access.\n        Poll::Ready(())\n    }\n}"
}
```

After insertion, validate JSON parses.

- [ ] **Step 5: Bump the count assertion**

Update `assert_eq!(rust_quality_axiom_count, 72);` to `assert_eq!(rust_quality_axiom_count, 75);`.

- [ ] **Step 6: Run the batch-3 test, expect pass**

Run: `cargo test --lib -- test_ownership_axioms_batch_3_present`
Expected: PASS.

- [ ] **Step 7: Add the ownership demo-query test**

Add inside `mod tests`:

```rust
    #[test]
    fn test_ownership_demo_query_returns_new_axioms() {
        let result =
            handle_axioms("ownership borrow lifetime variance drop pin reborrow interior mutability")
                .unwrap();
        // Expect at least 3 of the 9 new ownership categories to surface in the top results.
        let new_categories = [
            "Borrow Scope",
            "Reborrowing",
            "Reference Semantics",
            "Variance",
            "Drop Order",
            "Self-Referential Types",
            "Interior Mutability Selection",
            "Async Lock Hygiene",
            "Pin Discipline",
        ];
        let surfaced = new_categories
            .iter()
            .filter(|cat| result.contains(*cat))
            .count();
        assert!(
            surfaced >= 3,
            "expected at least 3 new ownership categories in demo query, got {surfaced}"
        );
    }
```

- [ ] **Step 8: Run the demo test, expect pass**

Run: `cargo test --lib -- test_ownership_demo_query_returns_new_axioms`
Expected: PASS. **If FAIL with `surfaced < 3`, STOP and report BLOCKED with the actual count and which categories did surface.**

- [ ] **Step 9: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass, including all three ownership batch tests and the ownership demo test.

- [ ] **Step 10: Lint and format**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 11: Commit batch 3**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add ownership batch 3 (interior mutability, MutexGuard/await, Pin) + demo test

Adds the final three ownership entries: an interior-mutability decision tree
(Cell -> RefCell -> atomic -> Mutex/RwLock), the MutexGuard-across-await
deadlock rule (drop before await, or use tokio::sync::Mutex), and the Pin /
Unpin contract for self-referential types and hand-rolled futures. Adds an
end-to-end demo test asserting >= 3 of the 9 new categories surface for the
canonical broad ownership query.
EOF
)"
```

---

## Task 4: End-to-End Verification and Demo

**Files:** none modified directly.

- [ ] **Step 1: Full cargo verification**

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
Expected: every command exits 0. (Phase 0 closed with disk-space pressure on `cargo build`; user has since cleaned up. If `cargo build` fails on disk space again, note it and skip — `cargo test` is the authoritative compile.)

- [ ] **Step 2: MCP-level demo query**

Call the running illu MCP server with a canonical ownership query:
```
mcp__illu__axioms(query: "ownership borrow lifetime variance drop pin reborrow interior mutability")
```
Expected: response markdown contains at least 3 of the 9 new category names. If FAIL, the running server may be stale (binary not rebuilt yet) — note and proceed.

- [ ] **Step 3: rust_preflight evidence packet check**

```
mcp__illu__rust_preflight(task: "Design a struct holding a borrowed slice and a mutable cache, with correct variance and drop order", baseline_query: "planning data structures documentation comments idiomatic rust verification performance ownership lifetime")
```
Expected: `axioms` section includes at least one new ownership category (most likely `Variance`, `Drop Order`, or `Self-Referential Types` given the task content).

- [ ] **Step 4: Sample function exercise (optional but valuable)**

Author a short snippet exercising the new axioms in the same chat session: e.g., a struct with non-trivial ownership and drop semantics, designed via `rust_preflight`. The result need not be committed; the goal is qualitative confirmation that `rust_preflight`'s evidence packet steers design toward the new rules.

- [ ] **Step 5: No additional commit needed.**

---

## Verification Summary

After all tasks complete:
- 9 new axioms in `assets/rust_quality_axioms.json` (or fewer if some rejected).
- 4 new tests in `src/server/tools/axioms.rs`: 3 batch coverage + 1 ownership demo.
- All cargo checks clean.
- MCP-level query returns the new ownership categories (assuming a server-side rebuild after merge).
- `rust_preflight` evidence packet includes them.

## Risks Realized During Execution

- **JSON parse error after append.** Most likely a missing or trailing comma between entries. `python3 -c "import json; json.load(open('assets/rust_quality_axioms.json'))"` gives line:column.
- **Batch test fails on focused query.** Inspect via `mcp__illu__axioms` with the specific failing query. Likely a trigger that doesn't substring-match because of casing or hyphenation. Tighten triggers (more specific words) or sharpen the category name.
- **Drop Order batch test fails because `"Drop Order"` collides** with the existing `[RAII]` axiom category. Both are about resource cleanup but different framing. Mitigation pre-checked: existing axioms do not use `"Drop Order"` as category or trigger; should be safe.
- **Pin Discipline test fails because the existing Unsafe Rust axiom mentions `Drop`/`forget`/`ManuallyDrop`** in its body. The new test asserts `"Pin Discipline"` (a unique category name); the `result.contains(...)` substring check is precise to the category header rendered as `### [Pin Discipline]`, so it cannot false-positive against other axioms' bodies.
- **`MutexGuard` axiom (id 74) overlaps with the existing `[Concurrency]` axiom** that says "Use `Arc<Mutex<T>>`...". The new rule sharpens it for async; categories are distinct (`Async Lock Hygiene` vs `Concurrency`); should not collide.
- **Interior Mutability Selection (id 73) overlaps with existing `[Interior Mutability]` axiom.** The new entry is a decision-tree refinement; categories differ (`Interior Mutability Selection` vs `Interior Mutability`). Both can co-exist; the new one wins for "decision tree" / "selection" queries.
- **Variance (id 70) overlaps with existing `[Types]` PhantomData axiom.** The existing axiom covers `PhantomData<T>` for FFI/raw-pointer marker semantics; the new axiom covers variance choice. Distinct categories; complementary.
- **Reference Semantics (id 69) overlaps with existing `[Borrowing]` axioms (mutable XOR, mutually exclusive reference framing).** The new axiom is at the level of "what does `&mut T` semantically mean for API design" — which-type-to-take. Distinct angle.
