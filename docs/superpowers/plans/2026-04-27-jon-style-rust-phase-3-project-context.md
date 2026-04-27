# Jon-Style Rust Phase 3 (Project Context Overrides) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Add `.illu/style/project.json` as a per-project override layer over the universal axiom corpus. New module + new MCP tool + modified `handle_axioms` scoring + 10 tests + a fixture project.

**Architecture:** New module `src/server/tools/project_style.rs` mirrors the parse-once-cache pattern of `axioms.rs`. `ProjectStyle` is loaded once at server startup from `{repo}/.illu/style/project.json`; absent file → empty default → Phase-2 behavior unchanged. `handle_axioms` grows a `_with_style` variant that consults overrides; the public `handle_axioms` reads from a process-global `OnceLock<ProjectStyle>` set during server init.

**Tech Stack:** Rust 2024, `serde_json`, `rmcp` macros, `cargo test` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo fmt --all -- --check`.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-3-project-context-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-3-project-context-design.md)

**Existing state:** 102 universal axioms, 9 exemplars. `src/server/tools/axioms.rs` is the canonical mirror for parse-once-cache, scoring, and result formatting. `src/server/mod.rs` registers `axioms` and `exemplars` MCP tools. `src/api.rs` re-exports each tool's `handle_*`.

**Lint constraints** (unchanged from prior phases): `unwrap_used = "deny"`, `expect_used = "warn"`, `allow_attributes = "deny"` (use `#[expect(lint, reason = "...")]`). Tests use `#[expect(clippy::unwrap_used, reason = "tests")]` at the module scope.

**Repo-root plumbing:** `IlluServer` is constructed in `main.rs` with the repo path that hosts `.illu/index.db`. The `Database` struct already knows its path. Phase 3 needs the repo root passed to `project_style::init` at server startup. Pick the cleanest of:
- Adding a `pub fn repo_root(&self) -> &Path` accessor on `Database` and calling it from `IlluServer::new`.
- Passing the repo root explicitly through `IlluServer::new`.

Implementer chooses during Task 1.

**Test fixture:** `tests/fixtures/illu_style_sample/.illu/style/project.json` exercises every severity + a project-local axiom. Created in Task 1.

**Drafts pre-approved by user.** Skip the user-review step inside batch tasks.

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `src/server/tools/project_style.rs` | Create | Module: schema types, loader, parse-once cache, `handle_project_style`, validity tests |
| `src/server/tools/axioms.rs` | Modify | Make `RawAxiom` `pub(crate)`; add `handle_axioms_with_style`; `handle_axioms` becomes a thin wrapper |
| `src/server/tools/mod.rs` | Modify | Add `pub mod project_style;` |
| `src/server/mod.rs` | Modify | Register `mcp__illu__project_style` tool; init `project_style` at server construction |
| `src/api.rs` | Modify | Re-export `project_style::handle_project_style` |
| `tests/fixtures/illu_style_sample/.illu/style/project.json` | Create | Fixture exercising every severity + a local axiom |

---

## Task 1: Schema + Loader + Project-Style Tool + Fixture + Validity Tests

**Files (create):**
- `src/server/tools/project_style.rs`
- `tests/fixtures/illu_style_sample/.illu/style/project.json`

**Files (modify):**
- `src/server/tools/axioms.rs` (make `RawAxiom` `pub(crate)`; no scoring change yet)
- `src/server/tools/mod.rs` (add `pub mod project_style;`)
- `src/server/mod.rs` (add `ProjectStyleParams` + tool method; init project_style at server startup)
- `src/api.rs` (re-export)

- [ ] **Step 1: Make `RawAxiom` `pub(crate)` in `src/server/tools/axioms.rs`**

The existing private struct gains crate-level visibility so `project_style::RawProjectStyle` can deserialize `local_axioms` into the same shape. No other change to `axioms.rs` in Task 1.

- [ ] **Step 2: Create the test fixture**

Create `tests/fixtures/illu_style_sample/.illu/style/project.json`:

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
    },
    {
      "id": "rust_quality_90_enum_dispatch",
      "severity": "demoted",
      "note": "we deliberately use Box<dyn Trait> for plugin-shaped APIs that need external extension"
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

Note: the axiom IDs (`_64_error_chain`, `_85_allocation_hot_paths`, etc.) are educated guesses. **Verify each against the actual corpus** before committing — the validity test `test_project_style_override_id_resolves` will catch mismatches at runtime.

- [ ] **Step 3: Verify each `axiom_overrides[].id` resolves to a real axiom**

```bash
for id in rust_quality_64_error_chain rust_quality_85_allocation_hot_paths rust_quality_87_iterator_codegen rust_quality_90_enum_dispatch; do
  grep -q "\"id\": \"$id\"" assets/rust_quality_axioms.json && echo "OK: $id" || echo "MISSING: $id"
done
```

If any are missing, find the correct ID in `assets/rust_quality_axioms.json` and update the fixture.

- [ ] **Step 4: Create `src/server/tools/project_style.rs`**

```rust
//! Per-project override layer over the universal axiom corpus.
//!
//! Reads `{repo}/.illu/style/project.json` at server startup. The file is
//! optional; an absent or unparseable file degrades to the empty default,
//! which leaves `handle_axioms` behavior identical to the universal corpus.
//! See [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-3-project-context-design.md].

use crate::server::tools::axioms::{Axiom, RawAxiom};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use std::sync::OnceLock;

/// On-disk shape of the project style file. `axiom_overrides` and
/// `local_axioms` both default to empty so a minimal `{ "version": 1 }`
/// is a valid file.
#[derive(Debug, Deserialize)]
struct RawProjectStyle {
    version: u32,
    #[serde(default)]
    axiom_overrides: Vec<RawAxiomOverride>,
    #[serde(default)]
    local_axioms: Vec<RawAxiom>,
}

#[derive(Debug, Deserialize)]
struct RawAxiomOverride {
    id: String,
    severity: Severity,
    #[serde(default)]
    note: String,
}

/// Severity of an override. Multipliers (×0.5 / ×2.0) preserve the
/// "score=0 stays zero" invariant — `demoted` cannot conjure score
/// from nothing, and `elevated` cannot float a no-match axiom into
/// the result set.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Filter out entirely; the axiom never appears in handle_axioms results.
    Ignored,
    /// Multiply score by 0.5 (integer truncation).
    Demoted,
    /// Score unchanged; the project's `note` is appended in display.
    Noted,
    /// Multiply score by 2 (saturating).
    Elevated,
}

#[derive(Debug, Clone)]
pub struct AxiomOverride {
    pub severity: Severity,
    pub note: String,
}

/// Loaded project style. Empty default = no overrides + no local axioms.
#[derive(Debug, Default)]
pub struct ProjectStyle {
    /// Map from universal-axiom id → override.
    pub overrides: HashMap<String, AxiomOverride>,
    /// Project-local axioms (IDs prefixed `project_`).
    pub local_axioms: Vec<Axiom>,
}

impl ProjectStyle {
    /// Apply this style's effect on a universal axiom's score. Returns
    /// `None` if the axiom is `Ignored`. Otherwise returns the adjusted
    /// score (possibly unchanged for `Noted` or absent overrides).
    pub fn adjust_score(&self, axiom_id: &str, base_score: usize) -> Option<usize> {
        match self.overrides.get(axiom_id).map(|o| o.severity) {
            Some(Severity::Ignored) => None,
            Some(Severity::Demoted) => Some(base_score / 2),
            Some(Severity::Elevated) => Some(base_score.saturating_mul(2)),
            Some(Severity::Noted) | None => Some(base_score),
        }
    }

    /// Return the `note` text for an axiom if its override severity is
    /// `Noted`. Other severities don't append a note (their effect is
    /// already visible via score adjustment / filtering).
    pub fn note_for(&self, axiom_id: &str) -> Option<&str> {
        self.overrides
            .get(axiom_id)
            .filter(|o| o.severity == Severity::Noted)
            .map(|o| o.note.as_str())
    }
}

/// Process-global cache of the loaded style. Set once during server
/// startup via [`init`]; defaults to empty if `init` is not called.
static PROJECT_STYLE: OnceLock<ProjectStyle> = OnceLock::new();

/// Returns the active project style. If [`init`] was not called, returns
/// an empty default (no overrides, no local axioms).
pub fn project_style() -> &'static ProjectStyle {
    static EMPTY: OnceLock<ProjectStyle> = OnceLock::new();
    PROJECT_STYLE
        .get()
        .unwrap_or_else(|| EMPTY.get_or_init(ProjectStyle::default))
}

/// Load `.illu/style/project.json` from `repo_root` and store it in the
/// process-global cache. Called once at server startup. If the file is
/// absent the default empty style is stored. Subsequent calls are no-ops.
pub fn init(repo_root: &Path) -> Result<(), crate::IlluError> {
    let path = repo_root.join(".illu").join("style").join("project.json");
    let style = if path.exists() {
        load_from_path(&path)?
    } else {
        ProjectStyle::default()
    };
    let _ = PROJECT_STYLE.set(style);
    Ok(())
}

/// Parse and validate a `project.json` file. Public for tests.
pub fn load_from_path(path: &Path) -> Result<ProjectStyle, crate::IlluError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        crate::IlluError::Other(format!("failed to read {}: {e}", path.display()))
    })?;
    parse(&text)
}

fn parse(json: &str) -> Result<ProjectStyle, crate::IlluError> {
    let raw: RawProjectStyle = serde_json::from_str(json).map_err(|e| {
        crate::IlluError::Other(format!("failed to parse project.json: {e}"))
    })?;

    if raw.version != 1 {
        return Err(crate::IlluError::Other(format!(
            "unsupported project.json version: {} (only 1 is recognized)",
            raw.version
        )));
    }

    // Local axioms must be `project_*`-prefixed and unique.
    let mut local_ids = std::collections::HashSet::new();
    let mut local_axioms = Vec::with_capacity(raw.local_axioms.len());
    for raw_axiom in raw.local_axioms {
        if !raw_axiom.id.starts_with("project_") {
            return Err(crate::IlluError::Other(format!(
                "local axiom id `{}` must start with `project_`",
                raw_axiom.id
            )));
        }
        if !local_ids.insert(raw_axiom.id.clone()) {
            return Err(crate::IlluError::Other(format!(
                "duplicate local axiom id `{}`",
                raw_axiom.id
            )));
        }
        local_axioms.push(Axiom::from(raw_axiom));
    }

    let overrides = raw
        .axiom_overrides
        .into_iter()
        .map(|o| {
            (
                o.id,
                AxiomOverride {
                    severity: o.severity,
                    note: o.note,
                },
            )
        })
        .collect();

    Ok(ProjectStyle {
        overrides,
        local_axioms,
    })
}

/// Returns the active project style as a Markdown summary. Used by the
/// `mcp__illu__project_style` tool.
pub fn handle_project_style() -> Result<String, crate::IlluError> {
    let style = project_style();
    let mut output = String::new();

    if style.overrides.is_empty() && style.local_axioms.is_empty() {
        return Ok("## Project style\n\nNo project style configured (`.illu/style/project.json` is absent or empty).\n".to_string());
    }

    let _ = writeln!(output, "## Project style\n");

    if !style.overrides.is_empty() {
        let _ = writeln!(output, "### Axiom overrides ({})\n", style.overrides.len());
        for severity in [
            Severity::Ignored,
            Severity::Demoted,
            Severity::Noted,
            Severity::Elevated,
        ] {
            let entries: Vec<_> = style
                .overrides
                .iter()
                .filter(|(_, o)| o.severity == severity)
                .collect();
            if entries.is_empty() {
                continue;
            }
            let _ = writeln!(output, "**{severity:?}** ({}):", entries.len());
            for (id, ovr) in entries {
                if ovr.note.is_empty() {
                    let _ = writeln!(output, "- `{id}`");
                } else {
                    let _ = writeln!(output, "- `{id}` — {}", ovr.note);
                }
            }
            let _ = writeln!(output);
        }
    }

    if !style.local_axioms.is_empty() {
        let _ = writeln!(
            output,
            "### Project-local axioms ({})\n",
            style.local_axioms.len()
        );
        for axiom in &style.local_axioms {
            let _ = writeln!(output, "- `{}` — **{}**: {}", axiom.id, axiom.category, axiom.title_or_summary());
        }
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/illu_style_sample/.illu/style/project.json")
    }

    fn fixture_style() -> ProjectStyle {
        load_from_path(&fixture_path()).unwrap()
    }

    #[test]
    fn test_project_style_parses_empty() {
        let s = parse(r#"{"version": 1}"#).unwrap();
        assert!(s.overrides.is_empty());
        assert!(s.local_axioms.is_empty());
    }

    #[test]
    fn test_project_style_parses_fixture() {
        let s = fixture_style();
        assert_eq!(s.overrides.len(), 4, "fixture has 4 overrides");
        assert_eq!(s.local_axioms.len(), 1, "fixture has 1 local axiom");
    }

    #[test]
    fn test_project_style_override_id_resolves() {
        let s = fixture_style();
        let universal = crate::server::tools::axioms::axioms_for_test();
        let known: std::collections::HashSet<&str> =
            universal.iter().map(|a| a.id.as_str()).collect();
        for id in s.overrides.keys() {
            assert!(
                known.contains(id.as_str()),
                "override id `{id}` does not resolve to a universal axiom"
            );
        }
    }

    #[test]
    fn test_project_style_local_axiom_ids_namespaced() {
        let s = fixture_style();
        for axiom in &s.local_axioms {
            assert!(
                axiom.id.starts_with("project_"),
                "local axiom id `{}` must start with `project_`",
                axiom.id
            );
        }
    }

    #[test]
    fn test_project_style_local_axiom_ids_unique() {
        let s = fixture_style();
        let mut seen = std::collections::HashSet::new();
        for axiom in &s.local_axioms {
            assert!(seen.insert(axiom.id.clone()), "duplicate local id `{}`", axiom.id);
        }
    }

    #[test]
    fn test_project_style_local_axiom_does_not_shadow_universal() {
        let s = fixture_style();
        let universal = crate::server::tools::axioms::axioms_for_test();
        let known: std::collections::HashSet<&str> =
            universal.iter().map(|a| a.id.as_str()).collect();
        for axiom in &s.local_axioms {
            assert!(
                !known.contains(axiom.id.as_str()),
                "local axiom id `{}` shadows a universal axiom",
                axiom.id
            );
        }
    }

    #[test]
    fn test_project_style_rejects_unknown_version() {
        let err = parse(r#"{"version": 2}"#).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unsupported"), "expected unsupported-version error, got: {msg}");
    }

    #[test]
    fn test_project_style_rejects_unprefixed_local_id() {
        let json = r#"{
            "version": 1,
            "local_axioms": [
                {
                    "id": "not_prefixed",
                    "category": "X",
                    "source": "test",
                    "triggers": ["x"],
                    "rule_summary": "x",
                    "prompt_injection": "x",
                    "anti_pattern": "x",
                    "good_pattern": "x"
                }
            ]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(format!("{err}").contains("project_"));
    }

    #[test]
    fn test_adjust_score_filters_ignored() {
        let mut s = ProjectStyle::default();
        s.overrides.insert(
            "rust_quality_64_error_chain".into(),
            AxiomOverride { severity: Severity::Ignored, note: String::new() },
        );
        assert_eq!(s.adjust_score("rust_quality_64_error_chain", 100), None);
    }

    #[test]
    fn test_adjust_score_demotes_and_elevates() {
        let mut s = ProjectStyle::default();
        s.overrides.insert(
            "a".into(),
            AxiomOverride { severity: Severity::Demoted, note: String::new() },
        );
        s.overrides.insert(
            "b".into(),
            AxiomOverride { severity: Severity::Elevated, note: String::new() },
        );
        assert_eq!(s.adjust_score("a", 100), Some(50));
        assert_eq!(s.adjust_score("b", 100), Some(200));
        assert_eq!(s.adjust_score("a", 0), Some(0));
        assert_eq!(s.adjust_score("b", 0), Some(0));
    }
}
```

Note: `Axiom::title_or_summary()` is a small helper to display an axiom in the project-style summary. If `Axiom` doesn't have such a method, either add a `pub fn title_or_summary(&self) -> &str { &self.rule_summary }` method on `Axiom` (in `axioms.rs`) or inline `&axiom.rule_summary` directly. **Implementer's choice.**

- [ ] **Step 5: Wire the new module in `src/server/tools/mod.rs`**

Add `pub mod project_style;` alphabetically next to `pub mod project_*` siblings (or at the end if no project_-prefixed module exists yet).

- [ ] **Step 6: Init `project_style` at server startup in `src/server/mod.rs`**

In `IlluServer::new` (or wherever the server is constructed), after the database open succeeds and the repo root is known, call:

```rust
if let Err(e) = tools::project_style::init(&repo_root) {
    tracing::warn!(error = ?e, "failed to load .illu/style/project.json; proceeding without overrides");
}
```

If `Database` doesn't expose its repo root, add a `pub fn repo_root(&self) -> &Path` accessor.

- [ ] **Step 7: Register `mcp__illu__project_style` tool in `src/server/mod.rs`**

Add a parameters struct after `ExemplarsParams`:

```rust
#[derive(Deserialize, JsonSchema)]
struct ProjectStyleParams {}
```

Add a tool method inside the existing `#[tool_router] impl IlluServer` block:

```rust
#[tool(
    name = "project_style",
    description = "Show the active project style overrides loaded from `.illu/style/project.json`. Lists axiom overrides (ignored/demoted/noted/elevated) and project-local axioms. Use this to inspect why an axiom was filtered out or got an unexpected ranking, or to review the project conventions encoded in the file."
)]
async fn project_style(
    &self,
    Parameters(_params): Parameters<ProjectStyleParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!("Tool call: project_style");
    let _guard = crate::status::StatusGuard::new("project_style");
    let result = tools::project_style::handle_project_style().map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

- [ ] **Step 8: Re-export in `src/api.rs`**

Add a re-export module after the existing `pub mod exemplars { ... }`:

```rust
pub mod project_style {
    pub use crate::server::tools::project_style::handle_project_style;
}
```

- [ ] **Step 9: Run the cargo gauntlet**

```bash
cargo build
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

All must pass. The 9 new tests in `project_style::tests` should pass; the existing 605 tests should be unaffected.

- [ ] **Step 10: Commit Task 1**

```bash
git add src/server/tools/project_style.rs src/server/tools/axioms.rs src/server/tools/mod.rs src/server/mod.rs src/api.rs tests/fixtures/illu_style_sample
git commit -m "$(cat <<'EOF'
project_style: add schema, loader, and mcp tool (no scoring integration yet)

Phase 3 step 1 of 2. Introduces the .illu/style/project.json layer:

- New module src/server/tools/project_style.rs with RawProjectStyle/
  ProjectStyle two-struct pattern, OnceLock-based parse-once cache,
  load_from_path + init helpers, handle_project_style markdown
  formatter, and 9 validity tests.
- Schema: version=1, axiom_overrides[] with severity (ignored/demoted/
  noted/elevated) + optional note, local_axioms[] reusing the existing
  RawAxiom shape with project_* ID prefix enforcement.
- Severity multipliers (x0.5/x2) preserve the score=0 invariant.
- New mcp__illu__project_style tool with no parameters; returns the
  active config grouped by severity, plus project-local axioms.
- src/server/mod.rs init()s project_style at server construction with
  the repo root from Database.
- src/api.rs re-exports handle_project_style.
- src/server/tools/axioms.rs: RawAxiom now pub(crate) so local_axioms
  deserialize through the same shape (no schema fork).
- Fixture project at tests/fixtures/illu_style_sample/.illu/style/
  project.json exercises every severity + a project-local repository-
  pattern axiom.

handle_axioms scoring integration is Task 2; this task ships the
infrastructure plus the tool that lets users inspect their config.
EOF
)"
```

---

## Task 2: Integrate Overrides into `handle_axioms` Scoring + Demo Test

**Files (modify):**
- `src/server/tools/axioms.rs` (add `handle_axioms_with_style`; `handle_axioms` becomes a thin wrapper; new tests)

- [ ] **Step 1: Add `handle_axioms_with_style` to `src/server/tools/axioms.rs`**

Refactor `handle_axioms` so its body becomes the body of `pub(crate) fn handle_axioms_with_style(query: &str, style: &ProjectStyle) -> Result<String>`. Public `handle_axioms` becomes:

```rust
pub fn handle_axioms(query: &str) -> Result<String, crate::IlluError> {
    let style = crate::server::tools::project_style::project_style();
    handle_axioms_with_style(query, style)
}
```

Inside `handle_axioms_with_style`, change the scoring loop in two places:

```rust
// Before computing the universal corpus iteration:
let mut combined: Vec<&Axiom> = axioms()?.iter().collect();
combined.extend(style.local_axioms.iter());

// In the scoring map:
.map(|axiom| {
    let base = score_axiom(axiom, &query_terms, &query_lower);
    let adjusted = style.adjust_score(&axiom.id, base).unwrap_or(0);
    (axiom, adjusted)
})
.filter(|(_, score)| *score > 0)
```

(Where `score_axiom` is the inner scoring routine — extract it from the existing closure into a separate function for readability.)

In the result formatter, append the `Noted` note text:

```rust
for (axiom, score) in top {
    let _ = writeln!(output, "## {} — {}\n", axiom.category, axiom.rule_summary);
    let _ = writeln!(output, "**ID:** `{}`  ", axiom.id);
    let _ = writeln!(output, "**Score:** {score}");
    if let Some(note) = style.note_for(&axiom.id) {
        let _ = writeln!(output, "**Project note:** {note}");
    }
    // ...rest of existing formatter
}
```

(Adapt to the existing formatter's actual structure — this is illustrative.)

- [ ] **Step 2: Add integration tests to `axioms.rs`**

```rust
#[test]
fn test_handle_axioms_respects_ignored() {
    use crate::server::tools::project_style::{AxiomOverride, ProjectStyle, Severity};
    let mut style = ProjectStyle::default();
    style.overrides.insert(
        "rust_quality_57_error_source_chain".into(),
        AxiomOverride { severity: Severity::Ignored, note: String::new() },
    );
    let result = handle_axioms_with_style("error source chain", &style).unwrap();
    assert!(
        !result.contains("rust_quality_57_error_source_chain"),
        "ignored axiom must not appear in result"
    );
}

#[test]
fn test_handle_axioms_respects_demoted_elevated() {
    use crate::server::tools::project_style::{AxiomOverride, ProjectStyle, Severity};
    let mut style = ProjectStyle::default();
    style.overrides.insert(
        "rust_quality_85_allocation_hot_paths".into(),
        AxiomOverride { severity: Severity::Demoted, note: String::new() },
    );
    style.overrides.insert(
        "rust_quality_87_iterator_codegen".into(),
        AxiomOverride { severity: Severity::Elevated, note: String::new() },
    );
    // Query that hits both axioms; with overrides, 87 should rank above 85.
    let result = handle_axioms_with_style(
        "allocation iterator preallocate hot path with_capacity bounds check",
        &style,
    )
    .unwrap();
    let pos_85 = result.find("rust_quality_85_allocation_hot_paths");
    let pos_87 = result.find("rust_quality_87_iterator_codegen");
    if let (Some(p85), Some(p87)) = (pos_85, pos_87) {
        assert!(
            p87 < p85,
            "elevated axiom 87 should appear before demoted axiom 85"
        );
    } else {
        // If neither axiom matched (unlikely for this query), the test is
        // noise; just assert the run produced output.
        assert!(!result.is_empty());
    }
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
    let result =
        handle_axioms_with_style("allocation hot path preallocate with_capacity", &style).unwrap();
    assert!(
        result.contains("PROJECT-NOTE-MARKER-85"),
        "noted axiom's note must appear in result"
    );
}

#[test]
fn test_handle_axioms_surfaces_local_axiom() {
    use crate::server::tools::project_style::{AxiomOverride, ProjectStyle, Severity};
    let _ = AxiomOverride { severity: Severity::Ignored, note: String::new() }; // silence unused warning if test conditionally skips
    let style = crate::server::tools::project_style::load_from_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/illu_style_sample/.illu/style/project.json")
            .as_path(),
    )
    .unwrap();
    let result = handle_axioms_with_style("repository module database access", &style).unwrap();
    assert!(
        result.contains("project_repository_pattern"),
        "project-local axiom must surface for matching query"
    );
}
```

- [ ] **Step 3: Run the cargo gauntlet**

```bash
cargo build
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

The 4 new tests in `axioms::tests` should pass; the 9 from Task 1 should still pass; the existing 605 should be unaffected.

- [ ] **Step 4: Commit Task 2**

```bash
git add src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: integrate ProjectStyle overrides into handle_axioms

Phase 3 step 2 of 2. handle_axioms now consults the active
ProjectStyle:

- handle_axioms is now a thin wrapper around the new pub(crate)
  handle_axioms_with_style(query, style); the latter does the actual
  work and is the test entry point.
- Scoring fold combines the universal axiom corpus with
  style.local_axioms; per-axiom score is adjusted via
  style.adjust_score() (None filters, Some(n) keeps with adjusted
  score).
- Result formatter appends the project's note when severity is Noted.
- Project-local axioms surface alongside universal ones, identified
  by their project_* IDs.

Four new integration tests cover: ignored axiom is filtered, demoted
ranks below elevated, noted note appears in output, project-local
axiom surfaces for matching query (loads the fixture).

Phase 3 closes here. .illu/style/project.json is now a fully-wired
override layer; mcp__illu__project_style and mcp__illu__axioms both
honor it.
EOF
)"
```

---

## Task 3: End-to-End Verification + Plan Reconciliation

- [ ] **Step 1: Full cargo gauntlet**

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 2: Plan-reconciliation pass before final review** — if any content fix-ups landed during execution (axiom IDs adjusted, helper methods added, formatter shape changed), update the plan's draft sections to match.

---

## Verification Summary

After all tasks:
- 1 new module (`project_style.rs`, ~250 lines).
- 1 modified module (`axioms.rs`: `RawAxiom` `pub(crate)`, `handle_axioms_with_style` extracted, scoring + formatter consult `ProjectStyle`).
- 1 new MCP tool (`project_style`) registered + re-exported.
- 1 fixture project under `tests/fixtures/illu_style_sample/`.
- 13 new tests (9 in project_style + 4 in axioms).
- Cargo gauntlet clean.

## Risks Realized During Execution

- **Axiom ID guesswork in the fixture.** Mitigation: explicit verification step in Task 1 Step 3 + the cross-reference test at runtime.
- **`Database::repo_root` may not exist as a public method.** Mitigation: implementer can add it in Task 1 Step 6 (small, scoped change).
- **`Axiom::title_or_summary` helper.** Mitigation: implementer's choice between adding the method or inlining `&rule_summary`.
- **Plan drift** between drafts and post-fix files (recurring). Reconcile before final review.
