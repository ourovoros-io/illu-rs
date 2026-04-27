# Jon-Style Rust — Phase 0 Design

**Date:** 2026-04-26
**Status:** Draft, pending user review
**Scope:** Phase 0 (preparation) of a holistic system to push illu-driven Rust generation toward the style of Jon Gjengset (author of *Rust for Rustaceans*).

## Motivation

Models writing Rust tend to produce code that has the *shape* of Rust without the *reasoning* behind it: clone-heavy ownership, runtime checks where the type system would do, `Box<dyn Error>` where a real enum belongs, allocations in hot paths, generics where a concrete type would be clearer. The owner of this repo is mentored by Jon Gjengset and wants the code illu helps produce to reflect his style — type-driven design, deliberate ownership, structured error types, performance discipline, comments that explain *why* and *what invariant*.

illu already ships strong axioms; many cite "Towards Impeccable Rust" (Jon's talk) and `thesquareplanet.com` (his site) directly. The work is gap-filling and enrichment, not greenfield construction.

This document specifies **Phase 0** only. Later phases are sketched at the end.

## Goal

Land 5–10 new error-handling axioms in the existing axioms pipeline, each user-reviewed, source-cited, and surfaced via `mcp__illu__axioms` and `rust_preflight`. Use the result as a working demo that Jon-style guidance flows into Rust generation end-to-end.

## Scope

**In scope (Phase 0):**
- Gap analysis on existing error-handling axioms vs. *Rust for Rustaceans* Ch. 4 and Jon's public repos.
- Drafting and user review of new axioms, vertical-sliced to error handling only.
- Adding accepted axioms to `assets/rust_quality_axioms.json`.
- Test additions covering the new entries' parse correctness and queryability.
- One end-to-end demo: a sample fallible function written via `rust_preflight` whose evidence packet cites the new axioms.

**Explicit non-goals (Phase 0):**
- No new MCP tools.
- No schema changes to the `Axiom` struct.
- No `.illu/style/` directory or filesystem-loaded axioms.
- No exemplars infrastructure.
- No project-context infrastructure.
- No `quality_gate` integration changes.
- No axioms outside error handling.

## Architecture

Three layers, all already in place:

1. **Source layer** — JSON files in `assets/`, `include_str!`'d into the binary. Phase 0 adds entries to `assets/rust_quality_axioms.json` (the source-cited bucket; `axioms.json` is the older general-purpose set).
2. **Index layer** — `axioms()` at [src/server/tools/axioms.rs:67-87](src/server/tools/axioms.rs:67) parses both files once into `&'static [Axiom]` and caches forever. No DB, no filesystem indexer pass.
3. **Consumption layer** — `mcp__illu__axioms` calls `handle_axioms()` at [src/server/tools/axioms.rs:89-176](src/server/tools/axioms.rs:89) for in-memory scoring. `rust_preflight` calls `render_axioms()` at [src/server/tools/rust_preflight.rs:73-79](src/server/tools/rust_preflight.rs:73) which delegates to the same path. New axioms surface to all consumers without code changes outside the JSON file.

**Why no new infrastructure.** The original design proposed `.illu/style/` files mirrored into new SQLite tables. Investigation of the existing pipeline showed: axioms are baked into the binary, parsed once at startup, searched in-memory. Adding a third source file is a known shape (already two sources). DB mirroring solves a problem (slow structured queries) the existing pipeline does not have. Deferring `.illu/style/` to Phase 2+ keeps Phase 0 to content-only enrichment with zero new code paths to validate.

## Schema

New entries match the `Axiom` struct at [src/server/tools/axioms.rs:27-38](src/server/tools/axioms.rs:27) exactly:

```json
{
  "category": "Error Source Chain",
  "source": "Rust for Rustaceans, ch. 4 §Error Source",
  "triggers": ["error source", "Error::source", "error chain", "wrapped error", "underlying cause"],
  "rule_summary": "Implement Error::source() so callers can walk the cause chain",
  "prompt_injection": "MANDATORY RULE: Library error variants that wrap an underlying error must implement source() to expose the cause. Do not flatten the cause into a Display string.",
  "good_pattern": "<Rust snippet>",
  "anti_pattern": "<Rust snippet>"
}
```

**Scoring relevance.** `handle_axioms()` weights matches as: trigger contains term +10, exact-trigger match +30, category contains term +5, exact-category match +20, rule_summary contains term +2. Triggers are the single most important field for findability.

**Trigger guidance for new entries:**
- 4–6 triggers per axiom.
- Mix of bare nouns (`"backtrace"`), method-form (`"Error::source"`), domain words (`"variant naming"`, `"error chain"`), and phrase forms a user might type.
- Prefer specific over generic — avoid `"error"`, `"library"`, `"rust"` alone.

## Candidate Axiom List

Final list iterated during drafting. Some may collapse into existing axioms or merge with each other.

1. **Error source chain** — `Error::source()` discipline; wrapped errors expose the cause through `source()`, not by flattening into `Display`.
2. **Wrap vs propagate** — when `?` (preserve original) vs `map_err` (add boundary context); avoid both at the same call site without justification.
3. **Variant naming and Display conventions** — variant names are nouns naming the failure (`InvalidUtf8`, not `FailedToParseUtf8`); `Display` is one short clause, no trailing punctuation, no leaked internal state.
4. **Error category structure** — distinguish I/O / parse / domain / invariant violations as distinct variants or sub-enums; never collapse into `Other(String)`.
5. **Backtrace policy** — when to capture `std::backtrace::Backtrace`; library policy (only at boundaries that lose stack context) vs application policy (closer to where errors originate).
6. **Stable error semantics** — `#[non_exhaustive]` is necessary but not sufficient; document which variants are stable contracts and which are internal-detail open to revision.
7. **Error context as values, not strings** — typed context fields on the variant over `format!`-built strings; reserve `eyre`-style chained context for application code.
8. **No `Box<dyn Error>` even internally in libraries** — strong form of the existing axiom; covers helper functions where the temptation is highest.
9. **`From` impls are public API** — every `From` for an error widens the conversion graph; review each one as a deliberate public-surface decision.
10. **Test the failure path's variant, not just `is_err()`** — strengthens the existing error-path axiom by requiring `assert!(matches!(...))` discipline.

## Drafting and Review Loop

Per the pipeline picked during brainstorming (assistant drafts, user reviews):

1. **Per axiom**: assistant drafts category, triggers, rule_summary, prompt_injection, good_pattern, anti_pattern, source.
2. **Batch**: 2–3 axioms per batch.
3. **Review**: user keeps / rejects / edits / merges with existing entries.
4. **Commit**: one commit per accepted batch, so each batch is independently reviewable and revertible.
5. **Source citations**: placeholder format `"Rust for Rustaceans, ch. 4 §<topic>"` if exact section unknown; user tightens during review.

**Inputs the user supplies:**
- Final approval on which file the new axioms land in (lean: `assets/rust_quality_axioms.json`).
- Optionally: Jon's repos cloned somewhere readable so good_pattern snippets can quote his actual code with a `repo:path:line` source field. Without that, drafted snippets are synthetic in his style.
- Optionally: book chapter material — paraphrased section content or page numbers for tight citations. Without that, citations stay at chapter granularity.

## Verification and Exit Criteria

Phase 0 is complete when **all** of the following hold:

- 5–10 new error-handling axioms drafted, reviewed, and merged into `assets/rust_quality_axioms.json`.
- All existing tests pass.
- One new test extends [test_rust_quality_axioms_are_loaded_with_sources](src/server/tools/axioms.rs:211-215) to assert the category coverage of new entries.
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.
- Demo gate: a representative `mcp__illu__axioms` query — `"error source chain wrap propagate variant naming"` — returns at least 3 of the new axioms in the top results.
- End-to-end demo: assistant writes a sample fallible function via `rust_preflight`; the evidence packet surfaces at least one new axiom; the resulting code reflects the guidance.

## Risks and Mitigations

- **Drafted axioms don't match Jon's actual reasoning.** Mitigation: review-by-batch with reject-or-merge as first-class outcomes; do not negotiate borderline drafts into shape.
- **New axioms dilute search relevance for existing queries.** Mitigation: specific triggers (`"Error::source"`, `"backtrace"`, `"variant naming"`) over generic ones; review trigger overlap against existing entries.
- **Source citations imprecise.** Mitigation: placeholder format with chapter granularity; user tightens during review.
- **Scope creep into other style areas during drafting.** Mitigation: explicit non-goals; ownership / type-system / performance axiom gaps stay in the backlog until Phase 1.
- **Schema mismatch breaks the loader.** Mitigation: extend the existing parse-once-at-startup test; the failure mode is loud (startup panic during `axioms()`), not silent.
- **Trigger collisions with existing axioms surface unrelated entries.** Mitigation: during drafting, query existing axioms for each new trigger word and adjust if dilution would occur.

## Phase 1+ Outline

Sketched only; deepened when its turn comes.

- **Phase 1** — replicate the slice horizontally: ownership/lifetime gap-fill, then type-system, then performance, then unsafe / FFI. Same pipeline, no new infrastructure.
- **Phase 2** — `.illu/style/exemplars.toml` plus a new `mcp__illu__exemplars(intent)` tool. Symbol references with one-line commentary; Jon's repos provide the corpus.
- **Phase 3** — `.illu/style/project_context.md` consumed by `rust_preflight`; structured sections for hot paths, allocation policy, unsafe policy.
- **Phase 4+** — review-layer tools: `design_record`, `critique`, `cost_profile`. Each consumes Phases 0–3 artifacts.
- **Phase N** — `quality_gate` integration: tripwires, design-record cross-check, exemplar-anchoring assertions.

## Open Questions for User Review

- Confirm `assets/rust_quality_axioms.json` as the destination file.
- Confirm whether Jon's repos are available locally for `repo:path:line` citation; if not, synthetic-but-faithful snippets are the fallback.
- Confirm whether you want batch sizes of 2 or 3 axioms per review round (default: 3).
- Any candidate axioms to drop or merge before drafting begins.
