# Jon-Style Rust — Phase 3 (Project Context Overrides) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 3 — `.illu/style/project.json` as a per-project override layer over the universal axiom corpus. Adds a new MCP tool to inspect the active config and modifies `handle_axioms` scoring to consult overrides.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md) for architecture inheritance; [Phase 2 spec](2026-04-27-jon-style-rust-phase-2-exemplars-design.md) for the most-recent precedent on adding a new MCP tool.

## Motivation

Phase 0 + Phase 1 (4 slices) shipped 102 universal axioms. Phase 2 added 9 exemplars. All of these apply uniformly to every codebase. But projects make local decisions:

- *"We use `anyhow` project-wide; suppress the `thiserror`-flavored axiom 64."*
- *"This codebase is allocation-sensitive; promote axiom 85 (allocation hot paths) above its usual ranking."*
- *"All database access must go through the `repository::*` module — that's a project-local axiom the universal corpus doesn't know about."*

Phase 3 introduces the first *trust model change* of the campaign: project authors gain a structured place to override the universal corpus. Source material: `clippy.toml`'s per-project lint configuration; the `.cargo/config.toml` precedent for project-wide tooling tuning.

**Trust model:** project authors *can* hide axioms they should respect, but that's their call — they own the codebase. The trust direction is: agent → corpus (universal) → project overrides (local). The agent treats `.illu/style/project.json` as authoritative for the working tree it lives in.

## Goal

1. Define `.illu/style/project.json` schema for axiom overrides + project-local axioms.
2. Add a new MCP tool `mcp__illu__project_style` returning the active config.
3. Modify `handle_axioms` scoring to honor overrides: `ignored` filters out, `demoted` halves score, `elevated` doubles, `noted` appends the project note.
4. Project-local axioms surface alongside universal ones in `handle_axioms` results, gated by their own `project_*` ID prefix to keep namespaces disjoint.
5. Tests including a fixture project that exercises every override severity and a project-local axiom.

## Scope

**In scope:**
- New module `src/server/tools/project_style.rs` (parse-once cache, schema types, `handle_project_style`, `load_project_style` helper).
- New MCP tool `mcp__illu__project_style` registered in `src/server/mod.rs`.
- Modified `handle_axioms` to consult `ProjectStyle` overrides during scoring and result formatting.
- Modified `axioms()` parse-once cache (or a sibling `axioms_with_local()` helper) to fold project-local axioms into the scoring corpus.
- A test fixture at `tests/fixtures/illu_style_sample/.illu/style/project.json` exercising every override severity + a project-local axiom.
- Tests: schema parses, override IDs resolve to real axioms, local axiom IDs are unique and `project_*`-prefixed, integration test that loads the fixture and asserts scoring behavior.

**Explicit non-goals:**
- No per-file path-pattern overrides (deferred to Phase 3.1 if useful).
- No project-local exemplars (deferred to Phase 3.1).
- No modifications to `assets/rust_quality_axioms.json` or `assets/rust_exemplars/manifest.json` — the universal corpus is unchanged.
- No interactive editing of the project config from the MCP server (project authors edit the JSON directly).
- No multi-tenant config (one config per project root).

## Architecture

```
illu-rs
├── assets/                                   (universal corpus, unchanged)
│   ├── rust_quality_axioms.json
│   └── rust_exemplars/
├── src/server/tools/
│   ├── axioms.rs                             (modified — consults ProjectStyle)
│   ├── exemplars.rs                          (unchanged in Phase 3)
│   └── project_style.rs                      (new)
└── tests/fixtures/illu_style_sample/
    └── .illu/style/project.json              (new — test fixture)
```

**Discovery:** the MCP server is invoked with a project path (`/path/to/repo`). The existing index lives at `{repo}/.illu/index.db`. Phase 3 reads `{repo}/.illu/style/project.json` from the same root. If absent, behavior is identical to Phase 2 (no overrides, no project-local axioms). The path is supplied via the same mechanism as the index path — captured at server startup.

**Schema** (`.illu/style/project.json`):

```json
{
  "version": 1,
  "axiom_overrides": [
    {
      "id": "rust_quality_64_error_chain",
      "severity": "ignored",
      "note": "we use anyhow project-wide; thiserror is not the convention here"
    },
    {
      "id": "rust_quality_85_allocation_hot_paths",
      "severity": "elevated",
      "note": "this codebase is allocation-sensitive; treat as mandatory in handlers"
    },
    {
      "id": "rust_quality_87_iterator_codegen",
      "severity": "noted",
      "note": "we have benchmarks showing indexed access is sometimes faster on our profile; pair iter() vs indexed by measurement"
    }
  ],
  "local_axioms": [
    {
      "id": "project_repository_pattern",
      "category": "Project Convention",
      "source": ".illu/style/project.json",
      "triggers": ["repository module", "database access", "data layer"],
      "rule_summary": "All database access goes through the repository::* module hierarchy. Direct sqlx/diesel calls outside repository:: are forbidden.",
      "prompt_injection": "MANDATORY RULE: When writing DB code, always go through repository::*. Do not call sqlx/diesel directly elsewhere.",
      "anti_pattern": "// In services/orders.rs (NOT a repository module):\nlet row = sqlx::query!(\"SELECT * FROM orders WHERE id = $1\", id).fetch_one(&pool).await?;",
      "good_pattern": "// services/orders.rs delegates to repository:\nlet order = repository::orders::find_by_id(&pool, id).await?;"
    }
  ]
}
```

**Severity values** (in order of effect on scoring):
- `"ignored"` — filter out entirely; the axiom never appears in `handle_axioms` results.
- `"demoted"` — multiply score by 0.5 (truncate to integer); axiom can still appear but is ranked lower.
- `"noted"` — score unchanged; the project's `note` is appended to the axiom's display in `handle_axioms` results.
- `"elevated"` — multiply score by 2 (saturating at the score type's bounds); axiom is ranked higher than its universal default.

The multiplier semantics (vs additive constants) keep `score=0` axioms at zero — i.e., `demoted` cannot accidentally promote a 0-score axiom into the result set, and `elevated` cannot conjure score from nothing.

**Schema invariants** (enforced by tests):
- `axiom_overrides[].id` must resolve to a real `rust_quality_*` axiom (typo'd or stale IDs are loud failures).
- `local_axioms[].id` must start with `project_` (separate namespace; collisions with `rust_quality_*` are forbidden).
- `local_axioms[].id` must be unique within `local_axioms`.
- The same `id` may appear in either `axiom_overrides` or `local_axioms`, but not both — overriding a project-local axiom is meaningless (the project owns it directly).
- `version` is checked; only `1` is supported in Phase 3.

**Server module** (`src/server/tools/project_style.rs`):

```rust
#[derive(Debug, Deserialize)]
struct RawProjectStyle {
    version: u32,
    #[serde(default)]
    axiom_overrides: Vec<RawAxiomOverride>,
    #[serde(default)]
    local_axioms: Vec<RawAxiom>,  // reuses the existing RawAxiom shape
}

#[derive(Debug, Deserialize)]
struct RawAxiomOverride {
    id: String,
    severity: Severity,
    #[serde(default)]
    note: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Severity {
    Ignored,
    Demoted,
    Noted,
    Elevated,
}

pub struct ProjectStyle {
    pub overrides: HashMap<String, AxiomOverride>,  // axiom id -> override
    pub local_axioms: Vec<Axiom>,                    // reuses Axiom from axioms.rs
}

pub struct AxiomOverride {
    pub severity: Severity,
    pub note: String,
}
```

**Integration with `handle_axioms`:** the scoring function consults `ProjectStyle::overrides` after computing the universal score; the formatting function appends `note` text when severity is `Noted`. The local-axiom corpus is iterated alongside the universal one (same scoring weights). The `project_*` prefix on local-axiom IDs makes the source-of-truth explicit in the response.

**Empty-project default:** `ProjectStyle::default()` returns empty maps; `axioms()` returns the universal corpus unchanged. Projects without `.illu/style/project.json` see Phase-2 behavior with zero overhead.

**Cache lifetime:** `ProjectStyle` is loaded once at server startup, identically to the axioms cache. Re-reading on file change is out of scope for Phase 3 (server restart is the supported edit-cycle).

## Schema (Rust types)

Defined inline above. The `Axiom` struct from `axioms.rs` is reused for `local_axioms` — same shape, no schema fork. The `id` field (added in Phase 2 Task 1) carries the `project_*` namespace.

## MCP Tool: `mcp__illu__project_style`

**Parameters:** none.

**Returns:** the active project style as a Markdown summary:
- `## Active project style at <repo path>` (or "No project style configured" if absent).
- Override list grouped by severity (Ignored / Demoted / Noted / Elevated), each with the axiom ID, the override note, and the universal axiom's title for context.
- Local axiom list, each with category, title, triggers, and rule summary.

**Use case:** an agent inspecting why an axiom was filtered out or got an unexpected ranking can call this tool to see the active overrides. Also useful for a human-in-the-loop review of project conventions encoded in the file.

## Verification and Exit Criteria

Phase 3 is complete when:
- `src/server/tools/project_style.rs` exists with the schema, parse-once cache, and `handle_project_style`.
- `mcp__illu__project_style` registered in `src/server/mod.rs`.
- `handle_axioms` honors `ProjectStyle::overrides` (filter / demote / elevate / note).
- Local axioms surface in `handle_axioms` results with `project_*` IDs.
- New tests:
  - `test_project_style_parses_empty` (no file → default config).
  - `test_project_style_parses_fixture` (fixture file → expected struct).
  - `test_project_style_override_id_resolves` (each override ID exists in the universal corpus).
  - `test_project_style_local_axiom_ids_namespaced` (each local ID starts with `project_`).
  - `test_project_style_local_axiom_ids_unique` (no duplicates within local_axioms).
  - `test_project_style_local_axiom_does_not_shadow_universal` (no local ID collides with `rust_quality_*`).
  - `test_handle_axioms_respects_ignored` (an `ignored` axiom never surfaces).
  - `test_handle_axioms_respects_demoted_elevated` (relative ranking honors demote/elevate).
  - `test_handle_axioms_appends_noted` (the project note appears in the result text for `noted` axioms).
  - `test_handle_axioms_surfaces_local_axiom` (a project-local axiom appears in results when its triggers match).
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.
- Live MCP smoke test (after rebuild + restart): the new tool returns the configured style for a project that has a fixture file, and `mcp__illu__axioms` honors overrides for that project.

## Risks and Mitigations

- **Schema drift between project files and corpus.** A project's `axiom_overrides[].id` may reference a stale axiom ID after a corpus update. Mitigation: server logs a warning at startup for unresolved IDs and proceeds (treat unresolved as no-op rather than fatal — projects shouldn't be locked out by an upstream rename).
- **Local-axiom ID collisions with future `rust_quality_*` axioms.** Mitigation: hard `project_*` prefix on local IDs is a static guarantee.
- **Project authors hiding important axioms.** Mitigation: this is by design — projects own their codebase. Defense: the `mcp__illu__project_style` tool makes the override list visible, so a reviewer can see what's been suppressed.
- **Cache invalidation.** A project author edits `.illu/style/project.json` and expects to see the change without restarting. Mitigation: not in scope for Phase 3; documented behavior is "restart to pick up changes" (matches the existing axiom corpus behavior, where an `assets/rust_quality_axioms.json` edit also requires rebuild + restart).
- **Plan drift between drafts and post-fix files** (recurring across all prior slices). Mitigation: explicit reconciliation pass before final review.

## Phase 3.1+ Continuation Outline

Out of scope for this slice; sketched only:
- Phase 3.1 — per-file path-pattern overrides (e.g., `tests/**/*.rs` exempts unwrap-related axioms).
- Phase 3.2 — project-local exemplars (`.illu/style/exemplars/`) mirroring `assets/rust_exemplars/`.
- Phase 3.3 — file-watching hot-reload of `.illu/style/project.json`.

Phase 4+ (per the original Phase 0 spec):
- Phase 4 — design record (structured "we chose X over Y because Z" capture).
- Phase 5 — critique (axiom-violation detection in diffs).
- Phase 6 — cost profile (per-axiom token-budget weight).

## Open Questions for User Review

- Confirm Architecture B (active overrides) — already approved in the brainstorm; this is a checkpoint.
- Confirm severity multipliers (× 0.5 / × 2.0 with integer truncation) vs additive (e.g., −10 / +10). Multipliers are recommended because they preserve the "score=0 stays zero" invariant.
- Confirm `version` field on the schema; only `1` is recognized in Phase 3. Future versions can either add a migration path or reject unknown versions.
- Confirm test-fixture path under `tests/fixtures/illu_style_sample/`. Alternative: put the fixture file in a more discoverable location (e.g., `assets/`).
- Any candidate features to defer further or pull forward.
