# Jon-Style Rust — Phase 4 (Design Record) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 4 — `.illu/style/decisions/` as a project-local corpus of structured "we chose X over Y because Z" architectural decision records, surfaced via a new MCP tool.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md) for architecture inheritance; [Phase 2 spec](2026-04-27-jon-style-rust-phase-2-exemplars-design.md) for the exemplars-tool precedent; [Phase 3 spec](2026-04-27-jon-style-rust-phase-3-project-context-design.md) for the project-local trust model and discovery pattern.

## Motivation

Phase 0 + Phase 1 shipped 102 universal axioms (rules to follow). Phase 2 added 9 compile-checked exemplars (integrated patterns to imitate). Phase 3 added per-project axiom overrides (which rules apply locally). All three speak to *what code should look like*. None of them capture *why a project's architecture is the way it is* — the rationale behind past decisions.

Today, a fresh agent context (or a new developer) has no way to recover *"why did we use enum dispatch in the handlers module?"* or *"why was the original mutex-per-row scheme abandoned?"* from the code alone. Those reasons live in PR descriptions, Slack threads, and tribal memory — outside the agent's reach.

Phase 4 introduces a structured place to encode that knowledge: ADR-style decision records living at `{repo}/.illu/style/decisions/<slug>.json`, surfaced via a new `mcp__illu__decisions` MCP tool. Source material: ADR/MADR conventions; the prior phases' parse-once-cache + project-local-discovery pattern.

## Goal

1. Define a JSON schema for design records following the ADR pattern, validated at parse time.
2. Add a new MCP tool `mcp__illu__decisions` that scores user queries against the loaded corpus and returns matched records as Markdown.
3. Discovery and load mirror Phase 3: walk `{repo}/.illu/style/decisions/*.json` at server startup; absent directory → empty default → no decisions surfaced.
4. Cross-reference: each decision optionally lists `related_axioms[]` that must resolve to real universal axioms (test-time invariant, parallel to Phase 3's project_style overrides).
5. Tests including a fixture decision directory under the existing Phase 3 sample.

## Scope

**In scope:**
- New module `src/server/tools/decisions.rs` (parse-once cache, schema types, `handle_decisions`, `load_from_dir` helper).
- New MCP tool `mcp__illu__decisions` registered in `src/server/mod.rs` with `query: String` parameter.
- Schema: `id`, `title`, `status` (enum), `date` (ISO-8601), `context`, `decision`, `alternatives_considered[]`, `consequences`, optional `related_axioms[]` and `related_files[]`.
- Server-startup load alongside `project_style::init`, sharing the same `repo_root` plumbing.
- Test fixture: 3 decisions under `tests/fixtures/illu_style_sample/.illu/style/decisions/` exercising different statuses and at least one `related_axioms` cross-reference.
- 8 tests: validity (5) + per-batch focused query (1) + demo query (1) + integration with the live tool surface (1).

**Explicit non-goals:**
- No universal corpus of decisions in `assets/` — design records are inherently project-specific.
- No interactive editing from the MCP server (project authors edit JSON directly).
- No two-way axiom↔decision links (decisions reference axioms; axioms don't reference decisions back).
- No file-watching hot-reload (server restart picks up changes; parallel to other corpora).
- No status-transition logic (e.g., enforcing that a `superseded` record has a `superseded_by` field) — this is decision *capture*, not workflow.

## Architecture

```
illu-rs
├── assets/                                   (universal corpora, unchanged)
│   ├── rust_quality_axioms.json
│   └── rust_exemplars/
└── src/server/tools/
    ├── axioms.rs                             (modified in Phase 3 — unchanged in Phase 4)
    ├── exemplars.rs                          (Phase 2 — unchanged)
    ├── project_style.rs                      (Phase 3 — unchanged)
    └── decisions.rs                          (new)

{repo}/.illu/style/                           (project-local, optional)
├── project.json                              (Phase 3)
└── decisions/                                (Phase 4)
    ├── 0001-enum-dispatch-handlers.json
    ├── 0002-mutex-row-scheme.json
    └── ...
```

**Discovery:** the same `repo_root` that Phase 3's `project_style::init` consumes is also passed to `decisions::init` (called from `IlluServer::new`). `decisions::init` walks `{repo_root}/.illu/style/decisions/`, parses each `*.json` file, deserializes into a `RawDecision`, validates, converts to `Decision`. Skip files that don't end in `.json`; surface `tracing::warn!` (not fatal) on parse failure for individual files so one bad record doesn't disable the whole directory.

**Schema (per file):**

```json
{
  "id": "decision_use_enum_dispatch_for_handlers",
  "title": "Use enum dispatch for command handlers",
  "status": "accepted",
  "date": "2026-04-15",
  "context": "We have 12 command handlers; dispatch happens on the hot path...",
  "decision": "We chose enum + match over Box<dyn Handler> because the variant set is closed and dispatch latency matters here...",
  "alternatives_considered": [
    {
      "option": "Box<dyn Handler>",
      "why_rejected": "Requires vtable indirect call on every dispatch; benchmarks showed 8% throughput regression vs enum on this workload."
    },
    {
      "option": "Function pointers (fn(&Cmd) -> Resp)",
      "why_rejected": "Loses the ability to carry per-handler state, which we need for connection-pool reuse."
    }
  ],
  "consequences": "Adding a new command requires touching the enum and the match (compile-time-enforced). External crates cannot add commands.",
  "related_axioms": ["rust_quality_90_enum_dispatch"],
  "related_files": ["src/handlers/mod.rs", "src/handlers/dispatch.rs"]
}
```

**Schema invariants** (enforced at parse time except where noted):
- `id` must start with `decision_`. Establishes namespace separation from `rust_quality_*` (universal axioms), `project_*` (Phase 3 local axioms), and the existing exemplar slugs.
- `id` must be unique across all loaded files.
- `status` must be one of `"proposed"`, `"accepted"`, `"superseded"`, `"deprecated"`. Standard ADR/MADR statuses.
- `date` must be valid ISO-8601 `YYYY-MM-DD`. Stored as `String` (we don't need calendar arithmetic).
- `alternatives_considered` is an array of objects, possibly empty, each with `option` and `why_rejected`. Typed (vs free-text) so future tooling can highlight rejected options or filter by alternative.
- `related_axioms[]` IDs must resolve to real universal axioms (test-time invariant, parallel to Phase 3 `project_style` — `parse` cannot see the universal corpus without a circular import).
- `related_files[]` are not validated; file paths can drift legitimately as code moves.

**Server module (`decisions.rs`):**

```rust
#[derive(Debug, Deserialize)]
struct RawDecision {
    id: String,
    title: String,
    status: Status,
    date: String,
    context: String,
    decision: String,
    #[serde(default)]
    alternatives_considered: Vec<RawAlternative>,
    consequences: String,
    #[serde(default)]
    related_axioms: Vec<String>,
    #[serde(default)]
    related_files: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawAlternative {
    option: String,
    why_rejected: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Status {
    Proposed,
    Accepted,
    Superseded,
    Deprecated,
}

#[non_exhaustive]
pub struct Alternative {
    pub option: String,
    pub why_rejected: String,
}

#[non_exhaustive]
pub struct Decision {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub date: String,
    pub context: String,
    pub decision: String,
    pub alternatives_considered: Vec<Alternative>,
    pub consequences: String,
    pub related_axioms: Vec<String>,
    pub related_files: Vec<String>,
    // Pre-lowercased mirrors used by the scorer.
    title_lower: String,
    context_lower: String,
    decision_lower: String,
    consequences_lower: String,
}
```

**Scoring** mirrors axioms: per-token partial-match against title (+10), context (+5), decision (+5), consequences (+2), alternatives_considered fields (+2 combined), full-query equality boost on title (+30) and status (+20). `MAX_DECISION_RESULTS = 4` — decisions are roughly the size of exemplars, so the same cap applies.

**Cache lifetime:** `Decision`s are loaded once at server startup; no hot-reload. Edit cycle is "edit JSON → restart server."

**Empty default:** absent directory → empty `Vec<Decision>` → `handle_decisions` returns "No decisions configured." → no behavior change for Phase-3-only projects.

## MCP tool: `mcp__illu__decisions`

**Parameters:** `query: String`.

**Returns:** Markdown summary of up to `MAX_DECISION_RESULTS` matched decisions. For each:
- `## {title} (`{status}`, {date})`
- `**ID:** \`{id}\``
- `### Context`, `### Decision`, `### Alternatives Considered` (if non-empty), `### Consequences`
- If `related_axioms` non-empty: `**Related axioms:** {comma-separated IDs}`
- If `related_files` non-empty: `**Related files:** {comma-separated paths}`

If no matches: `## Decisions\n\nNo decisions matched the query.\n` or, if the directory is absent entirely, `## Decisions\n\nNo decision records are configured (`.illu/style/decisions/` is absent or empty).\n`.

## Tests

8 tests in `decisions::tests`:

1. `test_decisions_parses_empty_dir` — absent or empty directory → `Vec::new()`, no error.
2. `test_decisions_parses_fixture` — fixture loads to 3 decisions with expected statuses.
3. `test_decisions_id_namespace` — every loaded ID starts with `decision_`.
4. `test_decisions_unique_ids` — no duplicate IDs across the loaded corpus.
5. `test_decisions_related_axioms_resolve` — every `related_axioms` ID exists in the universal corpus (uses `axioms_for_test`).
6. `test_decisions_status_enum_validates` — a fixture file with an unknown status string fails parse with a clear error.
7. `test_handle_decisions_focused_query` — query for the fixture's enum-dispatch record surfaces it.
8. `test_handle_decisions_demo_query` — broad query surfaces ≥ 2 of the 3 fixture records.

A 9th smoke test exists implicitly: the live MCP smoke test (after rebuild + restart) where `mcp__illu__decisions` returns the configured decisions for the fixture project.

## Fixture

`tests/fixtures/illu_style_sample/.illu/style/decisions/` (extends the existing Phase 3 sample) with 3 records:

- `0001-enum-dispatch-handlers.json` — status `accepted`, `related_axioms: ["rust_quality_90_enum_dispatch"]`. Demonstrates a typical "we chose enum over dyn" decision.
- `0002-mutex-per-row-scheme.json` — status `superseded`, no `related_axioms`. Demonstrates a decision that's been replaced (the body should mention what superseded it informally; `superseded_by` is out of scope per the non-goals).
- `0003-experimental-async-runtime.json` — status `proposed`, `related_axioms: ["rust_quality_74_mutexguard_across_await"]` (or similar). Demonstrates a decision under consideration.

Filenames are numbered for ordering convenience; the loader sorts by ID, not filename, so numbering is purely a human-author hint.

## Verification and Exit Criteria

Phase 4 is complete when:
- `src/server/tools/decisions.rs` exists with the schema, parse-once cache, `handle_decisions`, and `load_from_dir` helper.
- `mcp__illu__decisions` registered in `src/server/mod.rs`; re-exported in `src/api.rs`.
- `IlluServer::new` calls `decisions::init` alongside `project_style::init`.
- Fixture directory at `tests/fixtures/illu_style_sample/.illu/style/decisions/` with 3 records.
- 8 tests pass.
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.
- Live MCP smoke test (after rebuild + restart): the new tool returns the configured decisions for the fixture project, and queries match relevant records.

## Risks and Mitigations

- **Schema drift between project files and corpus.** A decision's `related_axioms[].id` may reference a stale axiom ID after a corpus rename. Mitigation: parallel to Phase 3 — `tracing::warn!` at startup for unresolving IDs (re-using the same machinery added in Phase 3 final-review nit fix).
- **One bad JSON file breaking the whole directory load.** Mitigation: per-file parse error → `tracing::warn!` and skip; the rest of the directory still loads. This differs from Phase 3, where the single `project.json` is all-or-nothing.
- **Status transitions encoding.** A `superseded` record without a `superseded_by` field is allowed today. If projects want machine-readable "what replaced this", they can add it via free-text in the body. Phase 4.1 could add a typed `superseded_by` field if demand surfaces.
- **Plan drift between drafts and post-fix files** (recurring). Mitigation: explicit reconciliation pass before final review.
- **Date validation cost.** Strict ISO-8601 parsing requires either `time` or `chrono` as a dependency. Mitigation: regex-light validation (`[0-9]{4}-[0-9]{2}-[0-9]{2}`) at parse time, no calendar correctness check. Storing as `String` keeps the surface tiny.

## Phase 4.1+ Continuation Outline

Out of scope for this slice; sketched only:
- Phase 4.1 — `superseded_by` typed field; status-transition validation (e.g., `superseded` requires the field).
- Phase 4.2 — file-watching hot-reload for `.illu/style/decisions/`.
- Phase 4.3 — query filters by status (e.g., "show me only `accepted`").

Phase 5+ (per the original Phase 0 spec):
- Phase 5 — critique (axiom-violation detection in diffs).
- Phase 6 — cost profile (per-axiom token-budget weight).

## Open Questions for User Review

- Confirm the `decision_*` ID prefix (vs `adr_*` or `dr_*` — `decision_` is most readable).
- Confirm `alternatives_considered` as a typed array of `{option, why_rejected}` (vs free-text). Typed is more queryable; free-text is more flexible.
- Confirm 3-decision fixture (vs more).
- Confirm date validation strictness (regex-light vs full chrono parse). Lean: regex-light, no new dep.
