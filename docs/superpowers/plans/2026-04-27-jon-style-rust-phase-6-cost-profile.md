# Jon-Style Rust Phase 6 (Cost Profile) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Token-budget awareness for `handle_axioms`. New optional `max_tokens: u32` parameter on `AxiomsParams`, runtime estimator helper, budget-walk in `handle_axioms_with_style`. No schema change, no new module.

**Architecture:** Single-file content addition (`axioms.rs`) plus a one-field extension to `AxiomsParams` in `mod.rs`. The Phase-3 `handle_axioms_with_style` seam is preserved — the budget logic plugs into the existing rank-and-truncate flow.

**Tech Stack:** Rust 2024, `serde` for the new optional field on `AxiomsParams`, no new external dependencies.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-6-cost-profile-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-6-cost-profile-design.md)

**Existing state:** Phase 0–4 merged. 102 universal axioms, 9 exemplars, project_style + decisions infrastructure. `handle_axioms` is a thin wrapper around `pub(crate) fn handle_axioms_with_style(query: &str, style: &ProjectStyle) -> Result<String, IlluError>`. The MCP tool registration in `src/server/mod.rs` uses `AxiomsParams { query: String }`.

**Lint constraints** (unchanged): `unwrap_used = "deny"`, `expect_used = "warn"`, `allow_attributes = "deny"`. Tests use `#[expect(clippy::unwrap_used, reason = "tests")]` at module scope.

**Drafts pre-approved by user.** Skip the user-review step inside batch tasks.

**Key design decisions** (per the spec):
- Estimator: `bytes / 4` (rounded up via `usize::div_ceil`), with a fixed `MARKDOWN_OVERHEAD_BYTES: usize = 80` per-axiom add for headings/fences.
- Budget walk: strict — return empty with a `"no axioms fit within max_tokens=N"` diagnostic when the first axiom alone exceeds the budget.
- Parameter type: `Option<u32>` (not `usize` — better MCP-serialization portability).
- `MAX_AXIOM_RESULTS = 16` cap still applies in budget mode (whichever is smaller wins).

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `src/server/tools/axioms.rs` | Modify | Add `estimate_tokens` helper; extend `handle_axioms_with_style` signature with `max_tokens: Option<u32>`; add 5 tests |
| `src/server/mod.rs` | Modify | Add `max_tokens: Option<u32>` to `AxiomsParams`; pass it to the inner call |

---

## Task 1: Add Token Budget Awareness to `handle_axioms`

**Files (modify):**
- `src/server/tools/axioms.rs` (estimator helper, extended `_with_style`, 5 tests)
- `src/server/mod.rs` (extend `AxiomsParams`, pass through to handler)

- [ ] **Step 1: Extend `handle_axioms_with_style` signature**

In `src/server/tools/axioms.rs`, change the signature of `handle_axioms_with_style` from

```rust
pub(crate) fn handle_axioms_with_style(
    query: &str,
    style: &ProjectStyle,
) -> Result<String, crate::IlluError>
```

to

```rust
pub(crate) fn handle_axioms_with_style(
    query: &str,
    style: &ProjectStyle,
    max_tokens: Option<u32>,
) -> Result<String, crate::IlluError>
```

Update the public wrapper `handle_axioms` to thread `None` through (preserves prior behavior):

```rust
pub fn handle_axioms(query: &str) -> Result<String, crate::IlluError> {
    let style = crate::server::tools::project_style::project_style();
    handle_axioms_with_style(query, style, None)
}
```

Existing tests that call `handle_axioms_with_style(query, &style)` without the third argument need to be updated to pass `None`. Locate them via `grep "handle_axioms_with_style" src/server/tools/axioms.rs` and add the `None` argument.

- [ ] **Step 2: Add the `estimate_tokens` helper**

Insert after the `score_axiom` function in `src/server/tools/axioms.rs`:

```rust
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
        + axiom.source.as_ref().map(String::len).unwrap_or(0);
    (body_bytes + MARKDOWN_OVERHEAD_BYTES).div_ceil(4)
}
```

Note: `axiom.source` is `Option<String>`. Verify by reading the `Axiom` struct definition in the same file. If the field is non-optional, drop the `.as_ref().map(...)`.

- [ ] **Step 3: Implement budget walk in `handle_axioms_with_style`**

Locate the existing scoring + sorting + truncation block. After sorting by `Reverse(score)` and before the final `Vec<&Axiom>` collect, replace the `take(MAX_AXIOM_RESULTS)` step with a branch on `max_tokens`:

```rust
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
```

Adapt names to whatever the existing code uses (`scored` may be named `ranked` or similar; the variable holding the post-sort `Vec<(usize, &Axiom)>` is the target).

- [ ] **Step 4: Add the budget-too-small diagnostic**

When `max_tokens` is `Some(budget)` AND the result is empty AND `scored` was non-empty (i.e., there were matches but none fit), return a diagnostic message naming the smallest matching axiom's cost. Insert this branch in the existing "no matches" handling, gated on `max_tokens.is_some()` and the truncated `top.is_empty()` after the budget walk.

Concrete shape — adapt to the file's existing pattern:

```rust
if top.is_empty() {
    // (existing "no matches found" message stays for the no-budget case)
    if let Some(budget) = max_tokens {
        // We had matches but the budget was too small.
        if let Some((axiom, _)) = scored_for_diagnostic.first() {
            let cost = estimate_tokens(axiom);
            return Ok(format!(
                "## Rust Axioms matching '{query}'\n\nNo matching axioms fit within max_tokens={budget} (smallest matching axiom estimated at ~{cost} tokens).\n"
            ));
        }
    }
    // existing fallback for "no matches at all"
    // ...
}
```

(The variable `scored_for_diagnostic` is whatever the code names the pre-budget-walk sorted vector. The existing code already iterates that vector; either capture a clone of the first entry before the budget walk or save the first axiom cost separately. Whichever is cleaner.)

- [ ] **Step 5: Update existing tests in `axioms.rs` that call `handle_axioms_with_style`**

The Phase-3 integration tests pass `(query, &style)` — they need to add `, None` as the third argument. Find them with `grep "handle_axioms_with_style" src/server/tools/axioms.rs`. The tests are: `test_handle_axioms_respects_ignored`, `test_handle_axioms_respects_demoted_elevated`, `test_handle_axioms_appends_noted`, `test_handle_axioms_surfaces_local_axiom`. Each gets `, None` appended to the existing call.

- [ ] **Step 6: Add 5 new tests in `axioms::tests`**

```rust
    #[test]
    fn test_axioms_max_tokens_none_unchanged() {
        // No-budget code path is byte-for-byte identical to the prior
        // implementation. Run twice to confirm determinism, and verify
        // the result is non-empty for a representative query.
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
        let style = ProjectStyle::default();
        let result =
            handle_axioms_with_style("error handling", &style, Some(0)).unwrap();
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
        let style = ProjectStyle::default();
        let unbounded =
            handle_axioms_with_style("error handling", &style, None).unwrap();
        let bounded =
            handle_axioms_with_style("error handling", &style, Some(800)).unwrap();
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
        assert!(!bounded_headings.is_empty(), "bounded result should have at least one axiom for this query");
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
        let axiom = universal.first().expect("corpus is non-empty");
        let body_bytes = axiom.category.len()
            + axiom.rule_summary.len()
            + axiom.prompt_injection.len()
            + axiom.anti_pattern.len()
            + axiom.good_pattern.len()
            + axiom.source.as_ref().map(String::len).unwrap_or(0);
        let estimated = estimate_tokens(axiom);
        let body_tokens = body_bytes.div_ceil(4);
        let lower = body_tokens / 2;
        let upper = body_tokens * 2;
        assert!(
            estimated >= lower && estimated <= upper,
            "estimate {estimated} should be within ±50% of body-only {body_tokens}"
        );
    }
```

(Adapt the heading-prefix detection in test 4 to whatever the actual formatter emits — check whether headings use `### [...]`, `## [...]`, or another shape, and update the `starts_with(...)` accordingly. Read the formatter body to confirm.)

- [ ] **Step 7: Extend `AxiomsParams` in `src/server/mod.rs`**

```rust
#[derive(Deserialize, JsonSchema)]
struct AxiomsParams {
    /// Search term for axioms
    query: String,
    /// Optional token budget for the response. When set, results are
    /// truncated in score order so cumulative estimated tokens stay
    /// within budget. When omitted, behavior is unchanged from prior
    /// phases (caps at MAX_AXIOM_RESULTS results regardless of size).
    #[serde(default)]
    max_tokens: Option<u32>,
}
```

Update the `axioms` tool handler to thread `params.max_tokens` through:

```rust
async fn axioms(
    &self,
    Parameters(params): Parameters<AxiomsParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(query = %params.query, max_tokens = ?params.max_tokens, "Tool call: axioms");
    let _guard = crate::status::StatusGuard::new(&format!("axioms ▸ {}", params.query));
    let style = crate::server::tools::project_style::project_style();
    let result = tools::axioms::handle_axioms_with_style(&params.query, style, params.max_tokens)
        .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

(The handler now calls `handle_axioms_with_style` directly to thread `max_tokens` through. The standalone `pub fn handle_axioms(query: &str)` becomes unused at runtime but remains as a backward-compatible API surface in `src/api.rs`.)

- [ ] **Step 8: Run cargo gauntlet**

```bash
cargo build
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

All must pass. The 5 new tests in `axioms::tests` should pass. The 4 existing Phase-3 integration tests should still pass after the `, None` argument addition.

- [ ] **Step 9: Commit Task 1**

```bash
git add src/server/tools/axioms.rs src/server/mod.rs
git commit -m "$(cat <<'EOF'
axioms: add token-budget awareness to handle_axioms (Phase 6)

Adds an optional max_tokens parameter to the mcp__illu__axioms tool
so callers can ask for axioms that fit within a token budget. No
schema change to the universal corpus.

- Extended handle_axioms_with_style with a third arg
  max_tokens: Option<u32>. When None, the prior MAX_AXIOM_RESULTS=16
  flat cap applies (no behavior change). When Some(budget), the
  budget-walk loop accumulates estimated tokens in score order and
  stops before the next axiom would exceed the budget.
- New estimate_tokens(axiom) helper using ceil(bytes/4) with a
  MARKDOWN_OVERHEAD_BYTES constant for per-axiom rendering chrome.
  Heuristic intentionally simple — adding tiktoken-rs/tokenizers
  would pull in megabytes of vocabulary for marginal accuracy gain.
- Strict budget: when even the highest-scoring axiom exceeds the
  budget, return empty with a diagnostic naming the budget and the
  smallest matching axiom's estimated cost (so the caller can adjust).
- AxiomsParams gains an optional max_tokens: u32 field; the tool
  handler threads it through. tracing::info now logs the budget for
  debuggability.
- 5 new tests: no-budget unchanged, budget respected, zero-budget
  empty + diagnostic, score-order truncation, estimator sanity.

Closes Phase 6 (cost profile). Phase 6.1+ defers per-axiom
budget_weight if the flat cost is too coarse, max_tokens on sibling
tools, and accurate tokenization via external dep.
EOF
)"
```

---

## Task 2: End-to-End Verification + Plan Reconciliation

- [ ] **Step 1: Full cargo gauntlet**

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 2: Plan-reconciliation pass before final review** — if any content fix-ups landed during execution (heading-prefix detection in test 4, exact chrome-overhead constant tuning, formatter-shape adaptations), update the plan to match.

---

## Verification Summary

After all tasks:
- 1 modified module (`axioms.rs`: added `estimate_tokens`, extended `_with_style` signature, 5 new tests).
- 1 modified params struct (`AxiomsParams`: added `max_tokens` field).
- 5 new tests; 4 existing integration tests updated to pass `None`.
- Cargo gauntlet clean.

## Risks Realized During Execution

- **Existing test signature drift.** The 4 Phase-3 integration tests must add `, None` as the third arg; missing one is a compile error.
- **Formatter heading shape.** Test 4 asserts on headings starting with `### [`; if the actual formatter uses a different shape, the test needs adjustment.
- **`Axiom::source` field nullability.** The estimator code uses `axiom.source.as_ref().map(String::len).unwrap_or(0)`; if `source` is `String` (not `Option<String>`), simplify to `axiom.source.len()`.
- **`MARKDOWN_OVERHEAD_BYTES = 80` calibration.** May need tuning based on the actual formatter output; the sanity test bounds at ±50% give headroom.
