# Jon-Style Rust Phase 1 Slice 2 (Type-System) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land 9 new type-system axioms (IDs 76–84) in `assets/rust_quality_axioms.json`, each user-reviewed and source-cited, with 3 per-batch focused-query coverage tests + 1 end-to-end demo test. Count assertion bumped 75 → 84.

**Architecture:** Unchanged from Phase 0 / Phase 1 Slice 1. Pure content enrichment; no new files, no new MCP tools, no schema changes.

**Tech Stack:** Rust 2024, `serde_json`, `cargo test` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo fmt --all -- --check`, `mcp__illu__axioms` for surfacing demo.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-1-types-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-1-types-design.md)

**Existing state:** 75 axioms in `assets/rust_quality_axioms.json` (Phase 0 + Phase 1 Slice 1). The test `test_axiom_assets_have_unique_ids_and_required_fields` currently asserts `rust_quality_axiom_count == 75`. New IDs: 76–84.

**Drafts pre-approved:** User reviewed the 9 candidate axioms and approved as-is during the brainstorming compression step. Skip the "User reviews drafted axiom" step inside each batch task; integrate verbatim. Per-batch checkpoints between dispatches still happen at the controller level.

**Per-batch tests use focused queries** (Phase 0 / Slice 1 standard).

**Quality gate:** `mcp__illu__quality_gate` likely returns BLOCKED on the test-`unwrap()` heuristic false-positive — acceptable when `cargo clippy -D warnings` exits 0.

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `assets/rust_quality_axioms.json` | Append 9 entries | New axiom data |
| `src/server/tools/axioms.rs` | Add 4 test fns + extend 1 | Per-batch coverage tests + types demo test |

No other files touched.

---

## Task 1: Batch 1 — Trait surface and sealing (Sealed traits, Object safety, Object-safe API design)

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries before the closing `]`, after `rust_quality_75_pin_discipline`)
- Modify: `src/server/tools/axioms.rs` (add `test_types_axioms_batch_1_present`; bump count assertion 75 → 78)

- [ ] **Step 1: Confirm starting ID**

```bash
grep -oE '"id": "rust_quality_[0-9]+_' assets/rust_quality_axioms.json | grep -oE '[0-9]+' | sort -n | tail -1
```
Expected: `75`. If different, bump IDs in the drafts and adjust subsequent batches.

- [ ] **Step 2: Write the failing batch-1 coverage test**

Add inside `mod tests` in `src/server/tools/axioms.rs`:

```rust
    #[test]
    fn test_types_axioms_batch_1_present() {
        let result = handle_axioms(
            "sealed trait private super trait block external impl pub trait extension prevention",
        )
        .unwrap();
        assert!(
            result.contains("Sealed Traits"),
            "Sealed Traits missing in focused query"
        );

        let result = handle_axioms(
            "object safety dyn compatible Self Sized escape dispatchable receiver generic methods trait object",
        )
        .unwrap();
        assert!(
            result.contains("Object Safety"),
            "Object Safety missing in focused query"
        );

        let result = handle_axioms(
            "object-safe API design preserve object safety extension trait dyn plugin trait deliberate non-object-safe",
        )
        .unwrap();
        assert!(
            result.contains("Object-Safe API Design"),
            "Object-Safe API Design missing in focused query"
        );
    }
```

- [ ] **Step 3: Run the new test, expect failure**

Run: `cargo test --lib -- test_types_axioms_batch_1_present`
Expected: FAIL — three categories not yet in JSON.

- [ ] **Step 4: SKIP — drafts pre-approved by user**

- [ ] **Step 5: Append approved drafts to JSON**

The file is a top-level JSON array. Locate the closing `]`. Insert the three drafts immediately before it, with a comma after the previous last entry (`rust_quality_75_pin_discipline`).

**Draft 1.1 — Sealed traits (id: rust_quality_76_sealed_traits):**

```json
{
  "id": "rust_quality_76_sealed_traits",
  "category": "Sealed Traits",
  "source": "Rust for Rustaceans, ch. 2 §Coherence; rust-imap pattern; Rust API Guidelines C-SEALED",
  "triggers": ["sealed trait", "private super trait", "block external impl", "pub trait", "extension prevention", "trait sealing"],
  "rule_summary": "Use a private super trait to seal a public trait: external crates can call methods on it but cannot implement it. Pattern: a private module containing a `Sealed` trait, then `pub trait Foo: private::Sealed {}` plus `impl private::Sealed for ...` only for the types you control.",
  "prompt_injection": "MANDATORY RULE: When you publish a trait whose set of implementors must remain bounded (closed-set extensions over std types, library-controlled implementations only), seal it with a private super trait. Without sealing, every minor-version method addition risks breaking external implementors.",
  "anti_pattern": "// Unsealed: external crate can `impl Pretty for u32 { ... }`, and any future\n// minor-version method addition would break that external impl.\npub trait Pretty {\n    fn pretty(&self) -> String;\n}",
  "good_pattern": "mod private {\n    pub trait Sealed {}\n}\n\npub trait Pretty: private::Sealed {\n    fn pretty(&self) -> String;\n}\n\n// We control all impls.\nimpl private::Sealed for String {}\nimpl Pretty for String {\n    fn pretty(&self) -> String { format!(\"\\\"{self}\\\"\") }\n}\n// External crates can call .pretty() but cannot impl Pretty for their own types\n// because they cannot impl private::Sealed."
}
```

**Draft 1.2 — Object safety / dyn compatibility (id: rust_quality_77_object_safety):**

```json
{
  "id": "rust_quality_77_object_safety",
  "category": "Object Safety",
  "source": "Rust Reference §Object Safety; Crust of Rust: Trait Objects",
  "triggers": ["object safety", "dyn compatible", "dyn Trait", "Self: Sized escape", "dispatchable receiver", "generic methods trait object"],
  "rule_summary": "A trait is object-safe (dyn-compatible) only if every method either has a dispatchable receiver and no generic type parameters, or is excluded from the vtable via `where Self: Sized`. `Self` may not appear by value in returns. Dispatchable receivers: `&self`, `&mut self`, `Box<Self>`, `Rc<Self>`, `Arc<Self>`, `Pin<P>`.",
  "prompt_injection": "MANDATORY RULE: If callers may use your trait as `dyn Trait`, design every method to be dispatchable (no generic params, dispatchable receiver, no Self by value) or gate it behind `where Self: Sized` so it is excluded from the vtable. Adding a generic method later silently breaks `dyn Trait` usage at every call site.",
  "anti_pattern": "pub trait Render {\n    fn into_string(self) -> String;        // Self by value: not object-safe\n    fn fmt<W: Write>(&self, w: &mut W);   // generic method: not object-safe\n}\nlet r: Box<dyn Render> = /* ... */;       // ERROR: trait `Render` is not dyn compatible",
  "good_pattern": "pub trait Render {\n    fn fmt(&self, w: &mut dyn Write);                 // dispatchable, no generics\n    fn into_string(self) -> String where Self: Sized; // gated: not on the vtable\n}\nlet r: Box<dyn Render> = /* ... */;                   // OK\nr.fmt(&mut buf);                                       // works through the vtable\n// r.into_string() is unavailable through dyn (Sized-gated) but works on concrete T."
}
```

**Draft 1.3 — Object-safe API design (id: rust_quality_78_object_safe_api_design):**

```json
{
  "id": "rust_quality_78_object_safe_api_design",
  "category": "Object-Safe API Design",
  "source": "Rust for Rustaceans, ch. 2 §Trait Objects and Object Safety",
  "triggers": ["object-safe API", "preserve object safety", "extension trait dyn", "plugin trait", "deliberate non-object-safe", "ext trait pattern"],
  "rule_summary": "Design traits to be object-safe by default; give it up only when the trait is genuinely a static-only abstraction. For traits exposed to user-defined plugins or heterogeneous collections, object safety is required. Provide generic conveniences via a separate extension trait with a blanket impl, so the core stays dyn-compatible.",
  "prompt_injection": "MANDATORY RULE: When designing a public trait, decide upfront whether `dyn Trait` is in scope. If yes, keep methods object-safe and put generic helpers on a separate `*Ext` extension trait with `impl<T: BaseTrait + ?Sized> Ext for T {}`. If no, document the decision in rustdoc — callers cannot store `Box<dyn YourTrait>`.",
  "anti_pattern": "// Trait meant for plugins, accidentally not object-safe due to a generic method.\npub trait LoggerPlugin {\n    fn name(&self) -> &str;                          // object-safe\n    fn log<W: Write>(&self, msg: &str, sink: W);     // generic: breaks dyn\n}",
  "good_pattern": "pub trait LoggerPlugin {\n    fn name(&self) -> &str;\n    fn log(&self, msg: &str, sink: &mut dyn Write);\n}\n// Generic helper as extension trait with blanket impl:\npub trait LoggerPluginExt: LoggerPlugin {\n    fn log_to<W: Write>(&self, msg: &str, mut sink: W) {\n        self.log(msg, &mut sink as &mut dyn Write);\n    }\n}\nimpl<T: LoggerPlugin + ?Sized> LoggerPluginExt for T {}"
}
```

After insertion, validate JSON parses:
```bash
python3 -c "import json; json.load(open('assets/rust_quality_axioms.json')); print('valid')"
```

- [ ] **Step 6: Bump the count assertion**

Update `assert_eq!(rust_quality_axiom_count, 75);` to `assert_eq!(rust_quality_axiom_count, 78);` in `test_axiom_assets_have_unique_ids_and_required_fields`.

- [ ] **Step 7: Run the batch-1 test, expect pass**

Run: `cargo test --lib -- test_types_axioms_batch_1_present`
Expected: PASS. **If any focused query fails, STOP and report BLOCKED.**

- [ ] **Step 8: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 9: Lint and format**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
Expected: clean.

- [ ] **Step 10: Commit batch 1**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add types batch 1 (sealed traits, object safety, object-safe API design)

Adds three entries on trait public surface: the sealed-trait pattern via
private super trait (block external impls while keeping methods callable);
the object-safety / dyn-compatibility rules (no generic methods, dispatchable
receivers, Self: Sized escape hatch); and the design-time decision of when
to preserve object safety via a separate *Ext extension trait with blanket
impl, vs when to deliberately give it up for monomorphization-only APIs.
EOF
)"
```

---

## Task 2: Batch 2 — Type and trait choice (Generic vs dyn, Associated types vs generics, HRTBs)

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_types_axioms_batch_2_present`; bump count assertion 78 → 81)

- [ ] **Step 1: Write the failing batch-2 coverage test**

```rust
    #[test]
    fn test_types_axioms_batch_2_present() {
        let result = handle_axioms(
            "generic vs dyn static dispatch dynamic dispatch monomorphization vtable binary size dispatch",
        )
        .unwrap();
        assert!(
            result.contains("Static vs Dynamic Dispatch"),
            "Static vs Dynamic Dispatch missing in focused query"
        );

        let result = handle_axioms(
            "associated type generic parameter Iterator Item Add Output type relation one impl per type",
        )
        .unwrap();
        assert!(
            result.contains("Associated Types"),
            "Associated Types missing in focused query"
        );

        let result = handle_axioms(
            "HRTB for<'a> higher-ranked trait bound callback any lifetime Fn arbitrary lifetime borrow checker callback",
        )
        .unwrap();
        assert!(
            result.contains("Higher-Ranked Trait Bounds"),
            "Higher-Ranked Trait Bounds missing in focused query"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

Run: `cargo test --lib -- test_types_axioms_batch_2_present`
Expected: FAIL.

- [ ] **Step 3: SKIP — drafts pre-approved by user**

- [ ] **Step 4: Append approved drafts to JSON**

**Draft 2.1 — Generic vs trait object (id: rust_quality_79_static_vs_dynamic_dispatch):**

```json
{
  "id": "rust_quality_79_static_vs_dynamic_dispatch",
  "category": "Static vs Dynamic Dispatch",
  "source": "Rust for Rustaceans, ch. 2 §Generics vs Trait Objects; Rust Book ch. 17",
  "triggers": ["generic vs dyn", "static dispatch", "dynamic dispatch", "monomorphization", "vtable", "binary size dispatch"],
  "rule_summary": "Pick generics (`fn foo<T: Trait>(x: T)`) for hot paths and homogeneous types: monomorphization, inlining, no indirect call. Pick trait objects (`fn foo(x: &dyn Trait)`) for heterogeneous collections, plugin boundaries, and binary-size budgets: single code copy, vtable indirection. Mixing both in one API is rarely the right answer.",
  "prompt_injection": "MANDATORY RULE: Default to generics for performance-sensitive code that handles a single type per call site. Use `dyn Trait` only when you need heterogeneous collections, plugin boundaries that decouple consumers from concrete types, or to keep binary size bounded. Document the choice when it is non-obvious.",
  "anti_pattern": "// Heterogeneous renderable items forced through monomorphization — Vec<T> can hold only one T.\nfn render_all<T: Render>(items: Vec<T>) { /* one concrete type per call */ }\n\n// Hot-path callback through dyn — indirect call, missed inlining.\nfn for_each(callback: &dyn Fn(usize), n: usize) {\n    for i in 0..n { callback(i); }\n}",
  "good_pattern": "// Heterogeneous: dyn.\nfn render_all(items: Vec<Box<dyn Render>>) { /* mixed concrete types in one Vec */ }\n\n// Hot-path: generic, monomorphized per F.\nfn for_each<F: Fn(usize)>(callback: F, n: usize) {\n    for i in 0..n { callback(i); }\n}"
}
```

**Draft 2.2 — Associated types vs generic parameters (id: rust_quality_80_associated_types):**

```json
{
  "id": "rust_quality_80_associated_types",
  "category": "Associated Types",
  "source": "Rust for Rustaceans, ch. 2 §Associated Types vs Generics; Rust Book ch. 19",
  "triggers": ["associated type", "generic parameter", "Iterator Item", "Add Output", "type relation", "one impl per type"],
  "rule_summary": "Use an associated type when the trait has one logical impl per type (`Iterator::Item` is determined by the iterator). Use a generic parameter when the caller picks (`Add<RHS>` lets one type implement addition with multiple right-hand-side types). Don't mix both unless the relation requires it.",
  "prompt_injection": "MANDATORY RULE: Decide for each trait whether the related type is determined by the implementor (associated type) or chosen by the caller (generic parameter). If implementors have exactly one natural choice, an associated type spares callers from spelling it out. If multiple choices coexist on one type, a generic parameter is necessary.",
  "anti_pattern": "// Generic parameter when only one impl per type makes sense — caller forced to spell\n// the only valid choice every time.\ntrait Container<Item> {\n    fn add(&mut self, x: Item);\n}\nimpl Container<u32> for MyVec { /* the only impl */ }\nfn use_it<C: Container<u32>>(c: &mut C) { /* must spell u32 every time */ }",
  "good_pattern": "// Associated type: one Item per Container type; caller does not spell it.\ntrait Container {\n    type Item;\n    fn add(&mut self, x: Self::Item);\n}\nimpl Container for MyVec { type Item = u32; /* ... */ }\nfn use_it<C: Container<Item = u32>>(c: &mut C) { /* spelled when caller pins it */ }\nfn use_it_any<C: Container>(c: &mut C, x: C::Item) { /* opaque to caller */ }"
}
```

**Draft 2.3 — HRTBs (id: rust_quality_81_hrtbs):**

```json
{
  "id": "rust_quality_81_hrtbs",
  "category": "Higher-Ranked Trait Bounds",
  "source": "Rust for Rustaceans, ch. 2 §HRTBs; Rust Reference §Higher-Rank Trait Bounds; Crust of Rust: Lifetime Annotations",
  "triggers": ["HRTB", "for<'a>", "higher-ranked trait bound", "callback any lifetime", "Fn arbitrary lifetime", "borrow checker callback"],
  "rule_summary": "`for<'a> Fn(&'a T) -> &'a U` declares the callback works for any caller-chosen lifetime, not a single named lifetime fixed at the bound site. Use HRTBs when a function passes borrowed data into a callback whose lifetime cannot be tied to the function's own type parameters.",
  "prompt_injection": "MANDATORY RULE: When a callback bound describes a closure or function pointer that the implementation will invoke with locally-borrowed data, write the bound as `for<'a> Fn(&'a T) -> ...` (HRTB). A single named lifetime tied to the outer function's type parameters constrains the callback to one specific lifetime, which usually fails when the implementation has only a shorter borrow to offer.",
  "anti_pattern": "// Single named lifetime: caller picks 'a, function is locked to that lifetime.\n// Trying to call f on a locally-borrowed value fails (E0597) because the local\n// borrow's scope is shorter than 'a.\nfn process<'a, F>(input: &'a str, f: F)\nwhere F: Fn(&'a str) -> bool\n{\n    let scratch = format!(\"[{input}]\"); // owned, function-local\n    if f(&scratch) {                     // ERROR: scratch's borrow shorter than 'a\n        /* ... */\n    }\n}",
  "good_pattern": "// HRTB: callback works for any lifetime — including scratch's local one.\nfn process<F>(input: &str, f: F)\nwhere F: for<'a> Fn(&'a str) -> bool\n{\n    let scratch = format!(\"[{input}]\");\n    if f(&scratch) {                     // OK: HRTB accepts any &str\n        /* ... */\n    }\n}"
}
```

After insertion, validate JSON parses.

- [ ] **Step 5: Bump count assertion 78 → 81.**

- [ ] **Step 6: Run the batch-2 test, expect pass**

Run: `cargo test --lib -- test_types_axioms_batch_2_present`
Expected: PASS.

- [ ] **Step 7: Full test suite**

Run: `cargo test --lib`
Expected: all pass.

- [ ] **Step 8: Lint and format**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 9: Commit batch 2**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add types batch 2 (static vs dynamic dispatch, associated types, HRTBs)

Adds three entries on interface design choices: generics vs trait objects
(monomorphization vs vtable; static for hot/homogeneous, dyn for
heterogeneous/binary-size); associated types vs generic parameters
(implementor-determined vs caller-chosen); higher-ranked trait bounds for
callbacks that must work over any lifetime the implementation can offer.
EOF
)"
```

---

## Task 3: Batch 3 — Type-level building blocks (?Sized, ZST markers, Marker/auto traits) + demo test

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_types_axioms_batch_3_present` and `test_types_demo_query_returns_new_axioms`; bump count assertion 81 → 84)

- [ ] **Step 1: Write the failing batch-3 coverage test**

```rust
    #[test]
    fn test_types_axioms_batch_3_present() {
        let result = handle_axioms(
            "?Sized DST dynamically sized type implicit Sized Box T ?Sized Arc T ?Sized",
        )
        .unwrap();
        assert!(
            result.contains("Sized Bound"),
            "Sized Bound missing in focused query"
        );

        let result = handle_axioms(
            "ZST zero-sized type unit struct compile-time marker type witness Vec () no allocation",
        )
        .unwrap();
        assert!(
            result.contains("Zero-Sized Types"),
            "Zero-Sized Types missing in focused query"
        );

        let result = handle_axioms(
            "auto trait Send Sync auto marker trait Eq Hash coherence derive Eq Hash agree manual auto trait",
        )
        .unwrap();
        assert!(
            result.contains("Marker and Auto Traits"),
            "Marker and Auto Traits missing in focused query"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

Run: `cargo test --lib -- test_types_axioms_batch_3_present`
Expected: FAIL.

- [ ] **Step 3: SKIP — drafts pre-approved by user**

- [ ] **Step 4: Append approved drafts to JSON**

**Draft 3.1 — `?Sized` discipline (id: rust_quality_82_sized_bound):**

```json
{
  "id": "rust_quality_82_sized_bound",
  "category": "Sized Bound",
  "source": "Rust for Rustaceans, ch. 2 §Sized and DSTs; Rustonomicon §Exotically Sized Types",
  "triggers": ["?Sized", "DST", "dynamically sized type", "implicit Sized", "Box T ?Sized", "Arc T ?Sized"],
  "rule_summary": "Generic type parameters carry an implicit `Sized` bound. Relax it with `T: ?Sized` to accept dynamically-sized types (`str`, `[T]`, `dyn Trait`). The cost is that the parameter can only be used through references or owning pointers (`&T`, `&mut T`, `Box<T>`, `Arc<T>`, `Rc<T>`); never by value, since the size is unknown at compile time.",
  "prompt_injection": "MANDATORY RULE: For container or wrapper types that should accept DSTs, write the parameter as `T: ?Sized` and use it only through references or owning smart pointers. For parameters used by value (returned, moved, stored inline), keep the implicit `Sized` bound.",
  "anti_pattern": "// Implicit Sized: cannot wrap str or dyn Trait.\nstruct Wrapper<T> { inner: Box<T> }\nlet w: Wrapper<str> = /* ... */;        // ERROR: str is not Sized\nlet w: Wrapper<dyn Display> = /* ... */; // ERROR: dyn Display is not Sized",
  "good_pattern": "struct Wrapper<T: ?Sized> { inner: Box<T> }\nlet w: Wrapper<str> = Wrapper { inner: \"hi\".to_owned().into_boxed_str() };\nlet w: Wrapper<dyn std::fmt::Display> = Wrapper { inner: Box::new(42_u32) };\n// Methods that move T by value still need T: Sized; reference-based methods are fine."
}
```

**Draft 3.2 — ZST markers (id: rust_quality_83_zst_markers):**

```json
{
  "id": "rust_quality_83_zst_markers",
  "category": "Zero-Sized Types",
  "source": "Rust for Rustaceans, ch. 2 §Zero-Sized Types; Rust Book ch. 5",
  "triggers": ["ZST", "zero-sized type", "unit struct", "compile-time marker", "type witness", "Vec () no allocation"],
  "rule_summary": "A zero-sized type (`struct A;`, `()`, `PhantomData<T>`, structs with all-ZST fields) costs no memory at runtime but carries information at compile time. Use them for typestate phases, capability tokens, type-level booleans, and marker bounds. `Vec<()>` never allocates element storage — pushes only update the length counter.",
  "prompt_injection": "MANDATORY RULE: When you need a compile-time witness that an operation has happened or that a value belongs to a category, use a ZST. Unit structs, `PhantomData`, and ZST-only fields encode information in the type system at zero runtime cost.",
  "anti_pattern": "// Runtime boolean to indicate an authentication state that is statically known.\nstruct Connection { authenticated: bool }\nimpl Connection {\n    fn send(&self, _msg: &str) {\n        assert!(self.authenticated, \"not authenticated\"); // runtime check\n    }\n}",
  "good_pattern": "use std::marker::PhantomData;\n// ZST markers + typestate: invalid state unrepresentable at compile time.\nstruct Anon;        // 0 bytes\nstruct Authed;      // 0 bytes\nstruct Connection<S> { _state: PhantomData<S> }\nimpl Connection<Anon> {\n    fn login(self) -> Connection<Authed> {\n        Connection { _state: PhantomData }\n    }\n}\nimpl Connection<Authed> {\n    fn send(&self, _msg: &str) { /* compile-time guaranteed authenticated */ }\n}"
}
```

**Draft 3.3 — Marker traits and auto traits (id: rust_quality_84_marker_auto_traits):**

```json
{
  "id": "rust_quality_84_marker_auto_traits",
  "category": "Marker and Auto Traits",
  "source": "Rust for Rustaceans, ch. 2 §Auto Traits and Markers; Rust Reference §Auto Traits",
  "triggers": ["auto trait", "Send Sync auto", "marker trait", "Eq Hash coherence", "derive Eq Hash agree", "manual auto trait"],
  "rule_summary": "Auto traits (`Send`, `Sync`, `Sized`, `Unpin`, `UnwindSafe`, `RefUnwindSafe`) are inferred by the compiler from the fields of your type. Do not impl them explicitly except `unsafe impl` for the rare type that wraps a raw pointer or other `!Send`/`!Sync` primitive whose external invariants you have audited. Non-auto coherence-bound traits (`Hash` and `Eq` must agree, `Ord` and `PartialOrd` must agree) require deliberate alignment when hand-implemented.",
  "prompt_injection": "MANDATORY RULE: Do not explicitly impl auto traits for normal types — let the compiler propagate them from fields. Use `unsafe impl Send`/`Sync` only for types that wrap raw pointers or other `!Send`/`!Sync` primitives whose invariants you have audited. For non-auto coherence-bound traits (`Hash`/`Eq`, `Ord`/`PartialOrd`), keep manual impls in agreement so `a == b` implies `hash(a) == hash(b)` and `cmp(a, b)` implies `partial_cmp(a, b)`.",
  "anti_pattern": "// Hand-implemented Hash that disagrees with derived Eq: Eq compares both fields,\n// but Hash only mixes one — items that are Eq can hash differently, breaking the\n// HashMap contract.\n#[derive(PartialEq, Eq)]\nstruct Item { id: u64, version: u64 }\nimpl std::hash::Hash for Item {\n    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {\n        self.id.hash(state); // ignores version that Eq compares\n    }\n}",
  "good_pattern": "// Default: derive both, fields agree.\n#[derive(PartialEq, Eq, Hash)]\nstruct Item { id: u64, version: u64 }\n\n// Or: id-only identity, with Hash and Eq aligned manually.\nstruct Item2 { id: u64, version: u64 }\nimpl PartialEq for Item2 { fn eq(&self, o: &Self) -> bool { self.id == o.id } }\nimpl Eq for Item2 {}\nimpl std::hash::Hash for Item2 {\n    fn hash<H: std::hash::Hasher>(&self, state: &mut H) { self.id.hash(state); }\n}\n// Now: a == b ⟺ hash(a) == hash(b). Contract preserved."
}
```

After insertion, validate JSON parses.

- [ ] **Step 5: Bump count assertion 81 → 84.**

- [ ] **Step 6: Run the batch-3 test, expect pass**

Run: `cargo test --lib -- test_types_axioms_batch_3_present`
Expected: PASS.

- [ ] **Step 7: Add the types demo-query test**

```rust
    #[test]
    fn test_types_demo_query_returns_new_axioms() {
        let result = handle_axioms(
            "trait object generic dyn associated type sealed Sized HRTB ZST marker Send Sync",
        )
        .unwrap();
        let new_categories = [
            "Sealed Traits",
            "Object Safety",
            "Object-Safe API Design",
            "Static vs Dynamic Dispatch",
            "Associated Types",
            "Higher-Ranked Trait Bounds",
            "Sized Bound",
            "Zero-Sized Types",
            "Marker and Auto Traits",
        ];
        let surfaced = new_categories
            .iter()
            .filter(|cat| result.contains(*cat))
            .count();
        assert!(
            surfaced >= 3,
            "expected at least 3 new types categories in demo query, got {surfaced}"
        );
    }
```

- [ ] **Step 8: Run the demo test, expect pass**

Run: `cargo test --lib -- test_types_demo_query_returns_new_axioms`
Expected: PASS. **If FAIL with surfaced < 3, STOP and report BLOCKED.**

- [ ] **Step 9: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 10: Lint and format**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 11: Commit batch 3**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add types batch 3 (?Sized, ZST markers, marker/auto traits) + demo test

Adds the final three type-system entries: ?Sized discipline (relax the
implicit Sized bound to accept DSTs through references/smart pointers);
ZST markers (zero-cost compile-time witnesses via unit structs and
PhantomData); and marker/auto-trait propagation rules (don't impl Send/Sync
manually except via unsafe impl for raw-pointer wrappers; keep Hash/Eq and
Ord/PartialOrd in agreement when hand-implemented). Adds an end-to-end
demo test asserting >= 3 of the 9 new types categories surface for the
canonical broad query.
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
Expected: all exit 0.

- [ ] **Step 2: MCP-level demo query** (will return stale results until binary restart)

```
mcp__illu__axioms(query: "trait object generic dyn associated type sealed Sized HRTB ZST marker Send Sync")
```
Expected after binary refresh: at least 3 of the 9 new categories surface.

- [ ] **Step 3: Pre-merge plan-reconciliation pass**

If any content fix-ups landed during execution, ensure the plan's draft sections were updated to match the post-fix JSON (recurring drift issue from Phases 0 / 1S1). One commit before merge if needed.

---

## Verification Summary

After all tasks: 9 new axioms in JSON; 4 new tests; cargo gauntlet clean; demo test asserts ≥3 surface; plan reflects final content.

## Risks Realized During Execution

- **Trigger collisions** with existing trait/type axioms (`[Trait Design]`, `[Trait Objects]`, `[Traits]`, `[Type Safety]`, `[Composition]`, `[Typestate]`). Mitigation: per-axiom focused queries verified by per-batch tests.
- **Subtle technical content** — object safety / dyn-compatibility rules, HRTBs, `?Sized` semantics, auto-trait propagation. These caught real bugs in Phase 1 Slice 1 (variance). Mitigation: pre-flight technical sanity check on each batch's drafts; engage code-quality reviewer on each batch.
- **Plan drift** between committed drafts and post-fix JSON content (recurring across Phase 0 and 1S1). Mitigation: explicit reconciliation pass before final review.
