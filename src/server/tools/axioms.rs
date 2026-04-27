use serde::Deserialize;
use std::cmp::Reverse;
use std::fmt::Write;
use std::sync::OnceLock;

/// On-disk JSON shape. Fields we never query (`id`) are not deserialised.
/// serde silently skips unknown keys, so dropping `id` from the struct
/// keeps the JSON authoritative without forcing a dead field.
#[derive(Deserialize, Debug)]
struct RawAxiom {
    category: String,
    #[serde(default)]
    source: Option<String>,
    triggers: Vec<String>,
    rule_summary: String,
    prompt_injection: String,
    anti_pattern: String,
    good_pattern: String,
}

/// In-memory axiom with pre-lowercased fields. Scoring touches every
/// axiom on every query; lowering once at load time trades ~a few KB
/// of steady-state memory for avoiding repeated per-query string
/// allocations per call.
#[derive(Debug)]
#[non_exhaustive]
pub struct Axiom {
    pub category: String,
    pub source: Option<String>,
    pub triggers: Vec<String>,
    pub rule_summary: String,
    pub prompt_injection: String,
    pub anti_pattern: String,
    pub good_pattern: String,
    category_lower: String,
    triggers_lower: Vec<String>,
    rule_summary_lower: String,
}

impl From<RawAxiom> for Axiom {
    fn from(raw: RawAxiom) -> Self {
        let category_lower = raw.category.to_lowercase();
        let triggers_lower = raw.triggers.iter().map(|t| t.to_lowercase()).collect();
        let rule_summary_lower = raw.rule_summary.to_lowercase();
        Self {
            category: raw.category,
            source: raw.source,
            triggers: raw.triggers,
            rule_summary: raw.rule_summary,
            prompt_injection: raw.prompt_injection,
            anti_pattern: raw.anti_pattern,
            good_pattern: raw.good_pattern,
            category_lower,
            triggers_lower,
            rule_summary_lower,
        }
    }
}

// Bake the JSON into the binary; the path is relative to this file.
const AXIOMS_JSON: &str = include_str!("../../../assets/axioms.json");
const RUST_QUALITY_AXIOMS_JSON: &str = include_str!("../../../assets/rust_quality_axioms.json");
const MAX_AXIOM_RESULTS: usize = 16;

/// Parse once, cache forever. Parse failure is returned to the caller on
/// first call; later calls will retry until one succeeds.
fn axioms() -> Result<&'static [Axiom], crate::IlluError> {
    static AXIOMS: OnceLock<Vec<Axiom>> = OnceLock::new();
    if let Some(cached) = AXIOMS.get() {
        return Ok(cached);
    }
    let mut raw: Vec<RawAxiom> = serde_json::from_str(AXIOMS_JSON)?;
    raw.extend(serde_json::from_str::<Vec<RawAxiom>>(
        RUST_QUALITY_AXIOMS_JSON,
    )?);
    let parsed: Vec<Axiom> = raw.into_iter().map(Axiom::from).collect();
    // Lost-race set is fine — the winner's Vec is equivalent to ours.
    let _ = AXIOMS.set(parsed);
    AXIOMS.get().map(Vec::as_slice).ok_or_else(|| {
        // Invariant: the `AXIOMS.set(parsed)` above succeeded (or a parallel
        // caller's did) before we reach this branch, so `AXIOMS.get()` must
        // be `Some`. If it isn't, `OnceLock` has violated its own contract —
        // surface as `Other` because this is a genuine should-never-happen
        // rather than any domain category.
        crate::IlluError::Other("axioms cache not initialised after set".to_string())
    })
}

pub fn handle_axioms(query: &str) -> Result<String, crate::IlluError> {
    let axioms = axioms()?;

    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scored: Vec<(&Axiom, usize)> = axioms
        .iter()
        .map(|axiom| {
            let mut score = 0;
            for term in &query_terms {
                if axiom.category_lower.contains(term) {
                    score += 5;
                }
                for trigger in &axiom.triggers_lower {
                    if trigger.contains(term) {
                        score += 10;
                    }
                }
                if axiom.rule_summary_lower.contains(term) {
                    score += 2;
                }
            }
            // Exact-match boost: user typed a category or trigger name
            // verbatim. `.contains` on the full query against single-word
            // strings was unreachable for multi-word queries — this
            // gives both single- and multi-word queries the same shot.
            if axiom.category_lower == query_lower {
                score += 20;
            }
            for trigger in &axiom.triggers_lower {
                if trigger == &query_lower {
                    score += 30;
                }
            }
            (axiom, score)
        })
        .filter(|(_, score)| *score > 0)
        .collect();

    scored.sort_by_key(|&(_, score)| Reverse(score));

    // Keep enough matches for the broad baseline quality query to include
    // both the project-specific design axioms and the stricter Rust API
    // axioms, while still keeping MCP responses short enough to read.
    let top: Vec<&Axiom> = scored
        .into_iter()
        .take(MAX_AXIOM_RESULTS)
        .map(|(a, _)| a)
        .collect();

    let mut output = String::new();
    let _ = writeln!(output, "## Rust Axioms matching '{query}'\n");

    if top.is_empty() {
        let _ = writeln!(
            output,
            "No matching Rust Axioms found in the database. Please refine your query."
        );
        return Ok(output);
    }

    for axiom in top {
        let _ = writeln!(output, "### [{}] {}", axiom.category, axiom.rule_summary);
        if let Some(source) = &axiom.source {
            let _ = writeln!(output, "_Source: {source}_");
        }
        let _ = writeln!(output, "> **{}**\n", axiom.prompt_injection);

        if !axiom.good_pattern.is_empty() {
            let _ = writeln!(
                output,
                "#### Good Pattern:\n```rust\n{}\n```",
                axiom.good_pattern
            );
        }
        if !axiom.anti_pattern.is_empty() {
            let _ = writeln!(
                output,
                "#### Anti-Pattern:\n```rust\n{}\n```",
                axiom.anti_pattern
            );
        }
        let _ = writeln!(output, "---");
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[derive(Deserialize)]
    struct TestAxiom {
        id: String,
        category: String,
        source: Option<String>,
        triggers: Vec<String>,
        rule_summary: String,
        prompt_injection: String,
        anti_pattern: String,
        good_pattern: String,
    }

    #[test]
    fn test_handle_axioms() {
        let query = "thread shared state unwrap";
        let result = handle_axioms(query).unwrap();
        assert!(result.contains("unwrap"));
    }

    #[test]
    fn test_exact_match_boost_single_word() {
        // An exact trigger match should outrank a mere substring hit.
        let result = handle_axioms("ownership").unwrap();
        assert!(result.contains("Ownership"));
    }

    #[test]
    fn test_rust_quality_axioms_are_loaded_with_sources() {
        let result = handle_axioms("miri undefined behavior impeccable rust").unwrap();
        assert!(result.contains("Miri"));
        assert!(result.contains("Source:"));
    }

    #[test]
    fn test_quality_query_returns_design_axioms() {
        let result = handle_axioms(crate::agents::instruction_md::RUST_QUALITY_QUERY).unwrap();
        assert!(result.contains("Design Workflow"));
        assert!(result.contains("Data Modeling"));
        assert!(result.contains("Documentation"));
        assert!(result.contains("Comments"));
        assert!(result.contains("Idiomatic Rust"));
        assert!(result.contains("Verification Sources"));
        assert!(result.contains("Performance Discipline"));
    }

    #[test]
    fn test_error_handling_axioms_batch_1_present() {
        // Per-axiom focused queries: each new entry must rank within
        // MAX_AXIOM_RESULTS for a query built from its own triggers.
        // Combined-broad queries are fragile against crowding from later batches.
        let result =
            handle_axioms("Error::source error chain source method wrapped error").unwrap();
        assert!(
            result.contains("Error Source Chain"),
            "Error Source Chain missing in focused query"
        );

        let result = handle_axioms("map_err propagate boundary domain context wrap error").unwrap();
        assert!(
            result.contains("Error Boundary Discipline"),
            "Error Boundary Discipline missing in focused query"
        );

        let result =
            handle_axioms("variant naming InvalidUtf8 Display impl error message style").unwrap();
        assert!(
            result.contains("Error API Surface"),
            "Error API Surface missing in focused query"
        );
    }

    #[test]
    fn test_error_handling_axioms_batch_2_present() {
        let result = handle_axioms(
            "error category io domain invariant Other variant stringly typed catchall",
        )
        .unwrap();
        assert!(
            result.contains("Error Category Structure"),
            "Error Category Structure missing in focused query"
        );

        let result = handle_axioms(
            "backtrace Backtrace::capture stack context RUST_BACKTRACE error origin stack walk",
        )
        .unwrap();
        assert!(
            result.contains("Backtrace Policy"),
            "Backtrace Policy missing in focused query"
        );

        let result = handle_axioms(
            "non_exhaustive insufficient stable variant error contract semver library error stability",
        )
        .unwrap();
        assert!(
            result.contains("Error Stability"),
            "Error Stability missing in focused query"
        );
    }

    #[test]
    fn test_error_handling_axioms_batch_3_present() {
        let result = handle_axioms(
            "error context typed format string eyre context structured error context fields",
        )
        .unwrap();
        assert!(
            result.contains("Error Context"),
            "Error Context missing in focused query"
        );

        let result = handle_axioms(
            "Box dyn Error internal helper library private function anyhow internally structured error",
        )
        .unwrap();
        assert!(
            result.contains("Error Type Discipline"),
            "Error Type Discipline missing in focused query"
        );

        let result = handle_axioms(
            "From impl error conversion auto convert error public surface conversion graph error From",
        )
        .unwrap();
        assert!(
            result.contains("Error Conversion Surface"),
            "Error Conversion Surface missing in focused query"
        );
    }

    #[test]
    fn test_error_handling_axioms_batch_4_present() {
        let result = handle_axioms(
            "test error variant is_err assert matches failure path coverage matches macro",
        )
        .unwrap();
        assert!(
            result.contains("Error Path Specificity"),
            "Error Path Specificity missing in focused query"
        );
    }

    #[test]
    fn test_error_handling_demo_query_returns_new_axioms() {
        let result = handle_axioms("error source chain wrap propagate variant naming").unwrap();
        // Expect at least 3 of the new categories to surface in the top results.
        let new_categories = [
            "Error Source Chain",
            "Error Boundary Discipline",
            "Error API Surface",
            "Error Category Structure",
            "Backtrace Policy",
            "Error Stability",
            "Error Context",
            "Error Type Discipline",
            "Error Conversion Surface",
            "Error Path Specificity",
        ];
        let surfaced = new_categories
            .iter()
            .filter(|cat| result.contains(*cat))
            .count();
        assert!(
            surfaced >= 3,
            "expected at least 3 new categories in demo query, got {surfaced}"
        );
    }

    #[test]
    fn test_ownership_axioms_batch_1_present() {
        let result =
            handle_axioms("NLL non-lexical lifetimes borrow scope last use borrow flow").unwrap();
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

    #[test]
    fn test_axiom_assets_have_unique_ids_and_required_fields() {
        let mut axioms: Vec<TestAxiom> = serde_json::from_str(AXIOMS_JSON).unwrap();
        axioms.extend(serde_json::from_str::<Vec<TestAxiom>>(RUST_QUALITY_AXIOMS_JSON).unwrap());
        let mut ids = BTreeSet::new();
        let mut rust_quality_axiom_count = 0;

        for axiom in axioms {
            if axiom.id.starts_with("rust_quality_") {
                rust_quality_axiom_count += 1;
                assert!(
                    axiom
                        .source
                        .as_deref()
                        .is_some_and(|source| !source.trim().is_empty()),
                    "{}",
                    axiom.id
                );
            }
            assert!(!axiom.id.trim().is_empty());
            assert!(
                ids.insert(axiom.id.clone()),
                "duplicate axiom id {}",
                axiom.id
            );
            assert!(!axiom.category.trim().is_empty(), "{}", axiom.id);
            assert!(!axiom.triggers.is_empty(), "{}", axiom.id);
            assert!(
                axiom
                    .triggers
                    .iter()
                    .all(|trigger| !trigger.trim().is_empty()),
                "{}",
                axiom.id
            );
            assert!(!axiom.rule_summary.trim().is_empty(), "{}", axiom.id);
            assert!(!axiom.prompt_injection.trim().is_empty(), "{}", axiom.id);
            assert!(!axiom.anti_pattern.trim().is_empty(), "{}", axiom.id);
            assert!(!axiom.good_pattern.trim().is_empty(), "{}", axiom.id);
        }

        assert_eq!(rust_quality_axiom_count, 102);
    }

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

        let result =
            handle_axioms("drop order field declaration order destructor sequence struct drop")
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

        let result =
            handle_axioms("MutexGuard await deadlock async lock std sync Mutex tokio sync Mutex")
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

    #[test]
    fn test_ownership_demo_query_returns_new_axioms() {
        let result = handle_axioms(
            "ownership borrow lifetime variance drop pin reborrow interior mutability",
        )
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

        let result =
            handle_axioms("#[inline] inline always inline never cross-crate inline force inline")
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

        let result =
            handle_axioms("repr(C) FFI safe layout stable Option NonNull c_int c_uchar FFI types")
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
}
