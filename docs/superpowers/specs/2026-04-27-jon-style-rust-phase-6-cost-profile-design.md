# Jon-Style Rust — Phase 6 (Cost Profile) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 6 — token-budget awareness for `handle_axioms`. New optional `max_tokens` parameter on the `axioms` MCP tool, runtime estimator, no schema change.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md) for overall architecture; [Phase 3 spec](2026-04-27-jon-style-rust-phase-3-project-context-design.md) for the `handle_axioms_with_style` extraction pattern that this slice extends.

## Motivation

Today the `mcp__illu__axioms` tool truncates with a flat cap (`MAX_AXIOM_RESULTS = 16`). An agent in a long conversation with a tight remaining context budget cannot ask "give me axioms within 5000 tokens" — it gets up to 16 results regardless of size, and either has to manually trim downstream or over-spend the budget.

Phase 6 adds a token-budget mode: the caller passes an optional `max_tokens: u32` parameter, the server estimates each axiom's render cost from byte length, sorts by score, and returns axioms in order until the next one would exceed the budget. When `max_tokens` is omitted, behavior is identical to Phase 5/today.

This is intentionally the lightest of the remaining phases. No JSON schema change, no per-axiom weight authoring; one estimator, one query parameter, one budget-walk loop.

## Goal

1. Add an optional `max_tokens: Option<u32>` field to `AxiomsParams` (the MCP tool's parameter struct).
2. Add a token estimator that returns approximate token cost for an `Axiom`'s rendered markdown form.
3. Modify `handle_axioms_with_style` to honor the budget when set: walk scored results in rank order, accumulate estimated tokens, stop before the budget would be exceeded.
4. When `max_tokens` is omitted (`None`), behavior is byte-for-byte identical to the current implementation (preserves the Phase 5 baseline).
5. Tests including budget-respecting, edge cases (0, smaller than smallest axiom), and an estimator sanity check.

## Scope

**In scope:**
- New optional `max_tokens` field on `AxiomsParams` in `src/server/mod.rs`.
- New helper `estimate_tokens(axiom: &Axiom) -> usize` in `src/server/tools/axioms.rs`.
- Modified `handle_axioms_with_style` to consult the budget when set.
- 5 tests in `axioms::tests`.

**Explicit non-goals:**
- No schema change to `assets/rust_quality_axioms.json`. The 102 axioms keep their current shape.
- No per-axiom `budget_weight` field. If usage shows the flat cost-per-axiom is too coarse, Phase 6.1 can add it.
- No equivalent budget mode for `exemplars`, `decisions`, or `project_style`. Those tools have smaller working sets and the flat caps are appropriate; if value emerges, follow-on phases can extend.
- No streaming truncation — the estimator runs over already-loaded `Axiom` data; nothing about the parse-once cache changes.
- No re-ranking by cost-effectiveness (`score / cost`). Strict score-order budget walk only. Cost-effectiveness reranking adds a knob nobody asked for and creates a new "why doesn't my high-score axiom show up?" debugging surface.

## Architecture

```
src/server/tools/axioms.rs           (modified — adds estimate_tokens + budget walk)
src/server/mod.rs                    (modified — AxiomsParams gains optional max_tokens)
```

No new module. No new asset directory. No new MCP tool.

**Estimator** (`estimate_tokens`):

```rust
/// Approximate token cost of rendering one axiom result block. The
/// estimate is a coarse byte-count heuristic — for English markdown,
/// real tokenizers produce roughly 1 token per 4 characters of UTF-8
/// (varies by tokenizer; GPT-4-style BPE averages ~3.5, Claude's
/// tokenizer is similar). We round up so budgets are conservative
/// (we may slightly under-fill, never over-spend).
fn estimate_tokens(axiom: &Axiom) -> usize {
    // Sum the byte counts of every rendered field; rough overhead for
    // markdown chrome (headings, separators) is folded in as a fixed
    // per-axiom add.
    let body_bytes = axiom.category.len()
        + axiom.rule_summary.len()
        + axiom.prompt_injection.len()
        + axiom.anti_pattern.len()
        + axiom.good_pattern.len()
        + axiom.source.as_ref().map(String::len).unwrap_or(0);
    const MARKDOWN_OVERHEAD_BYTES: usize = 80; // headings, fences, ID line
    let total_bytes = body_bytes + MARKDOWN_OVERHEAD_BYTES;
    // Round up: tokens = ceil(bytes / 4).
    total_bytes.div_ceil(4)
}
```

**Budget walk** (within `handle_axioms_with_style`, after sorting by score):

```rust
let max_tokens = max_tokens.map(usize::try_from).and_then(Result::ok);
// ...existing scoring + sort by Reverse(score)...
let top: Vec<&Axiom> = match max_tokens {
    None => scored.into_iter().take(MAX_AXIOM_RESULTS).map(|(a, _)| a).collect(),
    Some(budget) => {
        let mut spent = 0usize;
        let mut chosen = Vec::new();
        for (axiom, _) in scored {
            let cost = estimate_tokens(axiom);
            if spent.saturating_add(cost) > budget {
                break;
            }
            spent += cost;
            chosen.push(axiom);
            if chosen.len() >= MAX_AXIOM_RESULTS {
                break;
            }
        }
        chosen
    }
};
```

The `MAX_AXIOM_RESULTS` cap still applies in budget mode — it caps at whichever is smaller (count or token budget).

**Empty-result message** when budget is too small (the first axiom alone exceeds the budget): the existing "no matches" text is reused, but the diagnostic suffix changes:

```
No matching Rust Axioms fit within max_tokens=200 (smallest matching axiom estimated at ~340 tokens).
```

This makes "tightened the budget too far" diagnosable from the response alone.

## Schema

`AxiomsParams` gains one field:

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

`u32` is wide enough for any realistic budget (Claude 4.7 1M context = 1 million tokens; `u32::MAX` is 4 billion). Using `Option<u32>` rather than `0` as a sentinel keeps the omitted-vs-zero distinction clear.

## Tests

5 tests in `axioms::tests`:

1. `test_axioms_max_tokens_none_unchanged` — call `handle_axioms_with_style(query, &style)` (no budget) twice, confirm output is identical character-for-character. Also confirm a no-budget call returns a result count that matches the prior `MAX_AXIOM_RESULTS`-only behavior.

2. `test_axioms_respects_max_tokens` — call with `max_tokens = Some(2000)`. Compute the actual estimated token cost from the rendered response (use the same `estimate_tokens` on each axiom in the result). Assert cumulative cost ≤ 2000. (We're testing what we asked for: budget is respected per the estimator's own definition. We do not test against an external tokenizer.)

3. `test_axioms_max_tokens_zero_returns_empty` — `max_tokens = Some(0)` → empty result body, with the budget-too-small diagnostic message.

4. `test_axioms_max_tokens_truncates_in_score_order` — query that matches >5 axioms; budget tight enough to cut the result list to ≤3. Confirm the kept axioms are the highest-scoring (compare against the no-budget result's first 3 entries).

5. `test_estimate_tokens_is_reasonable` — sanity check on the estimator's heuristic: pick one axiom, compute `estimate_tokens(axiom)`, render it via the formatter, count actual bytes of the rendered string, divide by 4. Assert the estimator is within ±50% of `bytes / 4` (a coarse correctness band — the constant chrome overhead pushes the estimator slightly higher than a pure-body estimate would yield, which is the conservative direction).

## Verification and Exit Criteria

Phase 6 is complete when:
- `AxiomsParams` has the new optional `max_tokens` field.
- `handle_axioms_with_style` honors the budget when set, preserves prior behavior when not set.
- `estimate_tokens` helper exists with a doc comment naming the heuristic.
- 5 new tests pass.
- All existing tests still pass (especially the Phase-3 integration tests that anchor on the result format — none should break since the no-budget code path is identical).
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.
- Live MCP smoke test (after rebuild + restart): `mcp__illu__axioms` accepts the new parameter and respects it.

## Risks and Mitigations

- **Estimator accuracy.** The 4-bytes-per-token heuristic is coarse; real tokenizers vary by model. Mitigation: round up so we under-fill rather than over-spend; flag the heuristic in the helper's docstring; users requiring precise budgeting can apply their own clamp downstream.
- **Phase 5/Phase-3 integration tests reading specific axiom titles.** The existing tests anchor on category headings (`[Error Source Chain]`, `[Allocation Discipline]`, etc.) — they don't pass `max_tokens`, so they hit the unchanged code path. Risk minimal but explicitly preserved as a non-goal: the no-budget path is byte-identical.
- **MCP schema-discoverability.** `JsonSchema` derive should produce a documented optional field that the agent sees in tool descriptions. Mitigation: confirm in the live smoke test that the parameter shows up.
- **Budget too small for any axiom.** Mitigation: explicit empty-result diagnostic message naming the budget and the smallest matching axiom's estimated cost.
- **Plan drift between drafts and post-fix files** (recurring across all prior phases). Mitigation: explicit reconciliation pass before final review.

## Phase 6.1+ Continuation Outline

Out of scope for this slice; sketched only:
- Phase 6.1 — per-axiom `budget_weight` field if the flat cost-per-axiom turns out too coarse.
- Phase 6.2 — `max_tokens` parameter on `exemplars`/`decisions`/`project_style` if their working sets grow.
- Phase 6.3 — accurate tokenization via `tiktoken-rs` or `tokenizers` crate (adds dependency; currently overkill).

After Phase 6, the only remaining work from the original Phase 0 outline is Phase 5 (critique — axiom-violation detection in diffs).

## Open Questions for User Review

- Confirm Option A (estimator + query parameter, no schema change) is the chosen design.
- Confirm the strict-budget behavior (return empty with diagnostic when first axiom exceeds budget) vs. the "always return at least one" alternative.
- Confirm `u32` for the budget type (vs `usize` — `u32` is more portable across MCP serialization and capable enough for any realistic budget).
