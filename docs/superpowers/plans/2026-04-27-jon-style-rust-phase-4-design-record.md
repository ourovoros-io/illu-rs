# Jon-Style Rust Phase 4 (Design Record) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Add `.illu/style/decisions/` as a project-local corpus of ADR-style design records. New module + new MCP tool + 9 tests + 3-decision fixture.

**Architecture:** New module `src/server/tools/decisions.rs` mirrors the parse-once-cache pattern of `axioms.rs` and `project_style.rs`. `Decision`s are loaded once at server startup from `{repo}/.illu/style/decisions/*.json` (multi-file, one decision per file). Absent directory → empty default → Phase-3 behavior unchanged. New `mcp__illu__decisions` tool scores queries against title/context/decision/consequences/alternatives.

**Tech Stack:** Rust 2024, `serde_json`, `rmcp` macros. No new external dependencies (date validation is regex-light to avoid pulling in `chrono`/`time`).

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-4-design-record-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-4-design-record-design.md)

**Existing state:** Phase 0–3 merged. 102 universal axioms, 9 exemplars, project_style override layer. `IlluServer::new` in `src/server/mod.rs` already calls `tools::project_style::init(&repo_root)` — Phase 4 hooks alongside.

**Lint constraints** (unchanged): `unwrap_used = "deny"`, `expect_used = "warn"`, `allow_attributes = "deny"`. Tests use `#[expect(clippy::unwrap_used, reason = "tests")]` at module scope.

**Repo-root plumbing:** already plumbed by Phase 3 — `Database::repo_root()` accessor exists. `decisions::init` consumes the same `&Path`.

**Drafts pre-approved by user.** Skip the user-review step inside batch tasks.

**Key design decisions** (per the spec):
- ID prefix: `decision_*`.
- `alternatives_considered`: typed array of `{option, why_rejected}`.
- Fixture: 3 decisions (one per non-`deprecated` status).
- Date validation: regex-light `YYYY-MM-DD` (no chrono dep).
- Per-file parse error: log warn, skip that file. Duplicate ID: log warn, keep first.
- `related_axioms[]` resolution: test-time invariant + startup `tracing::warn!` for stale IDs (parallel to Phase 3).

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `src/server/tools/decisions.rs` | Create | Module: schema types, loader, parse-once cache, `handle_decisions`, 8 tests |
| `src/server/tools/mod.rs` | Modify | Add `pub mod decisions;` |
| `src/server/mod.rs` | Modify | Register `mcp__illu__decisions` tool; init `decisions` at server startup |
| `src/api.rs` | Modify | Re-export `decisions::handle_decisions` |
| `tests/fixtures/illu_style_sample/.illu/style/decisions/0001-enum-dispatch-handlers.json` | Create | Fixture decision (`accepted`) |
| `tests/fixtures/illu_style_sample/.illu/style/decisions/0002-mutex-per-row-scheme.json` | Create | Fixture decision (`superseded`) |
| `tests/fixtures/illu_style_sample/.illu/style/decisions/0003-experimental-async-runtime.json` | Create | Fixture decision (`proposed`) |

---

## Task 1: Schema + Loader + Decisions Tool + Fixture + Tests

**Files (create):** as listed above.

**Files (modify):**
- `src/server/tools/mod.rs` (add `pub mod decisions;`)
- `src/server/mod.rs` (add `DecisionsParams` + tool method; init from `IlluServer::new`)
- `src/api.rs` (re-export)

- [ ] **Step 1: Create the three fixture decisions**

Create `tests/fixtures/illu_style_sample/.illu/style/decisions/0001-enum-dispatch-handlers.json`:

```json
{
  "id": "decision_use_enum_dispatch_for_handlers",
  "title": "Use enum dispatch for command handlers",
  "status": "accepted",
  "date": "2026-04-15",
  "context": "We have 12 command handlers; dispatch happens on the request hot path. Profiling showed indirect-call overhead from Box<dyn Handler> was a measurable fraction of per-request time.",
  "decision": "We chose enum + match over Box<dyn Handler>. The variant set is closed at the library boundary, so external extensibility is not required, and dispatch latency matters more than open-set ergonomics here.",
  "alternatives_considered": [
    { "option": "Box<dyn Handler>", "why_rejected": "Requires vtable indirect call on every dispatch; benchmarks showed 8% throughput regression vs enum on this workload." },
    { "option": "Function pointers (fn(&Cmd) -> Resp)", "why_rejected": "Loses the ability to carry per-handler state; we need that for connection-pool reuse and per-handler metrics." }
  ],
  "consequences": "Adding a new command requires touching the Command enum and the dispatch match (compile-time-enforced exhaustiveness). External crates cannot add commands — that is the design choice.",
  "related_axioms": ["rust_quality_90_enum_dispatch"],
  "related_files": ["src/handlers/mod.rs", "src/handlers/dispatch.rs"]
}
```

Create `0002-mutex-per-row-scheme.json`:

```json
{
  "id": "decision_replace_mutex_per_row",
  "title": "Replace mutex-per-row with sharded RwLock cache",
  "status": "superseded",
  "date": "2026-03-02",
  "context": "Original cache used one Mutex per row to allow fine-grained locking. Profiling under concurrent read load showed catastrophic contention from the pthread mutex syscall overhead even when no two threads ever touched the same row.",
  "decision": "We chose a 64-shard RwLock<HashMap<Key, Value>> scheme. Reads are uncontended within a shard; writers take the per-shard write lock. Hot-key fairness is not required for our workload.",
  "alternatives_considered": [
    { "option": "Keep per-row Mutex", "why_rejected": "Profiler showed contention even on disjoint rows; per-row granularity wasn't paying for its overhead." },
    { "option": "DashMap", "why_rejected": "External dependency we can avoid for a 64-shard scheme that's <100 LOC." }
  ],
  "consequences": "Cache implementation is a custom 64-shard map; readers see uncontended access for ~98% of requests in production. Sharding is fixed at compile time. This decision was later superseded by switching to a fully lock-free structure when the contention shifted to the write path.",
  "related_axioms": ["rust_quality_73_interior_mutability_selection"],
  "related_files": ["src/cache/sharded.rs"]
}
```

Create `0003-experimental-async-runtime.json`:

```json
{
  "id": "decision_evaluate_smol_runtime",
  "title": "Evaluate smol as an alternative async runtime",
  "status": "proposed",
  "date": "2026-04-20",
  "context": "We currently use tokio for the server. Memory overhead per task is non-trivial (~2KB minimum for a tokio task) and we expect to launch ~100K concurrent connections under peak load. smol claims a smaller per-task footprint.",
  "decision": "Investigate smol as a swap-in replacement; benchmark memory and throughput under the production workload before deciding. No code change yet — this is a proposed experiment.",
  "alternatives_considered": [
    { "option": "Stay on tokio", "why_rejected": "Not yet — we want measurements before committing." },
    { "option": "async-std", "why_rejected": "Project is in maintenance mode; not a viable migration target." }
  ],
  "consequences": "If accepted: the runtime swap touches every spawn site (~40 call sites) plus tokio::sync usage (which would migrate to async-channel or similar). If rejected: we document why and revisit if memory pressure changes.",
  "related_axioms": ["rust_quality_74_mutex_guard_await"],
  "related_files": []
}
```

- [ ] **Step 2: Verify each `related_axioms[].id` exists in the corpus**

```bash
for id in rust_quality_90_enum_dispatch rust_quality_73_interior_mutability_selection rust_quality_74_mutex_guard_await; do
  grep -q "\"id\": \"$id\"" assets/rust_quality_axioms.json && echo "OK: $id" || echo "MISSING: $id"
done
```

If any are missing, find the correct ID in `assets/rust_quality_axioms.json` and update the fixture(s).

- [ ] **Step 3: Create `src/server/tools/decisions.rs`**

```rust
//! ADR-style design records loaded from `{repo}/.illu/style/decisions/`.
//!
//! Each `*.json` file under that directory is one decision record. The
//! loader walks the directory at server startup, parses each file, logs
//! `tracing::warn!` on per-file failures (so one bad record does not
//! disable the whole directory), and stores the result in a process-global
//! parse-once cache. The MCP tool `mcp__illu__decisions` scores user
//! queries against the loaded corpus and returns matches as Markdown.
//!
//! Schema invariants enforced at parse time:
//! - `id` starts with `decision_`.
//! - `id` is unique across the directory (duplicates → keep first, warn).
//! - `status` is one of the four enum values.
//! - `date` matches `YYYY-MM-DD` (regex-light; no calendar correctness
//!   check, intentional to avoid a chrono/time dependency).
//!
//! The `related_axioms[].id` resolution invariant is enforced as a
//! test-time check (parallel to `project_style`) because parse cannot see
//! the universal corpus without a circular import.

use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

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

/// Lifecycle status of a decision record. Matches MADR conventions.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Status {
    /// Under consideration; no action taken yet.
    Proposed,
    /// In effect today.
    Accepted,
    /// Replaced by a later decision; preserved for historical context.
    Superseded,
    /// No longer applicable; preserved for historical context.
    Deprecated,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Alternative {
    pub option: String,
    pub why_rejected: String,
}

/// Resolved decision record with pre-lowercased mirror fields used by the
/// scorer in [`handle_decisions`].
#[derive(Debug, Clone)]
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
    title_lower: String,
    context_lower: String,
    decision_lower: String,
    consequences_lower: String,
    alternatives_lower: String,
}

impl Decision {
    fn from_raw(raw: RawDecision) -> Self {
        let title_lower = raw.title.to_lowercase();
        let context_lower = raw.context.to_lowercase();
        let decision_lower = raw.decision.to_lowercase();
        let consequences_lower = raw.consequences.to_lowercase();
        let alternatives = raw
            .alternatives_considered
            .into_iter()
            .map(|a| Alternative {
                option: a.option,
                why_rejected: a.why_rejected,
            })
            .collect::<Vec<_>>();
        // Pre-flatten alternatives into one lowercase blob for scoring;
        // we don't need to score per-alternative.
        let alternatives_lower = alternatives
            .iter()
            .map(|a| format!("{} {}", a.option.to_lowercase(), a.why_rejected.to_lowercase()))
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            id: raw.id,
            title: raw.title,
            status: raw.status,
            date: raw.date,
            context: raw.context,
            decision: raw.decision,
            alternatives_considered: alternatives,
            consequences: raw.consequences,
            related_axioms: raw.related_axioms,
            related_files: raw.related_files,
            title_lower,
            context_lower,
            decision_lower,
            consequences_lower,
            alternatives_lower,
        }
    }
}

const MAX_DECISION_RESULTS: usize = 4;

/// Process-global cache, set once at server startup via [`init`]. If
/// `init` is not called the accessor falls back to an empty default,
/// preserving Phase-3 behavior.
static DECISIONS: OnceLock<Vec<Decision>> = OnceLock::new();

/// Returns the active decision corpus, or an empty slice if [`init`] was
/// not called or the directory was absent.
pub fn decisions() -> &'static [Decision] {
    static EMPTY: OnceLock<Vec<Decision>> = OnceLock::new();
    DECISIONS
        .get()
        .map(Vec::as_slice)
        .unwrap_or_else(|| EMPTY.get_or_init(Vec::new).as_slice())
}

/// Load `{repo_root}/.illu/style/decisions/*.json` into the global cache.
/// Called once from `IlluServer::new`. Per-file parse errors are logged
/// at `warn` and the file is skipped; duplicate IDs across files are
/// logged and the second occurrence is dropped. Stale `related_axioms[]`
/// IDs are logged at startup (parallel to Phase 3).
pub fn init(repo_root: &Path) -> Result<(), crate::IlluError> {
    let dir = repo_root.join(".illu").join("style").join("decisions");
    let loaded = if dir.exists() {
        load_from_dir(&dir)?
    } else {
        Vec::new()
    };

    // Stale-related-axiom warning, parallel to project_style::init.
    if !loaded.is_empty()
        && let Ok(universal) = crate::server::tools::axioms::axioms_for_runtime()
    {
        let known: HashSet<&str> = universal.iter().map(|a| a.id.as_str()).collect();
        for d in &loaded {
            for axiom_id in &d.related_axioms {
                if !known.contains(axiom_id.as_str()) {
                    tracing::warn!(
                        decision = %d.id,
                        %axiom_id,
                        "decision related_axiom does not resolve to a universal axiom"
                    );
                }
            }
        }
    }

    let _ = DECISIONS.set(loaded);
    Ok(())
}

/// Public for tests. Walks `dir`, parses each `*.json` file. Per-file
/// parse failures log `warn` and are skipped. Duplicate IDs across files
/// log `warn` and the second occurrence is dropped (first wins).
pub fn load_from_dir(dir: &Path) -> Result<Vec<Decision>, crate::IlluError> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| {
            crate::IlluError::Other(format!("failed to read {}: {e}", dir.display()))
        })?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .map(|ext| ext.eq_ignore_ascii_case("json"))
                    .unwrap_or(false)
        })
        .collect();
    entries.sort();

    let mut out: Vec<Decision> = Vec::with_capacity(entries.len());
    let mut seen: HashSet<String> = HashSet::new();

    for path in entries {
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to read decision file; skipping");
                continue;
            }
        };
        let decision = match parse_decision(&text) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to parse decision file; skipping");
                continue;
            }
        };
        if !seen.insert(decision.id.clone()) {
            tracing::warn!(
                path = %path.display(),
                id = %decision.id,
                "duplicate decision id; keeping the first occurrence"
            );
            continue;
        }
        out.push(decision);
    }

    Ok(out)
}

fn parse_decision(json: &str) -> Result<Decision, crate::IlluError> {
    let raw: RawDecision = serde_json::from_str(json)
        .map_err(|e| crate::IlluError::Other(format!("failed to parse decision: {e}")))?;
    if !raw.id.starts_with("decision_") {
        return Err(crate::IlluError::Other(format!(
            "decision id `{}` must start with `decision_`",
            raw.id
        )));
    }
    if !is_iso_date(&raw.date) {
        return Err(crate::IlluError::Other(format!(
            "decision `{}` has malformed date `{}`; expected YYYY-MM-DD",
            raw.id, raw.date
        )));
    }
    Ok(Decision::from_raw(raw))
}

/// Regex-light ISO-8601 date validator: 4 digits, dash, 2 digits, dash, 2
/// digits. We do not check that the date is a valid calendar date (e.g.
/// 2026-02-30 is accepted) — this would require a chrono/time dependency
/// and is not load-bearing for the use case.
fn is_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    bytes
        .iter()
        .enumerate()
        .all(|(i, &b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

/// Score one decision against a tokenized query. Mirrors the
/// axioms-tool scoring shape: per-token partial-match accumulation in
/// the loop, then full-query equality boosts after the loop.
fn score_decision(decision: &Decision, query_lower: &str, query_tokens: &[&str]) -> usize {
    let mut total = 0usize;
    for token in query_tokens {
        if decision.title_lower.contains(token) {
            total += 10;
        }
        if decision.context_lower.contains(token) {
            total += 5;
        }
        if decision.decision_lower.contains(token) {
            total += 5;
        }
        if decision.consequences_lower.contains(token) {
            total += 2;
        }
        if decision.alternatives_lower.contains(token) {
            total += 2;
        }
    }
    if decision.title_lower == query_lower {
        total = total.saturating_add(30);
    }
    // status equality boost (e.g. user types "accepted" to find all accepted records)
    let status_str = match decision.status {
        Status::Proposed => "proposed",
        Status::Accepted => "accepted",
        Status::Superseded => "superseded",
        Status::Deprecated => "deprecated",
    };
    if status_str == query_lower {
        total = total.saturating_add(20);
    }
    total
}

/// Returns up to [`MAX_DECISION_RESULTS`] decisions best matching `query`.
pub fn handle_decisions(query: &str) -> Result<String, crate::IlluError> {
    let corpus = decisions();
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    if corpus.is_empty() {
        return Ok(
            "## Decisions\n\nNo decision records are configured (`.illu/style/decisions/` is absent or empty).\n"
                .to_string(),
        );
    }

    let mut scored: Vec<(usize, &Decision)> = corpus
        .iter()
        .map(|d| (score_decision(d, &query_lower, &tokens), d))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by_key(|(s, _)| Reverse(*s));
    scored.truncate(MAX_DECISION_RESULTS);

    if scored.is_empty() {
        return Ok("## Decisions\n\nNo decisions matched the query.\n".to_string());
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Decisions matching '{query}'\n");
    for (i, (score, d)) in scored.iter().enumerate() {
        let status = match d.status {
            Status::Proposed => "proposed",
            Status::Accepted => "accepted",
            Status::Superseded => "superseded",
            Status::Deprecated => "deprecated",
        };
        let _ = writeln!(output, "## {} (`{}`, {})", d.title, status, d.date);
        let _ = writeln!(output, "**ID:** `{}`  ", d.id);
        let _ = writeln!(output, "**Match score:** {score}");
        let _ = writeln!(output, "\n### Context\n\n{}\n", d.context);
        let _ = writeln!(output, "### Decision\n\n{}\n", d.decision);
        if !d.alternatives_considered.is_empty() {
            let _ = writeln!(output, "### Alternatives Considered\n");
            for alt in &d.alternatives_considered {
                let _ = writeln!(output, "- **{}** — {}", alt.option, alt.why_rejected);
            }
            let _ = writeln!(output);
        }
        let _ = writeln!(output, "### Consequences\n\n{}\n", d.consequences);
        if !d.related_axioms.is_empty() {
            let _ = writeln!(
                output,
                "**Related axioms:** {}",
                d.related_axioms.join(", ")
            );
        }
        if !d.related_files.is_empty() {
            let _ = writeln!(
                output,
                "**Related files:** {}",
                d.related_files.join(", ")
            );
        }
        if i + 1 < scored.len() {
            let _ = writeln!(output, "\n---\n");
        }
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/illu_style_sample/.illu/style/decisions")
    }

    fn fixture_corpus() -> Vec<Decision> {
        load_from_dir(&fixture_dir()).unwrap()
    }

    #[test]
    fn test_decisions_parses_empty_dir() {
        let tmp = std::env::temp_dir().join("illu_decisions_empty_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let loaded = load_from_dir(&tmp).unwrap();
        assert!(loaded.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_decisions_parses_fixture() {
        let corpus = fixture_corpus();
        assert_eq!(corpus.len(), 3, "fixture has 3 decisions");
        let statuses: Vec<Status> = corpus.iter().map(|d| d.status).collect();
        assert!(statuses.contains(&Status::Accepted));
        assert!(statuses.contains(&Status::Superseded));
        assert!(statuses.contains(&Status::Proposed));
    }

    #[test]
    fn test_decisions_id_namespace() {
        for d in fixture_corpus() {
            assert!(
                d.id.starts_with("decision_"),
                "decision id `{}` must start with `decision_`",
                d.id
            );
        }
    }

    #[test]
    fn test_decisions_unique_ids() {
        let corpus = fixture_corpus();
        let mut seen = HashSet::new();
        for d in &corpus {
            assert!(seen.insert(d.id.clone()), "duplicate id `{}`", d.id);
        }
    }

    #[test]
    fn test_decisions_related_axioms_resolve() {
        let universal = crate::server::tools::axioms::axioms_for_test();
        let known: HashSet<&str> = universal.iter().map(|a| a.id.as_str()).collect();
        for d in fixture_corpus() {
            for axiom_id in &d.related_axioms {
                assert!(
                    known.contains(axiom_id.as_str()),
                    "decision `{}` references unknown axiom `{}`",
                    d.id,
                    axiom_id
                );
            }
        }
    }

    #[test]
    fn test_decisions_status_enum_validates() {
        let bad = r#"{
            "id": "decision_bad_status",
            "title": "X",
            "status": "not-a-status",
            "date": "2026-01-01",
            "context": "x",
            "decision": "x",
            "consequences": "x"
        }"#;
        assert!(parse_decision(bad).is_err());
    }

    #[test]
    fn test_decisions_date_validation() {
        // Reject malformed dates.
        let bad = r#"{
            "id": "decision_bad_date",
            "title": "X",
            "status": "accepted",
            "date": "April 2026",
            "context": "x",
            "decision": "x",
            "consequences": "x"
        }"#;
        let err = parse_decision(bad).unwrap_err();
        assert!(format!("{err}").contains("YYYY-MM-DD"));

        // Accept well-formed dates (no calendar correctness check).
        assert!(is_iso_date("2026-04-15"));
        assert!(is_iso_date("2026-02-30")); // not a real date but the format is right
        assert!(!is_iso_date("2026-4-15"));
        assert!(!is_iso_date("April 2026"));
    }

    #[test]
    fn test_handle_decisions_focused_query() {
        // Use the fixture's first decision as the query target.
        let result = handle_decisions("enum dispatch handlers vtable Box dyn").unwrap();
        assert!(
            result.contains("decision_use_enum_dispatch_for_handlers"),
            "enum-dispatch decision must surface; got: {result}"
        );
    }

    #[test]
    fn test_handle_decisions_demo_query() {
        // Broad query touching all three records.
        let result =
            handle_decisions("dispatch mutex async runtime alternatives chosen rejected")
                .unwrap();
        let expected_ids = [
            "decision_use_enum_dispatch_for_handlers",
            "decision_replace_mutex_per_row",
            "decision_evaluate_smol_runtime",
        ];
        let surfaced = expected_ids
            .iter()
            .filter(|id| result.contains(*id))
            .count();
        assert!(
            surfaced >= 2,
            "expected at least 2 fixture decisions in demo query; got {surfaced}; result: {result}"
        );
    }
}
```

- [ ] **Step 4: Wire the new module in `src/server/tools/mod.rs`**

Add `pub mod decisions;` alphabetically.

- [ ] **Step 5: Init `decisions` at server startup in `src/server/mod.rs`**

In `IlluServer::new`, alongside the existing `tools::project_style::init` call:

```rust
if let Err(e) = tools::decisions::init(&repo_root) {
    tracing::warn!(error = ?e, "failed to load .illu/style/decisions/; proceeding without decisions");
}
```

- [ ] **Step 6: Register `mcp__illu__decisions` tool in `src/server/mod.rs`**

```rust
#[derive(Deserialize, JsonSchema)]
struct DecisionsParams {
    /// Search term for decisions
    query: String,
}
```

```rust
#[tool(
    name = "decisions",
    description = "Query the project's design records loaded from `.illu/style/decisions/`. Returns up to 4 ADR-style decisions matching the query, with full context/decision/alternatives/consequences sections plus related axiom and file links. Use this to recover the rationale behind project-specific architectural choices."
)]
async fn decisions(
    &self,
    Parameters(params): Parameters<DecisionsParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(query = %params.query, "Tool call: decisions");
    let _guard = crate::status::StatusGuard::new(&format!("decisions ▸ {}", params.query));
    let result = tools::decisions::handle_decisions(&params.query).map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

- [ ] **Step 7: Re-export in `src/api.rs`**

```rust
pub mod decisions {
    pub use crate::server::tools::decisions::handle_decisions;
}
```

- [ ] **Step 8: Run cargo gauntlet**

```bash
cargo build
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

All must pass. The 8 new tests in `decisions::tests` should pass; the existing 620 tests should be unaffected.

- [ ] **Step 9: Commit Task 1**

```bash
git add src/server/tools/decisions.rs src/server/tools/mod.rs src/server/mod.rs src/api.rs tests/fixtures/illu_style_sample/.illu/style/decisions
git commit -m "$(cat <<'EOF'
decisions: add Phase 4 design-record schema, loader, and mcp tool

Phase 4 introduces project-local ADR-style design records loaded from
`{repo}/.illu/style/decisions/<slug>.json`, surfaced via a new
mcp__illu__decisions tool.

- Multi-file directory (one decision per file, matching how ADRs are
  authored). Discovery and load from the same repo_root that
  project_style::init consumes.
- Schema: id (decision_* prefix), title, status (proposed/accepted/
  superseded/deprecated), date (YYYY-MM-DD regex-validated; no chrono
  dep), context, decision, alternatives_considered[] as typed
  {option, why_rejected}, consequences, optional related_axioms[]
  (test-time resolution invariant + startup tracing::warn for stale
  ids, parallel to Phase 3) and related_files[] (no validation).
- Per-file parse failures log warn and skip that file (vs Phase 3's
  all-or-nothing project.json) — a bad decision file should not
  disable the whole directory. Duplicate ids keep the first
  occurrence and warn about the second.
- handle_decisions tool with single query parameter; scoring mirrors
  axioms (per-token partial-match against title/context/decision/
  consequences/alternatives, full-query equality boost on title +
  status). MAX_DECISION_RESULTS = 4 (decision-sized).
- 8 tests + a 3-decision fixture extending the existing Phase 3
  illu_style_sample (one record per non-deprecated status:
  accepted/superseded/proposed).

Closes Phase 4 (design record). Phase 4.1+ defers superseded_by typed
field, hot-reload, and status-filtered queries.
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

- [ ] **Step 2: Plan-reconciliation pass before final review** — if any content fix-ups landed during execution (axiom IDs adjusted, date validation tweaks, formatter shape changed), update the plan's draft sections to match.

---

## Verification Summary

After all tasks:
- 1 new module (`decisions.rs`, ~500 lines incl. tests).
- 1 new MCP tool (`decisions`) registered + re-exported.
- 1 fixture directory (3 decisions).
- 8 new tests.
- Cargo gauntlet clean.

## Risks Realized During Execution

- **Axiom ID guesswork in the fixture.** Mitigation: explicit verification step in Task 1 Step 2 + test-time cross-reference test.
- **`chrono`/`time` dependency creep.** Mitigation: explicit regex-light date validation; don't accept a calendar-correctness PR from the implementer.
- **Plan drift** between drafts and post-fix files (recurring across all prior phases). Reconcile before final review.
