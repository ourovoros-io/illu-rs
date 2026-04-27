# Jon-Style Rust Phase 1 Slice 3 (Performance) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Land 9 new performance/codegen axioms (IDs 85–93) in `assets/rust_quality_axioms.json`, with 3 per-batch coverage tests + 1 demo test. Count assertion bumped 84 → 93.

**Architecture:** Unchanged from prior phases. Pure content enrichment.

**Tech Stack:** Rust 2024, `serde_json`, `cargo test` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo fmt --all -- --check`.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-1-performance-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-1-performance-design.md)

**Existing state:** 84 axioms (Phase 0 + Slice 1 + Slice 2). Test asserts `rust_quality_axiom_count == 84`. New IDs: 85–93.

**Drafts pre-approved by user.** Skip the user-review step inside batch tasks.

**Verification discipline.** Performance claims are quantitative; the spec mandates each axiom cite a Godbolt link, `cargo bench` / iai-callgrind output, `cargo llvm-lines` / `cargo bloat`, or authoritative documentation in the `source` field. Drafts use placeholder citations to the Rust Performance Book and *Rust for Rustaceans* ch. 9 — tighten during review if exact links are at hand.

**Per-batch tests use focused queries** (Phase 0 / Slice 1 / Slice 2 standard).

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `assets/rust_quality_axioms.json` | Append 9 entries | New axiom data |
| `src/server/tools/axioms.rs` | Add 4 test fns + extend 1 | Per-batch coverage tests + perf demo test |

---

## Task 1: Batch 1 — Allocation discipline (hot paths, string allocation, iterator codegen)

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries before the closing `]`, after `rust_quality_84_marker_auto_traits`)
- Modify: `src/server/tools/axioms.rs` (add `test_perf_axioms_batch_1_present`; bump count assertion 84 → 87)

- [ ] **Step 1: Confirm starting ID**

```bash
grep -oE '"id": "rust_quality_[0-9]+_' assets/rust_quality_axioms.json | grep -oE '[0-9]+' | sort -n | tail -1
```
Expected: `84`. If different, bump IDs accordingly.

- [ ] **Step 2: Write the failing batch-1 coverage test**

```rust
    #[test]
    fn test_perf_axioms_batch_1_present() {
        let result = handle_axioms(
            "allocation hot path Vec with_capacity format! in loop buffer reuse preallocate",
        )
        .unwrap();
        assert!(
            result.contains("Allocation Discipline"),
            "Allocation Discipline missing in focused query"
        );

        let result = handle_axioms(
            "Cow str Box<str> static str string with_capacity string memory allocation strategy",
        )
        .unwrap();
        assert!(
            result.contains("String Allocation"),
            "String Allocation missing in focused query"
        );

        let result = handle_axioms(
            "iter elide bounds check indexed access bounds check iterator vectorize for i in 0..n compiler bounds proof",
        )
        .unwrap();
        assert!(
            result.contains("Iterator Codegen"),
            "Iterator Codegen missing in focused query"
        );
    }
```

- [ ] **Step 3: Run the test, expect failure**

Run: `cargo test --lib -- test_perf_axioms_batch_1_present`. Expected: FAIL.

- [ ] **Step 4: SKIP — drafts pre-approved**

- [ ] **Step 5: Append approved drafts to JSON**

**Draft 1.1 — Allocation in hot paths (id: rust_quality_85_allocation_hot_paths):**

```json
{
  "id": "rust_quality_85_allocation_hot_paths",
  "category": "Allocation Discipline",
  "source": "Rust for Rustaceans, ch. 9 §Allocation; Rust Performance Book §Heap Allocations",
  "triggers": ["allocation hot path", "Vec with_capacity", "format! in loop", "buffer reuse", "preallocate", "avoid alloc"],
  "rule_summary": "In hot paths, preallocate `Vec` and `String` with `with_capacity` for known sizes, avoid `format!`/`to_string` calls inside tight loops, and reuse buffers with `.clear()` rather than reallocating. Each `format!` allocates a fresh `String`; `write!` into a preallocated buffer with `std::fmt::Write` does not.",
  "prompt_injection": "MANDATORY RULE: When a function runs in a hot path (per profile or by design), preallocate output buffers with `with_capacity`, route formatted output through `write!` against an existing buffer rather than `format!`, and reuse rather than recreate buffers across iterations. Each unjustified allocation in a hot loop is a measurable cost.",
  "anti_pattern": "fn render_lines(items: &[Item]) -> String {\n    let mut out = String::new(); // 0 capacity — reallocates as it grows\n    for item in items {\n        out.push_str(&format!(\"{}\\n\", item)); // format! allocates a fresh String each iteration\n    }\n    out\n}",
  "good_pattern": "use std::fmt::Write;\nfn render_lines(items: &[Item]) -> String {\n    let mut out = String::with_capacity(items.len() * 32); // preallocated estimate\n    for item in items {\n        let _ = write!(out, \"{}\\n\", item); // writes into the existing buffer; no per-iteration alloc\n    }\n    out\n}"
}
```

**Draft 1.2 — String allocation strategy (id: rust_quality_86_string_allocation):**

```json
{
  "id": "rust_quality_86_string_allocation",
  "category": "String Allocation",
  "source": "Rust for Rustaceans, ch. 1 §Cow; Rust Performance Book §Type Sizes; std docs §Box<str>",
  "triggers": ["Cow str", "Box<str>", "static str", "string with_capacity", "string memory", "allocation strategy"],
  "rule_summary": "Pick the right string type by use case: `&'static str` for compile-time strings (no allocation), `&str` for borrowed slices, `Box<str>` for owned-but-not-growable (drops the capacity field — saves ~8 bytes per string vs `String`), `String` for owned+growable, `Cow<'a, str>` for sometimes-borrowed-sometimes-owned. Defaulting everything to `String` allocates where you don't need to.",
  "prompt_injection": "MANDATORY RULE: Choose the string type that matches the value's actual lifecycle. `String` is owned and growable — use it only when you need both. For struct fields known at compile time, use `&'static str`. For owned-but-frozen, use `Box<str>`. For sometimes-borrowed, use `Cow<'a, str>`. Excess `String` allocations compound across hot paths.",
  "anti_pattern": "// Every field is String even though some are static or rarely modified.\nstruct Config {\n    name: String,        // even when name is always a built-in default like \"timeout\"\n    description: String, // even when often borrowed from a config file slice\n}\nlet _ = Config {\n    name: \"timeout\".to_owned(),       // unnecessary heap alloc\n    description: \"...\".to_owned(),\n};",
  "good_pattern": "use std::borrow::Cow;\nstruct Config<'a> {\n    name: &'static str,        // no allocation for built-in defaults\n    description: Cow<'a, str>, // borrows when possible, owns when needed\n}\n// Owned-but-frozen field: Box<str> drops the unused capacity field.\nstruct Frozen { label: Box<str> }"
}
```

**Draft 1.3 — Iterator semantics over indexed access (id: rust_quality_87_iterator_codegen):**

```json
{
  "id": "rust_quality_87_iterator_codegen",
  "category": "Iterator Codegen",
  "source": "Rust Performance Book §Bounds Checks; Crust of Rust: Iterators",
  "triggers": ["iter elide bounds check", "indexed access bounds check", "iterator vectorize", "for i in 0..n", "compiler bounds proof"],
  "rule_summary": "Use `iter()`/`iter_mut()` over indexed loops where possible. Iterators carry the structural guarantee that each element is touched exactly once, which lets the compiler reliably elide bounds checks, autovectorize, and inline aggressively. Indexed access (`arr[i]` in `for i in 0..arr.len()`) can also be optimized but the elision is conditional on the compiler proving `i < len`, which it doesn't always do for non-trivial loops.",
  "prompt_injection": "MANDATORY RULE: Default to `iter()`/`iter_mut()`/`iter_each()` for elementwise traversal, not indexed loops. The iterator form gives the compiler a structural proof of safety; the indexed form requires the compiler to derive that proof from the loop bounds. Reach for indexed access only when you genuinely need an index alongside the value (use `.enumerate()` instead in many cases).",
  "anti_pattern": "fn sum_indices(arr: &[i64]) -> i64 {\n    let mut total = 0;\n    for i in 0..arr.len() {     // each `arr[i]` may emit a bounds check\n        total += arr[i];\n    }\n    total\n}",
  "good_pattern": "fn sum_iter(arr: &[i64]) -> i64 {\n    arr.iter().copied().sum() // bounds-check-free; autovectorizes on most targets\n}\nfn first_negative_index(arr: &[i64]) -> Option<usize> {\n    arr.iter().position(|&x| x < 0) // structural proof carried by the iterator\n}"
}
```

After insertion, validate JSON: `python3 -c "import json; json.load(open('assets/rust_quality_axioms.json')); print('valid')"`.

- [ ] **Step 6: Bump count assertion 84 → 87**

In `src/server/tools/axioms.rs`, change `assert_eq!(rust_quality_axiom_count, 84);` to `assert_eq!(rust_quality_axiom_count, 87);`.

- [ ] **Step 7: Run the batch-1 test, expect pass**

Run: `cargo test --lib -- test_perf_axioms_batch_1_present`. Expected: PASS.

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
axioms: add perf batch 1 (allocation hot paths, string strategy, iterator codegen)

Adds three entries on allocation-driven performance: preallocate buffers
in hot paths and route formatted output through write! rather than format!;
pick the right string type by lifecycle (static/borrow/Box<str>/String/Cow);
prefer iter() over indexed loops so the compiler reliably elides bounds
checks and autovectorizes.
EOF
)"
```

---

## Task 2: Batch 2 — Dispatch and codegen (monomorphization, inlining, enum dispatch)

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_perf_axioms_batch_2_present`; bump count assertion 87 → 90)

- [ ] **Step 1: Write the failing batch-2 coverage test**

```rust
    #[test]
    fn test_perf_axioms_batch_2_present() {
        let result = handle_axioms(
            "monomorphization code bloat binary size generic instantiation cargo llvm-lines",
        )
        .unwrap();
        assert!(
            result.contains("Monomorphization Cost"),
            "Monomorphization Cost missing in focused query"
        );

        let result = handle_axioms(
            "#[inline] inline always inline never cross-crate inline force inline",
        )
        .unwrap();
        assert!(
            result.contains("Inline Hints"),
            "Inline Hints missing in focused query"
        );

        let result = handle_axioms(
            "enum dispatch closed set dispatch enum vs dyn no vtable static heterogeneous",
        )
        .unwrap();
        assert!(
            result.contains("Enum Dispatch"),
            "Enum Dispatch missing in focused query"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

- [ ] **Step 3: SKIP — drafts pre-approved**

- [ ] **Step 4: Append approved drafts to JSON**

**Draft 2.1 — Monomorphization budget (id: rust_quality_88_monomorphization_cost):**

```json
{
  "id": "rust_quality_88_monomorphization_cost",
  "category": "Monomorphization Cost",
  "source": "Rust for Rustaceans, ch. 9 §Compile Times; Rust Performance Book §Type Sizes; cargo llvm-lines documentation",
  "triggers": ["monomorphization", "code bloat", "binary size", "generic instantiation", "cargo llvm-lines", "cargo bloat"],
  "rule_summary": "Each concrete instantiation of a generic function generates a separate copy in the binary. Hot path: generics win (inline, no indirection). Cold or rarely-used code with many instantiations: `dyn Trait` wins (single code copy, smaller binary, slightly slower call). Profile binary size with `cargo llvm-lines` or `cargo bloat` before introducing generics in cold code, and pair this rule with axiom 79 (when to switch to dyn).",
  "prompt_injection": "MANDATORY RULE: Generics are not free — each new concrete `T` duplicates the function body. For cold or many-typed call sites, accept the indirect call cost of `dyn Trait` to keep binary size bounded. Profile, don't guess: `cargo llvm-lines` and `cargo bloat` show actual binary impact per generic.",
  "anti_pattern": "// Library exports a heavily-generic decoder used in mostly-cold code paths.\n// Every concrete `T: Read` instantiates the entire decode tree separately.\npub fn decode_from<T: Read>(reader: T) -> Result<Doc, Error> { /* large impl */ }",
  "good_pattern": "// Hot path: keep generics for the inner decoders that benefit from inlining.\npub fn decode_hot<T: Read>(reader: T) -> Result<Token, Error> { /* small, inlined */ }\n// Cold entry point: take &mut dyn Read, single code copy regardless of source.\npub fn decode_from(reader: &mut dyn Read) -> Result<Doc, Error> {\n    /* large impl, monomorphized once */\n}"
}
```

**Draft 2.2 — Inlining hints (id: rust_quality_89_inline_hints):**

```json
{
  "id": "rust_quality_89_inline_hints",
  "category": "Inline Hints",
  "source": "Rust Reference §inline attribute; Rust Performance Book §Inlining",
  "triggers": ["#[inline]", "inline always", "inline never", "cross-crate inline", "force inline"],
  "rule_summary": "`#[inline]` *suggests* the compiler inline this function across crate boundaries (within a crate, the compiler decides regardless). `#[inline(always)]` *forces* inlining even where the compiler would refuse — rarely the right call, can hurt I-cache for large functions. `#[inline(never)]` disables inlining; useful for debugging codegen, profile attribution, or preserving symbols in stack traces.",
  "prompt_injection": "MANDATORY RULE: Use `#[inline]` on small, hot, cross-crate-callable functions. Reserve `#[inline(always)]` for functions you have measured to need it, not as a generic perf incantation. Use `#[inline(never)]` deliberately for debugging or to keep a function present in profiles and stack traces.",
  "anti_pattern": "// 200-line function annotated to \"make it fast\" — bloats every call site.\n// Likely net-negative: I-cache pressure outweighs the saved call overhead.\n#[inline(always)]\nfn render_complex_thing(/* ... */) -> Output { /* 200 lines */ }",
  "good_pattern": "// Tiny accessor in a hot path: cross-crate inline is a clear win.\n#[inline]\npub fn count(&self) -> usize { self.items.len() }\n\n// Debugging codegen: keep this function distinct in profiles and stack traces.\n#[inline(never)]\nfn cold_path_for_diagnostics() { /* ... */ }"
}
```

**Draft 2.3 — Closed-set enum dispatch over `dyn Trait` (id: rust_quality_90_enum_dispatch):**

```json
{
  "id": "rust_quality_90_enum_dispatch",
  "category": "Enum Dispatch",
  "source": "Rust for Rustaceans, ch. 2 §Trait Objects; enum_dispatch crate; Crust of Rust: Trait Objects",
  "triggers": ["enum dispatch", "closed set dispatch", "enum vs dyn", "no vtable", "static heterogeneous"],
  "rule_summary": "When the variant set is fixed at the library boundary (you control all impls), an enum dispatches faster than `Box<dyn Trait>`: no vtable, no indirect branch, the `match` compiles to a jump table the optimizer can specialize per arm. Use `dyn Trait` only when external code must add new variants. Caveat: when variants vary wildly in size, `Vec<MyEnum>` reserves space for the largest variant per element, so `Vec<Box<dyn Trait>>` (16-byte fat pointers) may use less memory — measure both for size-sensitive workloads.",
  "prompt_injection": "MANDATORY RULE: Reach for `dyn Trait` only when external crates need to add their own implementors. For closed-set polymorphism (your library knows all variants), define an enum and dispatch via `match`. The enum form is faster (no indirect call), more inspectable (exhaustive matches surface missed variants at compile time), and often clearer to read.",
  "anti_pattern": "trait Shape { fn area(&self) -> f64; }\nstruct Circle(f64);\nstruct Square(f64);\nimpl Shape for Circle { fn area(&self) -> f64 { std::f64::consts::PI * self.0 * self.0 } }\nimpl Shape for Square { fn area(&self) -> f64 { self.0 * self.0 } }\n// vtable per element, indirect call per .area(); external crates can add Shape impls.\nlet shapes: Vec<Box<dyn Shape>> = vec![Box::new(Circle(1.0)), Box::new(Square(2.0))];",
  "good_pattern": "enum Shape {\n    Circle(f64),\n    Square(f64),\n}\nimpl Shape {\n    fn area(&self) -> f64 {\n        match self {\n            Self::Circle(r) => std::f64::consts::PI * r * r,\n            Self::Square(s) => s * s,\n        }\n    }\n}\n// No vtable, direct call per .area(). External crates cannot add new variants —\n// that is the design choice; document it.\nlet shapes: Vec<Shape> = vec![Shape::Circle(1.0), Shape::Square(2.0)];"
}
```

After insertion, validate JSON parses.

- [ ] **Step 5: Bump count assertion 87 → 90**

- [ ] **Step 6: Run the batch-2 test, expect pass**

- [ ] **Step 7: Full test suite + lint + fmt**

- [ ] **Step 8: Commit batch 2**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add perf batch 2 (monomorphization budget, inlining hints, enum dispatch)

Adds three entries on dispatch and codegen tradeoffs: monomorphization
duplicates code per concrete T (profile with cargo llvm-lines / cargo bloat;
switch cold paths to dyn for binary-size budget); the three #[inline] hints
and when each is appropriate; enum dispatch as the closed-set alternative
to dyn (no vtable, no indirect call, with a size caveat for wide-variant
enums in Vec).
EOF
)"
```

---

## Task 3: Batch 3 — Memory and atomics (struct layout, atomic ordering, Box discipline) + demo test

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_perf_axioms_batch_3_present` and `test_perf_demo_query_returns_new_axioms`; bump count assertion 90 → 93)

- [ ] **Step 1: Write the failing batch-3 coverage test**

```rust
    #[test]
    fn test_perf_axioms_batch_3_present() {
        let result = handle_axioms(
            "struct layout field reorder padding alignment repr(C) size_of cache line",
        )
        .unwrap();
        assert!(
            result.contains("Struct Layout"),
            "Struct Layout missing in focused query"
        );

        let result = handle_axioms(
            "atomic ordering Relaxed Acquire Release SeqCst memory ordering atomic synchronization",
        )
        .unwrap();
        assert!(
            result.contains("Atomic Ordering"),
            "Atomic Ordering missing in focused query"
        );

        let result = handle_axioms(
            "Box small T stack vs heap unnecessary Box Box Copy type heap indirection",
        )
        .unwrap();
        assert!(
            result.contains("Heap Allocation Discipline"),
            "Heap Allocation Discipline missing in focused query"
        );
    }
```

- [ ] **Step 2: Run, expect fail**

- [ ] **Step 3: SKIP — drafts pre-approved**

- [ ] **Step 4: Append approved drafts**

**Draft 3.1 — Struct field layout for size (id: rust_quality_91_struct_layout):**

```json
{
  "id": "rust_quality_91_struct_layout",
  "category": "Struct Layout",
  "source": "Rust Reference §Type Layout; Rust Performance Book §Type Sizes",
  "triggers": ["struct layout", "field reorder", "padding", "alignment", "repr(C)", "size_of", "cache line"],
  "rule_summary": "Rust's default `repr(Rust)` reorders fields automatically to minimize padding. `#[repr(C)]` (FFI, stable layout) freezes declaration order — manually order fields by *descending* alignment to avoid padding gaps. For hot-path structs accessed by separate cores, lay out with cache-line awareness (64-byte boundary on most x86-64) to avoid false sharing.",
  "prompt_injection": "MANDATORY RULE: Trust `repr(Rust)` to minimize padding by default. When you must use `#[repr(C)]` (FFI, stable layout, hand-tuned layout), order fields by descending alignment so smaller fields fill the trailing space without inserting padding. For shared cache-line state, separate hot fields onto distinct lines; `crossbeam-utils::CachePadded<T>` is the standard helper.",
  "anti_pattern": "#[repr(C)]\nstruct Bad {\n    a: u8,   // 1 byte + 7 padding before the next u64\n    b: u64,  // 8 bytes\n    c: u8,   // 1 byte + 7 trailing padding to keep struct alignment\n}\n// size_of::<Bad>() == 24",
  "good_pattern": "#[repr(C)]\nstruct Better {\n    b: u64,  // 8 bytes (largest first)\n    a: u8,   // 1 byte\n    c: u8,   // 1 byte + 6 trailing padding\n}\n// size_of::<Better>() == 16\n\n// Or: drop #[repr(C)] entirely and let the compiler reorder — `repr(Rust)` does\n// the equivalent reorder for you and may pick an even tighter layout.\nstruct AutoLayout { a: u8, b: u64, c: u8 }"
}
```

**Draft 3.2 — Atomic ordering selection (id: rust_quality_92_atomic_ordering):**

```json
{
  "id": "rust_quality_92_atomic_ordering",
  "category": "Atomic Ordering",
  "source": "Rust Reference §Atomics; Rust Performance Book §Atomics; Crust of Rust: Atomics",
  "triggers": ["atomic ordering", "Relaxed", "Acquire", "Release", "SeqCst", "memory ordering"],
  "rule_summary": "Pick the weakest ordering that satisfies your synchronization need. `Relaxed`: atomicity only, no ordering between threads (counters, stats). `Acquire` (loads) / `Release` (stores): synchronizes-with the paired op (one-flag-protects-data patterns). `AcqRel`: read-modify-write that does both. `SeqCst`: total global order across all SeqCst ops — strongest, slowest, expensive on weak-memory architectures.",
  "prompt_injection": "MANDATORY RULE: `SeqCst` is the safest default but rarely the right one. Identify what synchronizes-with what: `Relaxed` for counters with no read-after-write across threads; `Acquire`/`Release` pairs for locks, channels, and one-shot signalling; `AcqRel` for fetch-update RMW; `SeqCst` only when you actually need a total global order (rare).",
  "anti_pattern": "use std::sync::atomic::{AtomicU64, Ordering};\nlet counter = AtomicU64::new(0);\n// Stat counter that is never read with synchronization meaning — SeqCst is overkill.\ncounter.fetch_add(1, Ordering::SeqCst);",
  "good_pattern": "use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};\n// Stat counter: no inter-thread ordering needed.\nlet counter = AtomicU64::new(0);\ncounter.fetch_add(1, Ordering::Relaxed);\n\n// One-shot signal: writer publishes data, then sets the flag with Release;\n// reader checks the flag with Acquire and only then reads the data.\nlet ready = AtomicBool::new(false);\n// writer side\n/* ...write data fields... */\nready.store(true, Ordering::Release);\n// reader side\nwhile !ready.load(Ordering::Acquire) { std::hint::spin_loop(); }\n// /* ...read data fields, guaranteed visible... */"
}
```

**Draft 3.3 — Don't `Box` when stack works (id: rust_quality_93_heap_discipline):**

```json
{
  "id": "rust_quality_93_heap_discipline",
  "category": "Heap Allocation Discipline",
  "source": "Rust for Rustaceans, ch. 9 §Allocation; Rust Performance Book §Heap Allocations",
  "triggers": ["Box small T", "stack vs heap", "unnecessary Box", "Box Copy type", "heap indirection"],
  "rule_summary": "`Box<T>` is for trait objects, recursive types, or genuinely large values you need to move cheaply. Don't wrap small `Copy` or `Clone` types in `Box` for \"indirection\" — heap allocation costs an alloc/dealloc plus a pointer chase on every access. Stack values are faster, simpler, and the compiler optimizes them better. `Box<String>` and `Box<Vec<T>>` are double indirection through an already-heap-allocated type.",
  "prompt_injection": "MANDATORY RULE: Reach for `Box<T>` only when one of three reasons applies — `Box<dyn Trait>` for trait-object polymorphism, `Box<Self>` for recursive types, or `Box<T>` for genuinely large `T` you want to move cheaply. For everything else, store `T` directly. `Box<u32>`, `Box<bool>`, and `Box<String>` are usually a code-smell.",
  "anti_pattern": "struct Config {\n    timeout: Box<u32>,   // u32 is Copy and 4 bytes — Box adds alloc + pointer indirection for nothing\n    name: Box<String>,   // String already owns its bytes on the heap; Box<String> double-indirects\n}",
  "good_pattern": "struct Config {\n    timeout: u32,   // stack, direct access\n    name: String,   // already heap-allocated; one indirection is enough\n}\n// Legitimate Box uses:\nstruct Plugin { handler: Box<dyn EventHandler> }            // trait object\nstruct LinkedList<T> { head: Option<Box<Node<T>>> }         // recursive type\nstruct Frame { huge_buffer: Box<[u8; 4 * 1024 * 1024]> }    // genuinely large; avoid stack overflow"
}
```

After insertion, validate JSON parses.

- [ ] **Step 5: Bump count assertion 90 → 93**

- [ ] **Step 6: Run the batch-3 test, expect pass**

- [ ] **Step 7: Add the perf demo-query test**

```rust
    #[test]
    fn test_perf_demo_query_returns_new_axioms() {
        let result = handle_axioms(
            "performance allocation hot path string Cow iterator monomorphization inline enum dispatch struct layout atomic ordering Box heap",
        )
        .unwrap();
        let new_categories = [
            "Allocation Discipline",
            "String Allocation",
            "Iterator Codegen",
            "Monomorphization Cost",
            "Inline Hints",
            "Enum Dispatch",
            "Struct Layout",
            "Atomic Ordering",
            "Heap Allocation Discipline",
        ];
        let surfaced = new_categories
            .iter()
            .filter(|cat| result.contains(*cat))
            .count();
        assert!(
            surfaced >= 3,
            "expected at least 3 new perf categories in demo query, got {surfaced}"
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
axioms: add perf batch 3 (struct layout, atomic ordering, heap discipline) + demo test

Adds the final three performance entries: struct field layout for minimal
padding (repr(Rust) reorders by default; repr(C) freezes — manual ordering
by descending alignment); atomic ordering selection (pick the weakest that
satisfies the synchronization need; SeqCst is rarely right); and heap
discipline (Box only for trait objects, recursive types, or genuinely large
values; don't double-indirect String). Adds an end-to-end demo test
asserting >= 3 of the 9 new perf categories surface for the broad query.
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

After all tasks: 9 new axioms; 4 new tests; cargo gauntlet clean.

## Risks Realized During Execution

- **Performance claims are quantitative.** If a reviewer challenges "X is faster than Y" without a Godbolt or benchmark reference, treat as a real defect — content correctness for performance is not catchable by `cargo test`.
- **Trigger collisions** with existing performance axioms. Per-axiom focused queries verified by per-batch tests.
- **Plan drift** between drafts and post-fix JSON (recurring). Reconcile before final review.
