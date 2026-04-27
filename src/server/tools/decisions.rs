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
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use thiserror::Error;

/// Failure categories for loading and parsing decision records.
///
/// Distinct variants let callers (and tests) decide how to react without
/// parsing `Display` strings. Wrapped as
/// [`crate::IlluError::Decision`] when bubbled across the module
/// boundary; inside this module, [`parse_decision`] returns this type
/// directly so [`Error Path Specificity`] tests can match the precise
/// category.
///
/// `#[non_exhaustive]` so future categories (e.g. a richer status
/// validation error) can be added without breaking downstream `match`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DecisionError {
    /// JSON-level parse failure. The wrapped [`serde_json::Error`] is
    /// returned via [`std::error::Error::source`] so the caller can walk
    /// the chain. `Display` describes only this layer.
    #[error("failed to parse decision JSON")]
    Json(#[from] serde_json::Error),

    /// `id` field present but missing the mandatory `decision_` prefix.
    /// The prefix keeps decision ids in their own namespace and prevents
    /// accidental shadowing with axiom ids.
    #[error("decision id `{0}` must start with `decision_`")]
    UnprefixedId(String),

    /// `date` field is not a `YYYY-MM-DD` string. The validator is
    /// regex-light (no calendar correctness), so any 10-character
    /// `\d{{4}}-\d{{2}}-\d{{2}}` is accepted; anything else is rejected.
    #[error("decision `{id}` has malformed date `{date}`; expected YYYY-MM-DD")]
    MalformedDate {
        /// Decision id whose date failed validation; lets callers point
        /// to the offending file without parsing the message.
        id: String,
        /// The literal date string that failed validation.
        date: String,
    },

    /// Filesystem read of the decisions directory failed. Carries the
    /// path as a typed field. Per-file read failures inside the
    /// directory walk are logged at `warn` and skipped (so one bad file
    /// does not disable the whole directory) rather than surfacing
    /// here — this variant is reserved for the directory-level failure.
    #[error("failed to read {path}", path = path.display())]
    Read {
        /// Path the loader attempted to read (the decisions directory).
        path: PathBuf,
        /// Underlying I/O error returned by `std::fs::read_dir`.
        #[source]
        source: std::io::Error,
    },
}

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

impl Status {
    /// Stable lowercase string representation matching the on-disk JSON
    /// values. Used by the scorer (full-query equality boost) and the
    /// formatter (display in result headings) — extracted to keep both
    /// sites in sync as new variants are added.
    fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Superseded => "superseded",
            Self::Deprecated => "deprecated",
        }
    }
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
#[expect(
    clippy::struct_field_names,
    reason = "ADR vocabulary: the `decision` field stores the chosen-path text and is named the same as the struct on purpose. Renaming would lose the schema mapping."
)]
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
            .map(|a| {
                format!(
                    "{} {}",
                    a.option.to_lowercase(),
                    a.why_rejected.to_lowercase()
                )
            })
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
/// not called or the directory was absent. The empty branch is just `&[]`
/// because `&'static [T]` already exists statically — no `OnceLock` cell
/// needed for the fallback (unlike `project_style::project_style`, which
/// must return `&'static ProjectStyle` and therefore needs storage).
pub fn decisions() -> &'static [Decision] {
    DECISIONS.get().map_or(&[], Vec::as_slice)
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
///
/// # Errors
///
/// Returns [`crate::IlluError::Decision`] wrapping
/// [`DecisionError::Read`] when the directory itself cannot be read
/// (typically: missing, permission denied, or a non-directory at that
/// path). Per-file failures inside the walk are not propagated — they
/// are logged at `warn` and the file is skipped.
pub fn load_from_dir(dir: &Path) -> Result<Vec<Decision>, crate::IlluError> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|source| DecisionError::Read {
            path: dir.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
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

/// Parse and validate a single decision record.
///
/// Returns the module-local [`DecisionError`] rather than
/// [`crate::IlluError`] so [`Error Path Specificity`] tests can match
/// the precise category. The directory walker [`load_from_dir`]
/// handles the conversion at the module boundary.
///
/// # Errors
///
/// Returns [`DecisionError::Json`] on malformed JSON,
/// [`DecisionError::UnprefixedId`] when the id is missing the
/// `decision_` prefix, or [`DecisionError::MalformedDate`] when the
/// `date` field is not a `YYYY-MM-DD` string.
fn parse_decision(json: &str) -> Result<Decision, DecisionError> {
    // `?` triggers `DecisionError: From<serde_json::Error>` via the
    // `Json` variant's `#[from]`, preserving the original parse error
    // in `source()` rather than stringifying it.
    let raw: RawDecision = serde_json::from_str(json)?;
    if !raw.id.starts_with("decision_") {
        return Err(DecisionError::UnprefixedId(raw.id));
    }
    if !is_iso_date(&raw.date) {
        return Err(DecisionError::MalformedDate {
            id: raw.id,
            date: raw.date,
        });
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
    bytes.iter().enumerate().all(|(i, &b)| match i {
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
    if decision.status.as_str() == query_lower {
        total = total.saturating_add(20);
    }
    total
}

/// Returns up to [`MAX_DECISION_RESULTS`] decisions best matching `query`,
/// reading from the process-global cache populated by [`init`]. The
/// `Result` wrapper is for parity with sibling `handle_*` tool entry
/// points; the underlying [`handle_decisions_with_corpus`] is infallible.
pub fn handle_decisions(query: &str) -> Result<String, crate::IlluError> {
    Ok(handle_decisions_with_corpus(query, decisions()))
}

/// Same as [`handle_decisions`] but takes the corpus as an argument so
/// tests can exercise the formatter and scorer without touching the
/// process-global `DECISIONS` cache. Mirrors
/// [`crate::server::tools::axioms::handle_axioms_with_style`]. Returns
/// `String` (not `Result`) because no operation here can fail — parsing
/// already happened at [`init`] time.
pub(crate) fn handle_decisions_with_corpus(query: &str, corpus: &[Decision]) -> String {
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    if corpus.is_empty() {
        return "## Decisions\n\nNo decision records are configured (`.illu/style/decisions/` is absent or empty).\n"
            .to_string();
    }

    let mut scored: Vec<(usize, &Decision)> = corpus
        .iter()
        .map(|d| (score_decision(d, &query_lower, &tokens), d))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by_key(|(s, _)| Reverse(*s));
    scored.truncate(MAX_DECISION_RESULTS);

    if scored.is_empty() {
        return "## Decisions\n\nNo decisions matched the query.\n".to_string();
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Decisions matching '{query}'\n");
    for (i, (score, d)) in scored.iter().enumerate() {
        let _ = writeln!(
            output,
            "## {} (`{}`, {})",
            d.title,
            d.status.as_str(),
            d.date
        );
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
            let _ = writeln!(output, "**Related files:** {}", d.related_files.join(", "));
        }
        if i + 1 < scored.len() {
            let _ = writeln!(output, "\n---\n");
        }
    }

    output
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
        // serde rejects an unknown enum tag at JSON parse time, so the
        // failure category is `Json(_)`, not a hypothetical
        // `UnknownStatus` variant. [Error Path Specificity] catches a
        // future regression that mistakenly treats this as a domain
        // validation error rather than a parse error.
        let bad = r#"{
            "id": "decision_bad_status",
            "title": "X",
            "status": "not-a-status",
            "date": "2026-01-01",
            "context": "x",
            "decision": "x",
            "consequences": "x"
        }"#;
        let err = parse_decision(bad).unwrap_err();
        assert!(
            matches!(err, DecisionError::Json(_)),
            "expected Json(_), got: {err:?}"
        );
    }

    #[test]
    fn test_decisions_rejects_unprefixed_id() {
        let bad = r#"{
            "id": "not_prefixed",
            "title": "X",
            "status": "accepted",
            "date": "2026-04-27",
            "context": "x",
            "decision": "x",
            "consequences": "x"
        }"#;
        let err = parse_decision(bad).unwrap_err();
        assert!(
            matches!(&err, DecisionError::UnprefixedId(id) if id == "not_prefixed"),
            "expected UnprefixedId(`not_prefixed`), got: {err:?}"
        );
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
        assert!(
            matches!(
                &err,
                DecisionError::MalformedDate { id, date }
                    if id == "decision_bad_date" && date == "April 2026"
            ),
            "expected MalformedDate {{ decision_bad_date, April 2026 }}, got: {err:?}"
        );

        // Accept well-formed dates (no calendar correctness check).
        assert!(is_iso_date("2026-04-15"));
        assert!(is_iso_date("2026-02-30")); // not a real date but the format is right
        assert!(!is_iso_date("2026-4-15"));
        assert!(!is_iso_date("April 2026"));
    }

    #[test]
    fn test_decisions_load_from_dir_missing_is_typed() {
        // Directory-level read failure crosses into IlluError; the
        // wrapped category must remain reachable so callers can react
        // to a missing decisions directory distinctly from a parse
        // failure inside a present directory.
        let missing = PathBuf::from("/definitely/does/not/exist/decisions");
        let err = load_from_dir(&missing).unwrap_err();
        assert!(
            matches!(err, crate::IlluError::Decision(DecisionError::Read { .. })),
            "expected IlluError::Decision(Read), got: {err:?}"
        );
    }

    #[test]
    fn test_handle_decisions_focused_query() {
        // Tests pass the fixture corpus directly via handle_decisions_with_corpus
        // rather than touching the process-global DECISIONS cache. This mirrors
        // the Phase 3 testing seam (handle_axioms_with_style) and avoids the
        // cross-test coupling that any OnceLock-write would introduce.
        let corpus = fixture_corpus();
        let result = handle_decisions_with_corpus("enum dispatch handlers vtable Box dyn", &corpus);
        assert!(
            result.contains("decision_use_enum_dispatch_for_handlers"),
            "enum-dispatch decision must surface; got: {result}"
        );
    }

    #[test]
    fn test_handle_decisions_demo_query() {
        let corpus = fixture_corpus();
        let result = handle_decisions_with_corpus(
            "dispatch mutex async runtime alternatives chosen rejected",
            &corpus,
        );
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
