use serde::Deserialize;
use std::cmp::Reverse;
use std::fmt::Write;
use std::sync::OnceLock;

/// On-disk JSON shape. The `id` is preserved on the parsed [`Axiom`] so
/// other modules (e.g. `exemplars`, `project_style`) can cross-reference
/// axioms by stable identifier; serde still silently skips any other
/// unknown keys.
///
/// Visibility: `pub(crate)` so the project-style loader can deserialize
/// `local_axioms[]` through the same shape rather than forking the schema.
#[derive(Deserialize, Debug)]
pub(crate) struct RawAxiom {
    pub(crate) id: String,
    pub(crate) category: String,
    #[serde(default)]
    pub(crate) source: Option<String>,
    pub(crate) triggers: Vec<String>,
    pub(crate) rule_summary: String,
    pub(crate) prompt_injection: String,
    pub(crate) anti_pattern: String,
    pub(crate) good_pattern: String,
}

/// In-memory axiom with pre-lowercased fields. Scoring touches every
/// axiom on every query; lowering once at load time trades ~a few KB
/// of steady-state memory for avoiding repeated per-query string
/// allocations per call.
#[derive(Debug)]
#[non_exhaustive]
pub struct Axiom {
    pub id: String,
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

impl Axiom {
    /// Short human-readable label for the axiom, used by display surfaces
    /// (e.g. `project_style` summary) that show one axiom per line and
    /// don't need the full prompt injection. Today this is just the
    /// `rule_summary`; if a future schema version adds a distinct `title`,
    /// callers gain richer output without changing.
    #[must_use]
    pub fn title_or_summary(&self) -> &str {
        &self.rule_summary
    }
}

impl From<RawAxiom> for Axiom {
    fn from(raw: RawAxiom) -> Self {
        let category_lower = raw.category.to_lowercase();
        let triggers_lower = raw.triggers.iter().map(|t| t.to_lowercase()).collect();
        let rule_summary_lower = raw.rule_summary.to_lowercase();
        Self {
            id: raw.id,
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

/// Test-only handle to the parsed axiom corpus, used by sibling tool tests
/// that cross-reference axioms by stable `id` (e.g. the exemplars manifest).
/// Cross-module tests cannot reach the private [`axioms`] cache, so this
/// `pub(crate)` helper exposes the same slice without widening the runtime
/// API. Panic on parse failure is acceptable here because the same parse
/// already runs in [`handle_axioms`] in production paths.
#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "test helper; parse failure aborts the test run intentionally"
)]
pub(crate) fn axioms_for_test() -> &'static [Axiom] {
    axioms().expect("axioms parse for tests")
}

/// Production-time cross-module accessor for the parsed universal corpus.
///
/// Used by `project_style::init` to validate that override IDs resolve.
/// Returns `Err` instead of panicking so a malformed bundled corpus
/// doesn't bring down server startup; the caller is expected to log and
/// proceed. The same parse-once cache as [`handle_axioms`] backs this.
pub(crate) fn axioms_for_runtime() -> Result<&'static [Axiom], crate::IlluError> {
    axioms()
}

/// Public entry point used by the `mcp__illu__axioms` tool. Reads the
/// process-global [`ProjectStyle`] populated at server startup and forwards
/// to [`handle_axioms_with_style`]. The cached style is empty by default,
/// so callers running before [`init`](crate::server::tools::project_style::init)
/// (or with no `.illu/style/project.json`) observe the universal-corpus
/// behavior unchanged from Phase 2.
///
/// Threads `None` for `max_tokens` so behavior matches prior phases — the
/// MCP tool handler in `src/server/mod.rs` calls
/// [`handle_axioms_with_style`] directly when it needs to forward a
/// caller-supplied budget. This wrapper remains as a backward-compatible
/// API surface for `src/api.rs` consumers.
pub fn handle_axioms(query: &str) -> Result<String, crate::IlluError> {
    let style = crate::server::tools::project_style::project_style();
    handle_axioms_with_style(query, style, None)
}

/// Score one axiom against the (already-lowercased) query. Extracted from
/// the closure inside [`handle_axioms_with_style`] so the override-aware
/// scorer reads as: compute base, then ask the project style what to do
/// with it. Behavior of the base formula is preserved verbatim from the
/// Phase-2 scorer: category-substring (+5/term), trigger-substring
/// (+10/term), summary-substring (+2/term), then exact-match boosts
/// (+20 category, +30 trigger).
fn score_axiom(axiom: &Axiom, query_terms: &[&str], query_lower: &str) -> usize {
    let mut score = 0_usize;
    for term in query_terms {
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
        if trigger == query_lower {
            score += 30;
        }
    }
    score
}

/// Approximate token cost of rendering one axiom result block.
///
/// Heuristic: `ceil(bytes / 4)` — for English markdown a real tokenizer
/// (GPT-4-style BPE, Claude's tokenizer) averages ~3.5 chars per token.
/// We use 4 and round up so the estimator under-fills the budget rather
/// than over-spending. A fixed `MARKDOWN_OVERHEAD_BYTES` covers the
/// per-axiom rendering chrome (headings, source line, separator).
///
/// This is intentionally not a real tokenizer call — adding `tiktoken-rs`
/// or `tokenizers` would pull in megabytes of vocabulary data for ~10%
/// accuracy improvement that callers can clamp downstream if they need.
fn estimate_tokens(axiom: &Axiom) -> usize {
    const MARKDOWN_OVERHEAD_BYTES: usize = 80;
    let body_bytes = axiom.category.len()
        + axiom.rule_summary.len()
        + axiom.prompt_injection.len()
        + axiom.anti_pattern.len()
        + axiom.good_pattern.len()
        + axiom.source.as_ref().map_or(0, String::len);
    (body_bytes + MARKDOWN_OVERHEAD_BYTES).div_ceil(4)
}

/// Score the universal corpus *and* the project's local axioms against
/// `query`, applying [`ProjectStyle`] overrides per axiom. The split from
/// [`handle_axioms`] exists so cross-module tests can construct a
/// `ProjectStyle` and exercise scoring without mutating the OnceLock-cached
/// style set at server startup.
///
/// Override semantics:
/// - `Ignored` → `adjust_score` returns `None`, the axiom is filtered out.
/// - `Demoted` / `Elevated` → score is halved / doubled before sort.
/// - `Noted` → score is unchanged; the project's `note` is appended to
///   the per-axiom result block as a `**Project note:** ...` line.
/// - Missing override → identity.
///
/// The score-zero filter still applies *after* override adjustment, so a
/// non-matching axiom (base score 0) cannot be conjured into the result
/// set by `Demoted`/`Elevated` (multiplied to 0), which preserves the
/// "no match → no display" invariant.
pub(crate) fn handle_axioms_with_style(
    query: &str,
    style: &crate::server::tools::project_style::ProjectStyle,
    max_tokens: Option<u32>,
) -> Result<String, crate::IlluError> {
    let universal = axioms()?;

    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    // Iterate the universal corpus and the project's local axioms in one
    // pass. `chain` avoids materialising a combined `Vec`; the local slice
    // borrows from `style` and the static slice from the cache, so the
    // resulting `&Axiom` items live for the shorter of the two — bounded
    // by `style`'s lifetime, which is fine for the immediate `.collect()`.
    let mut scored: Vec<(&Axiom, usize)> = universal
        .iter()
        .chain(style.local_axioms.iter())
        .filter_map(|axiom| {
            let base = score_axiom(axiom, &query_terms, &query_lower);
            // `Ignored` filters via `None`; other severities (or no
            // override) yield `Some(adjusted)`. The score>0 filter below
            // catches the case where adjusting (or just no-matching) lands
            // at zero, including `Demoted` of a no-match axiom.
            let adjusted = style.adjust_score(&axiom.id, base)?;
            (adjusted > 0).then_some((axiom, adjusted))
        })
        .collect();

    scored.sort_by_key(|&(_, score)| Reverse(score));

    // Capture the top-scored axiom (if any) before the budget walk consumes
    // `scored`. Used by the budget-too-small diagnostic to name the cost
    // floor: "smallest matching axiom estimated at ~N tokens" really means
    // the first (highest-scoring) matching axiom — that defines the minimum
    // cost a caller must afford to receive *anything* in this score order.
    let top_match: Option<&Axiom> = scored.first().map(|&(a, _)| a);

    // Keep enough matches for the broad baseline quality query to include
    // both the project-specific design axioms and the stricter Rust API
    // axioms, while still keeping MCP responses short enough to read. When
    // a caller supplies `max_tokens`, walk the score-ordered candidates
    // and stop before cumulative estimated cost would exceed the budget;
    // the `MAX_AXIOM_RESULTS` cap still applies — whichever bound trips
    // first wins.
    let top: Vec<&Axiom> = match max_tokens {
        None => scored
            .into_iter()
            .take(MAX_AXIOM_RESULTS)
            .map(|(a, _)| a)
            .collect(),
        Some(budget) => {
            let budget = budget as usize;
            let mut spent: usize = 0;
            let mut chosen = Vec::new();
            for (axiom, _) in scored {
                let cost = estimate_tokens(axiom);
                // Strict: stop on the first axiom that would push us past
                // the budget rather than skipping it for a cheaper later
                // entry. Score-order priority is more useful to callers
                // than maximizing fill of the budget window.
                if spent.saturating_add(cost) > budget {
                    break;
                }
                spent = spent.saturating_add(cost);
                chosen.push(axiom);
                if chosen.len() >= MAX_AXIOM_RESULTS {
                    break;
                }
            }
            chosen
        }
    };

    let mut output = String::new();
    let _ = writeln!(output, "## Rust Axioms matching '{query}'\n");

    if top.is_empty() {
        // Distinguish "no matches at all" from "matches existed but the
        // budget couldn't fit any of them". The latter names the budget
        // and the cost floor so the caller can raise `max_tokens`.
        if let (Some(budget), Some(axiom)) = (max_tokens, top_match) {
            let cost = estimate_tokens(axiom);
            let _ = writeln!(
                output,
                "No matching axioms fit within max_tokens={budget} (smallest matching axiom estimated at ~{cost} tokens)."
            );
            return Ok(output);
        }
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
        // Surface the project's `Noted` note. Other severities encode
        // their effect through ranking or filtering already; only `Noted`
        // exists *to* render text.
        if let Some(note) = style.note_for(&axiom.id) {
            let _ = writeln!(output, "**Project note:** {note}\n");
        }

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
        // Surface the project-local axiom's id so a reader can correlate
        // results back to `mcp__illu__project_style` output.
        if axiom.id.starts_with("project_") {
            let _ = writeln!(output, "_Project-local axiom: `{}`_", axiom.id);
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

    // --- Phase 3 Task 2: ProjectStyle integration tests --------------------
    //
    // These exercise `handle_axioms_with_style` directly with a constructed
    // `ProjectStyle` rather than the OnceLock-cached one set at server
    // startup, so test outcomes do not depend on the host repo's
    // `.illu/style/project.json` (or its absence).

    #[test]
    fn test_handle_axioms_respects_ignored() {
        use crate::server::tools::project_style::{AxiomOverride, ProjectStyle, Severity};
        let mut style = ProjectStyle::default();
        style.overrides.insert(
            "rust_quality_57_error_source_chain".into(),
            AxiomOverride {
                severity: Severity::Ignored,
                note: String::new(),
            },
        );
        // Differential assertion: run the same query with and without the
        // override and assert the axiom's category heading appears in
        // the unfiltered result but NOT in the filtered one. The
        // formatter does not emit literal axiom IDs for universal
        // axioms, so asserting on the heading anchor `[Error Source
        // Chain]` (unique to axiom 57) is the only way to detect
        // whether `Ignored` actually fired.
        let query = "Error::source error chain source method wrapped error";
        let unfiltered = handle_axioms_with_style(query, &ProjectStyle::default(), None).unwrap();
        let filtered = handle_axioms_with_style(query, &style, None).unwrap();
        assert!(
            unfiltered.contains("[Error Source Chain]"),
            "sanity: axiom 57 should match this query without overrides; got: {unfiltered}"
        );
        assert!(
            !filtered.contains("[Error Source Chain]"),
            "ignored axiom 57 must not appear with the override; got: {filtered}"
        );
    }

    #[test]
    fn test_handle_axioms_respects_demoted_elevated() {
        use crate::server::tools::project_style::{AxiomOverride, ProjectStyle, Severity};
        let mut style = ProjectStyle::default();
        style.overrides.insert(
            "rust_quality_85_allocation_hot_paths".into(),
            AxiomOverride {
                severity: Severity::Demoted,
                note: String::new(),
            },
        );
        style.overrides.insert(
            "rust_quality_87_iterator_codegen".into(),
            AxiomOverride {
                severity: Severity::Elevated,
                note: String::new(),
            },
        );
        // Query that hits both axioms. Anchor on `[Allocation Discipline]`
        // and `[Iterator Codegen]` — both unique substrings in the corpus
        // (single occurrence each), so we don't false-match other
        // allocation-flavored or iterator-flavored axioms. With overrides
        // applied, 87 (elevated) should rank above 85 (demoted).
        let result = handle_axioms_with_style(
            "allocation iterator preallocate hot path with_capacity bounds check",
            &style,
            None,
        )
        .unwrap();
        assert!(
            result.contains("[Allocation Discipline]"),
            "axiom 85 must appear in result for this query; got: {result}"
        );
        assert!(
            result.contains("[Iterator Codegen]"),
            "axiom 87 must appear in result for this query; got: {result}"
        );
        let pos_85 = result.find("[Allocation Discipline]").unwrap();
        let pos_87 = result.find("[Iterator Codegen]").unwrap();
        assert!(
            pos_87 < pos_85,
            "elevated axiom 87 should appear before demoted axiom 85; result: {result}"
        );
    }

    #[test]
    fn test_handle_axioms_appends_noted() {
        use crate::server::tools::project_style::{AxiomOverride, ProjectStyle, Severity};
        let mut style = ProjectStyle::default();
        style.overrides.insert(
            "rust_quality_85_allocation_hot_paths".into(),
            AxiomOverride {
                severity: Severity::Noted,
                note: "PROJECT-NOTE-MARKER-85".into(),
            },
        );
        let result = handle_axioms_with_style(
            "allocation hot path preallocate with_capacity",
            &style,
            None,
        )
        .unwrap();
        assert!(
            result.contains("PROJECT-NOTE-MARKER-85"),
            "noted axiom's note must appear in result, got: {result}"
        );
        assert!(
            result.contains("**Project note:**"),
            "noted axiom must use the `Project note` label, got: {result}"
        );
    }

    #[test]
    fn test_handle_axioms_surfaces_local_axiom() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/illu_style_sample/.illu/style/project.json");
        let style = crate::server::tools::project_style::load_from_path(&fixture).unwrap();
        let result =
            handle_axioms_with_style("repository module database access", &style, None).unwrap();
        assert!(
            result.contains("project_repository_pattern"),
            "project-local axiom must surface for matching query, got: {result}"
        );
    }

    // --- Phase 6 Task 1: token-budget tests --------------------------------
    //
    // The estimator is intentionally a heuristic (`ceil(bytes / 4)` plus a
    // fixed `MARKDOWN_OVERHEAD_BYTES`); these tests assert the contract
    // (no-budget unchanged, budget respected, zero-budget diagnostic,
    // score-order truncation, estimator sanity) rather than exact token
    // counts that would couple to a specific tokenizer.

    #[test]
    fn test_axioms_max_tokens_none_unchanged() {
        // No-budget code path is byte-for-byte identical to the prior
        // implementation. Run twice to confirm determinism, and verify
        // the result is non-empty for a representative query.
        use crate::server::tools::project_style::ProjectStyle;
        let style = ProjectStyle::default();
        let a = handle_axioms_with_style("error handling", &style, None).unwrap();
        let b = handle_axioms_with_style("error handling", &style, None).unwrap();
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn test_axioms_respects_max_tokens() {
        // Budget mode: cumulative estimated cost of returned axioms must
        // not exceed the budget. We test against the estimator's own
        // definition, not an external tokenizer.
        use crate::server::tools::project_style::ProjectStyle;
        let style = ProjectStyle::default();
        let budget: u32 = 2000;
        let result = handle_axioms_with_style("error handling", &style, Some(budget)).unwrap();
        // The result must be non-empty for this broad query at 2k tokens.
        assert!(!result.is_empty());
        // Sanity: the rendered response itself should be ≤ ~budget * 4
        // bytes (the estimator's inverse). Allow 50% slop for response
        // chrome (top heading, separator) we don't account for in
        // estimate_tokens.
        let max_bytes = (budget as usize) * 4 * 3 / 2;
        assert!(
            result.len() <= max_bytes,
            "response length {} exceeded loose budget bound {}",
            result.len(),
            max_bytes
        );
    }

    #[test]
    fn test_axioms_max_tokens_zero_returns_empty_with_diagnostic() {
        use crate::server::tools::project_style::ProjectStyle;
        let style = ProjectStyle::default();
        let result = handle_axioms_with_style("error handling", &style, Some(0)).unwrap();
        assert!(
            result.contains("max_tokens=0"),
            "zero-budget result must surface the diagnostic naming the budget; got: {result}"
        );
        assert!(
            !result.contains("[Error"),
            "no axiom headings should appear in zero-budget result; got: {result}"
        );
    }

    #[test]
    fn test_axioms_max_tokens_truncates_in_score_order() {
        // Compare a no-budget result with a tight-budget result for the
        // same query: the budgeted result's axioms must all appear in the
        // no-budget result's prefix (highest-scoring first).
        use crate::server::tools::project_style::ProjectStyle;
        let style = ProjectStyle::default();
        let unbounded = handle_axioms_with_style("error handling", &style, None).unwrap();
        let bounded = handle_axioms_with_style("error handling", &style, Some(800)).unwrap();
        // Extract category headings (lines starting with "### [") in order
        // from each result. Bounded headings must be a prefix of unbounded.
        let extract = |s: &str| -> Vec<String> {
            s.lines()
                .filter(|l| l.starts_with("### ["))
                .map(String::from)
                .collect()
        };
        let bounded_headings = extract(&bounded);
        let unbounded_headings = extract(&unbounded);
        assert!(
            !bounded_headings.is_empty(),
            "bounded result should have at least one axiom for this query"
        );
        for (i, h) in bounded_headings.iter().enumerate() {
            assert_eq!(
                unbounded_headings.get(i),
                Some(h),
                "budget walk must pick top-scored axioms in order; mismatch at index {i}"
            );
        }
    }

    #[test]
    fn test_estimate_tokens_is_reasonable() {
        // Sanity: estimate_tokens for a real axiom is within ±50% of
        // bytes / 4 of its full body. The constant chrome overhead pushes
        // the estimator slightly higher than a pure-body estimate, which
        // is the conservative direction.
        let universal = axioms_for_test();
        let axiom = universal.first().unwrap();
        let body_bytes = axiom.category.len()
            + axiom.rule_summary.len()
            + axiom.prompt_injection.len()
            + axiom.anti_pattern.len()
            + axiom.good_pattern.len()
            + axiom.source.as_ref().map_or(0, String::len);
        let estimated = estimate_tokens(axiom);
        let body_tokens = body_bytes.div_ceil(4);
        let lower = body_tokens / 2;
        let upper = body_tokens * 2;
        assert!(
            estimated >= lower && estimated <= upper,
            "estimate {estimated} should be within ±50% of body-only {body_tokens}"
        );
    }
}
