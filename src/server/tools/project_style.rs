//! Per-project override layer over the universal axiom corpus.
//!
//! Reads `{repo}/.illu/style/project.json` at server startup. The file is
//! optional; an absent or unparseable file degrades to the empty default,
//! which leaves `handle_axioms` behavior identical to the universal corpus.
//!
//! The schema is `version: 1` plus two arrays:
//!
//! - `axiom_overrides[]`: each entry references a universal axiom by `id`
//!   and tags it with a [`Severity`] (`ignored`/`demoted`/`noted`/
//!   `elevated`) and an optional `note`. Severities are *multipliers*
//!   (×0.5 / ×2.0 / passthrough) or filters; they cannot conjure score
//!   from nothing, which preserves the `score == 0 stays zero` invariant
//!   of `handle_axioms` scoring.
//! - `local_axioms[]`: project-defined axioms that participate in the
//!   same scoring as universal axioms. Their `id` MUST start with
//!   `project_` so a flat namespace cannot collide with universal IDs.
//!
//! See `docs/superpowers/specs/2026-04-27-jon-style-rust-phase-3-project-context-design.md`.

use crate::server::tools::axioms::{Axiom, RawAxiom};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use std::sync::OnceLock;

/// On-disk shape of the project style file. `axiom_overrides` and
/// `local_axioms` both default to empty, so `{ "version": 1 }` alone is a
/// valid (no-op) configuration.
#[derive(Debug, Deserialize)]
struct RawProjectStyle {
    version: u32,
    #[serde(default)]
    axiom_overrides: Vec<RawAxiomOverride>,
    #[serde(default)]
    local_axioms: Vec<RawAxiom>,
}

/// On-disk shape of a single axiom override. `note` is optional; when
/// severity is [`Severity::Noted`] the note text is what surfaces in
/// `handle_axioms` output (Phase 3 Task 2).
#[derive(Debug, Deserialize)]
struct RawAxiomOverride {
    id: String,
    severity: Severity,
    #[serde(default)]
    note: String,
}

/// Severity of a per-project axiom override.
///
/// The four variants are intentionally chosen so any score adjustment is
/// *monotone* with respect to the base score: a base of 0 stays 0 in every
/// case. `Demoted` cannot conjure a hit out of nothing, and `Elevated`
/// cannot float a no-match axiom into the result set.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Severity {
    /// Filter the axiom out entirely; it never appears in `handle_axioms`
    /// results regardless of how strongly the query matches.
    Ignored,
    /// Multiply the matched score by 0.5 (integer truncation), so a
    /// demoted axiom still surfaces if the user explicitly searches for
    /// its triggers but ranks below comparable un-demoted axioms.
    Demoted,
    /// Score is unchanged; the project's `note` is appended in display.
    /// Use this when a project wants to keep the universal advice but
    /// add organisation-specific context the model should consider.
    Noted,
    /// Multiply the matched score by 2 (saturating). Used to push an
    /// axiom toward the top of result lists when the project considers
    /// it especially load-bearing.
    Elevated,
}

/// Resolved override entry stored in [`ProjectStyle::overrides`].
///
/// `note` is always present (empty string if the on-disk JSON omitted it),
/// which keeps the lookup paths in [`ProjectStyle::note_for`] branch-free
/// on the hot path.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AxiomOverride {
    pub severity: Severity,
    pub note: String,
}

/// Loaded, validated project style.
///
/// The empty default — no overrides, no local axioms — is the value used
/// when `.illu/style/project.json` is absent. `handle_axioms` keeps its
/// universal-corpus behavior unchanged in that case (Phase 3 Task 2 wires
/// the actual scoring integration).
///
/// `overrides` is a `HashMap<String, _>` because the only access pattern
/// is `axiom_id -> override`; iteration order does not affect any
/// observable output. `local_axioms` is a `Vec` because the scoring loop
/// iterates linearly and the display surface preserves declaration order.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ProjectStyle {
    /// Map from universal-axiom id to its override.
    pub overrides: HashMap<String, AxiomOverride>,
    /// Project-local axioms. By construction every id starts with
    /// `project_`, enforced in [`parse`].
    pub local_axioms: Vec<Axiom>,
}

impl ProjectStyle {
    /// Apply this style's effect on a universal axiom's score.
    ///
    /// Returns `None` when the axiom is [`Severity::Ignored`] (so the
    /// scorer can drop it from the result set). Otherwise returns the
    /// adjusted score; an axiom with no override or with severity
    /// [`Severity::Noted`] passes through unchanged. `Demoted` halves the
    /// score; `Elevated` doubles it (saturating to guard against an
    /// already-near-`usize::MAX` score, even though the production scorer
    /// stays well below that).
    #[must_use]
    pub fn adjust_score(&self, axiom_id: &str, base_score: usize) -> Option<usize> {
        match self.overrides.get(axiom_id).map(|o| o.severity) {
            Some(Severity::Ignored) => None,
            Some(Severity::Demoted) => Some(base_score / 2),
            Some(Severity::Elevated) => Some(base_score.saturating_mul(2)),
            Some(Severity::Noted) | None => Some(base_score),
        }
    }

    /// Return the project's `note` for an axiom only when its override
    /// severity is [`Severity::Noted`].
    ///
    /// Other severities encode their effect via score adjustment or
    /// filtering, so duplicating the note in display would be redundant
    /// noise; `noted` is the variant that *exists* to surface text.
    #[must_use]
    pub fn note_for(&self, axiom_id: &str) -> Option<&str> {
        self.overrides
            .get(axiom_id)
            .filter(|o| o.severity == Severity::Noted)
            .map(|o| o.note.as_str())
    }
}

/// Process-global cache of the loaded style.
///
/// Set once during server startup via [`init`]; if `init` is never called
/// (e.g. unit tests that exercise the formatter without a server), the
/// accessor falls back to a separate empty `OnceLock` so callers always
/// see a valid `&'static ProjectStyle`.
static PROJECT_STYLE: OnceLock<ProjectStyle> = OnceLock::new();

/// Returns the active project style.
///
/// If [`init`] was not called, returns the empty default — no overrides,
/// no local axioms — which preserves Phase-2 `handle_axioms` behavior.
pub fn project_style() -> &'static ProjectStyle {
    static EMPTY: OnceLock<ProjectStyle> = OnceLock::new();
    PROJECT_STYLE
        .get()
        .unwrap_or_else(|| EMPTY.get_or_init(ProjectStyle::default))
}

/// Load `.illu/style/project.json` from `repo_root` and store the result
/// in the process-global cache.
///
/// Called once at server startup. If the file is absent the empty default
/// is stored, which means subsequent code paths can rely on
/// [`project_style`] returning a valid reference without re-checking the
/// filesystem. Subsequent calls are no-ops because [`OnceLock::set`]
/// rejects writes after the first success.
pub fn init(repo_root: &Path) -> Result<(), crate::IlluError> {
    let path = repo_root.join(".illu").join("style").join("project.json");
    let style = if path.exists() {
        load_from_path(&path)?
    } else {
        ProjectStyle::default()
    };

    // Warn about override entries whose id no longer resolves to a
    // universal axiom. The trust model says project files are tolerated
    // through corpus renames (so we don't fail server startup), but a
    // silent no-op leaves operators wondering why their override has
    // no effect — log so they can repair the file.
    if !style.overrides.is_empty()
        && let Ok(universal) = crate::server::tools::axioms::axioms_for_runtime()
    {
        let known: std::collections::HashSet<&str> =
            universal.iter().map(|a| a.id.as_str()).collect();
        for id in style.overrides.keys() {
            if !known.contains(id.as_str()) {
                tracing::warn!(
                    %id,
                    "axiom_overrides[].id does not resolve to a universal axiom; the override will silently no-op"
                );
            }
        }
    }

    // `init` is intended to be called exactly once per `IlluServer`
    // lifetime (from `IlluServer::new`). If a test calls `init` again
    // with a different path, the second call's value is discarded —
    // `OnceLock::set` rejects writes after the first success. Tests
    // that want to swap configs should use [`load_from_path`] directly
    // and pass the resulting `ProjectStyle` to scoring helpers.
    let _ = PROJECT_STYLE.set(style);
    Ok(())
}

/// Parse and validate a `project.json` file.
///
/// Public for tests that want to load arbitrary fixtures without going
/// through [`init`]. Production code should rely on [`project_style`]
/// after the server has called [`init`] once.
///
/// # Errors
///
/// Returns [`crate::IlluError::Other`] if the file cannot be read or if
/// its contents fail validation (see [`parse`] for the rules).
pub fn load_from_path(path: &Path) -> Result<ProjectStyle, crate::IlluError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::IlluError::Other(format!("failed to read {}: {e}", path.display())))?;
    parse(&text)
}

/// Parse and validate a `project.json` document.
///
/// Validation rules enforced here:
///
/// - `version` must be `1` (the only schema version recognised today).
/// - Every `local_axioms[].id` must start with `project_` so it cannot
///   collide with universal `rust_quality_*` ids.
/// - Local axiom ids must be unique within the file.
/// - The same id cannot appear in both `axiom_overrides` and
///   `local_axioms` — overriding a project-local axiom is meaningless
///   because the project owns it directly.
///
/// Two related invariants are enforced as *test*-time checks rather
/// than at parse time, because `parse` does not have access to the
/// universal axiom corpus:
///
/// - `axiom_overrides[].id` resolves to a real `rust_quality_*` axiom.
/// - `local_axioms[].id` does not shadow a universal `rust_quality_*`
///   axiom.
///
/// # Errors
///
/// Returns [`crate::IlluError::Other`] on JSON parse failure, version
/// mismatch, unprefixed local id, duplicate local id, or an id that
/// appears in both `axiom_overrides` and `local_axioms`.
fn parse(json: &str) -> Result<ProjectStyle, crate::IlluError> {
    let raw: RawProjectStyle = serde_json::from_str(json)
        .map_err(|e| crate::IlluError::Other(format!("failed to parse project.json: {e}")))?;

    if raw.version != 1 {
        return Err(crate::IlluError::Other(format!(
            "unsupported project.json version: {} (only 1 is recognized)",
            raw.version
        )));
    }

    // Local axioms must be `project_*`-prefixed (avoids namespace
    // collisions with universal `rust_quality_*` ids) and unique within
    // the file (no shadowing inside the same project).
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

    // An id may live in `axiom_overrides` (referring to a universal
    // axiom) or in `local_axioms` (a project-defined axiom), but not
    // both. The prefix discipline makes accidental collision unlikely,
    // but a project author could write `axiom_overrides[].id` with a
    // `project_*` value that also exists locally; reject that.
    let mut overrides = std::collections::HashMap::with_capacity(raw.axiom_overrides.len());
    for o in raw.axiom_overrides {
        if local_ids.contains(&o.id) {
            return Err(crate::IlluError::Other(format!(
                "id `{}` appears in both axiom_overrides and local_axioms; \
                 overriding a project-local axiom is meaningless because \
                 the project owns it directly",
                o.id
            )));
        }
        overrides.insert(
            o.id,
            AxiomOverride {
                severity: o.severity,
                note: o.note,
            },
        );
    }

    Ok(ProjectStyle {
        overrides,
        local_axioms,
    })
}

/// Render the active project style as a Markdown summary.
///
/// Used by the `mcp__illu__project_style` MCP tool. The output lets the
/// caller inspect why an axiom was filtered out or reranked, and review
/// the project conventions encoded in `.illu/style/project.json`.
///
/// # Errors
///
/// Currently infallible; the `Result` is preserved for future error
/// surfaces (e.g. async configuration reload) without a public API
/// break.
pub fn handle_project_style() -> Result<String, crate::IlluError> {
    let style = project_style();
    let mut output = String::new();

    if style.overrides.is_empty() && style.local_axioms.is_empty() {
        return Ok(
            "## Project style\n\nNo project style configured (`.illu/style/project.json` is absent or empty).\n"
                .to_string(),
        );
    }

    let _ = writeln!(output, "## Project style\n");

    if !style.overrides.is_empty() {
        let _ = writeln!(output, "### Axiom overrides ({})\n", style.overrides.len());
        // Iterate severities in a stable display order rather than the
        // HashMap's hash-randomised one; users scan top-down and expect
        // ignored→demoted→noted→elevated to read consistently.
        for severity in [
            Severity::Ignored,
            Severity::Demoted,
            Severity::Noted,
            Severity::Elevated,
        ] {
            let mut entries: Vec<(&String, &AxiomOverride)> = style
                .overrides
                .iter()
                .filter(|(_, o)| o.severity == severity)
                .collect();
            if entries.is_empty() {
                continue;
            }
            // Sort by id for deterministic output across HashMap layouts.
            entries.sort_by(|left, right| left.0.cmp(right.0));
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
            let _ = writeln!(
                output,
                "- `{}` — **{}**: {}",
                axiom.id,
                axiom.category,
                axiom.title_or_summary()
            );
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
            assert!(
                seen.insert(axiom.id.clone()),
                "duplicate local id `{}`",
                axiom.id
            );
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
        assert!(
            msg.contains("unsupported"),
            "expected unsupported-version error, got: {msg}"
        );
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
    fn test_project_style_rejects_override_id_in_local_axioms() {
        // Same id appearing in both arrays is a schema violation: the
        // project owns its local axioms directly, so overriding them is
        // meaningless.
        let json = r#"{
            "version": 1,
            "axiom_overrides": [
                { "id": "project_collision", "severity": "demoted" }
            ],
            "local_axioms": [
                {
                    "id": "project_collision",
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
        let msg = format!("{err}");
        assert!(
            msg.contains("both axiom_overrides and local_axioms"),
            "expected collision error, got: {msg}"
        );
    }

    #[test]
    fn test_project_style_adjust_score_filters_ignored() {
        let mut s = ProjectStyle::default();
        s.overrides.insert(
            "rust_quality_64_no_box_dyn_error_internal".into(),
            AxiomOverride {
                severity: Severity::Ignored,
                note: String::new(),
            },
        );
        assert_eq!(
            s.adjust_score("rust_quality_64_no_box_dyn_error_internal", 100),
            None
        );
    }

    #[test]
    fn test_project_style_adjust_score_demotes_and_elevates() {
        let mut s = ProjectStyle::default();
        s.overrides.insert(
            "a".into(),
            AxiomOverride {
                severity: Severity::Demoted,
                note: String::new(),
            },
        );
        s.overrides.insert(
            "b".into(),
            AxiomOverride {
                severity: Severity::Elevated,
                note: String::new(),
            },
        );
        assert_eq!(s.adjust_score("a", 100), Some(50));
        assert_eq!(s.adjust_score("b", 100), Some(200));
        // The score-zero invariant: no severity can conjure a hit out of
        // nothing. Demoted/Elevated of 0 must still be 0.
        assert_eq!(s.adjust_score("a", 0), Some(0));
        assert_eq!(s.adjust_score("b", 0), Some(0));
    }
}
