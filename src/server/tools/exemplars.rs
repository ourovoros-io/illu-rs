//! Curated, compile-checked Rust exemplars: real `.rs` files demonstrating
//! integrated idiomatic patterns. The manifest at
//! `assets/rust_exemplars/manifest.json` carries metadata; each exemplar's
//! code body is loaded via `include_str!` mapped from `slug` by
//! [`lookup_code`]. The mapping is hand-maintained — adding a manifest
//! entry without a matching `lookup_code` arm fails the
//! `test_every_exemplar_slug_has_code` test.

use serde::Deserialize;
use std::cmp::Reverse;
use std::fmt::Write;
use std::sync::OnceLock;
use thiserror::Error;

/// Failure categories for loading the embedded exemplar manifest.
///
/// All three variants represent build-time-shaped invariants: the
/// manifest JSON, the slug→source mapping, and the cache itself are
/// all baked into the binary, so reaching any of these errors at
/// runtime indicates an illu-rs build defect rather than a user
/// configuration issue. They are still surfaced as typed variants
/// (rather than `panic!`) so MCP callers see a structured error
/// instead of a server abort.
///
/// `#[non_exhaustive]` because future runtime checks (e.g. signature
/// verification on the manifest) can be added without breaking
/// downstream `match`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExemplarManifestError {
    /// Embedded `manifest.json` failed to parse. Should never trigger
    /// in a release build because the manifest is validated by tests.
    /// Wraps [`serde_json::Error`] in `source()` for the chain walk.
    #[error("failed to parse rust_exemplars/manifest.json")]
    Parse(#[from] serde_json::Error),

    /// Manifest references a slug that has no embedded source file.
    /// `lookup_code` is hand-maintained alongside the manifest;
    /// `test_every_exemplar_slug_has_code` enforces parity at test
    /// time, so this should never fire in production.
    #[error("exemplar slug `{0}` has no entry in lookup_code")]
    MissingSource(String),

    /// Cache `OnceLock::get` returned `None` immediately after `set`
    /// — only possible if a concurrent thread cleared it (which the
    /// type doesn't allow). Defensive variant kept to avoid
    /// `unwrap()` in the cache-fill path.
    #[error("exemplars OnceLock unexpectedly empty")]
    CacheUninit,
}

/// Top-level manifest shape: a single object whose `exemplars` array holds
/// each entry. Wrapping the array in an object reserves room for sibling
/// metadata (corpus version, generated-at) without breaking parsers.
#[derive(Debug, Deserialize)]
struct RawManifest {
    exemplars: Vec<RawExemplar>,
}

/// On-disk JSON shape for one exemplar entry. All fields are required;
/// missing fields are a manifest authoring bug, not a runtime case.
#[derive(Debug, Deserialize)]
struct RawExemplar {
    slug: String,
    category: String,
    title: String,
    description: String,
    triggers: Vec<String>,
    axioms_demonstrated: Vec<String>,
    source: String,
}

/// In-memory exemplar with pre-lowercased mirror fields used by the scorer
/// in [`handle_exemplars`]. The `code` field is `&'static str` from
/// [`lookup_code`]; it lives in the binary's read-only data section.
#[derive(Debug)]
#[non_exhaustive]
pub struct Exemplar {
    pub slug: String,
    pub category: String,
    pub title: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub axioms_demonstrated: Vec<String>,
    pub source: String,
    pub code: &'static str,
    triggers_lower: Vec<String>,
    category_lower: String,
    description_lower: String,
}

impl Exemplar {
    fn from_raw(raw: RawExemplar, code: &'static str) -> Self {
        let triggers_lower = raw.triggers.iter().map(|t| t.to_lowercase()).collect();
        let category_lower = raw.category.to_lowercase();
        let description_lower = raw.description.to_lowercase();
        Self {
            slug: raw.slug,
            category: raw.category,
            title: raw.title,
            description: raw.description,
            triggers: raw.triggers,
            axioms_demonstrated: raw.axioms_demonstrated,
            source: raw.source,
            code,
            triggers_lower,
            category_lower,
            description_lower,
        }
    }
}

const MANIFEST_JSON: &str = include_str!("../../../assets/rust_exemplars/manifest.json");
const MAX_EXEMPLAR_RESULTS: usize = 4;

/// Maps a manifest slug to its code body. `None` if the slug has no
/// associated file. The match must be kept in sync with the manifest;
/// `test_every_exemplar_slug_has_code` enforces this.
fn lookup_code(slug: &str) -> Option<&'static str> {
    match slug {
        "errors/api_error" => Some(include_str!(
            "../../../assets/rust_exemplars/errors/api_error.rs"
        )),
        "ownership/cow_string" => Some(include_str!(
            "../../../assets/rust_exemplars/ownership/cow_string.rs"
        )),
        "ownership/drop_guard" => Some(include_str!(
            "../../../assets/rust_exemplars/ownership/drop_guard.rs"
        )),
        "types/sealed_trait" => Some(include_str!(
            "../../../assets/rust_exemplars/types/sealed_trait.rs"
        )),
        "types/typestate_builder" => Some(include_str!(
            "../../../assets/rust_exemplars/types/typestate_builder.rs"
        )),
        "types/extension_trait" => Some(include_str!(
            "../../../assets/rust_exemplars/types/extension_trait.rs"
        )),
        "perf/closed_dispatch" => Some(include_str!(
            "../../../assets/rust_exemplars/perf/closed_dispatch.rs"
        )),
        "unsafe_ffi/maybe_uninit_init" => Some(include_str!(
            "../../../assets/rust_exemplars/unsafe_ffi/maybe_uninit_init.rs"
        )),
        "unsafe_ffi/c_string_wrapper" => Some(include_str!(
            "../../../assets/rust_exemplars/unsafe_ffi/c_string_wrapper.rs"
        )),
        _ => None,
    }
}

/// Returns the parsed exemplar corpus. Cached after first call. Parse
/// failure is surfaced to the caller; a later call retries until one
/// succeeds, mirroring [`crate::server::tools::axioms`].
///
/// # Errors
///
/// Returns [`crate::IlluError::ExemplarManifest`] wrapping an
/// [`ExemplarManifestError`] tagged with the precise failure category.
/// All three categories indicate a build-time invariant violation
/// (manifest authoring, `lookup_code` parity, or cache plumbing), so
/// they should never fire in a tested release build.
fn exemplars() -> Result<&'static [Exemplar], crate::IlluError> {
    static EXEMPLARS: OnceLock<Vec<Exemplar>> = OnceLock::new();

    if let Some(parsed) = EXEMPLARS.get() {
        return Ok(parsed.as_slice());
    }

    // `?` triggers `ExemplarManifestError: From<serde_json::Error>`
    // (via the `Parse` variant's `#[from]`), then
    // `IlluError: From<ExemplarManifestError>` to reach the
    // crate-wide error type — preserving the original parse error in
    // `source()`.
    let raw: RawManifest =
        serde_json::from_str(MANIFEST_JSON).map_err(ExemplarManifestError::from)?;

    let mut parsed = Vec::with_capacity(raw.exemplars.len());
    for entry in raw.exemplars {
        let code = lookup_code(&entry.slug)
            .ok_or_else(|| ExemplarManifestError::MissingSource(entry.slug.clone()))?;
        parsed.push(Exemplar::from_raw(entry, code));
    }

    // Lost-race set is fine — the winner's Vec is equivalent to ours.
    let _ = EXEMPLARS.set(parsed);
    EXEMPLARS
        .get()
        .map(Vec::as_slice)
        .ok_or(ExemplarManifestError::CacheUninit.into())
}

/// Score a single exemplar against a query. Mirrors
/// [`crate::server::tools::axioms`]'s scoring exactly: per-token
/// `.contains` accumulation in the loop, then full-query equality
/// boosts after the loop. The equality boost only fires when the user
/// types a category or trigger phrase verbatim (multi-word triggers
/// would never match a single token), so users get the same +20/+30
/// behaviour they get for axioms.
fn score(exemplar: &Exemplar, query_lower: &str, query_tokens: &[&str]) -> usize {
    let mut total = 0;
    for token in query_tokens {
        if exemplar.category_lower.contains(token) {
            total += 5;
        }
        for trigger in &exemplar.triggers_lower {
            if trigger.contains(token) {
                total += 10;
            }
        }
        if exemplar.description_lower.contains(token) {
            total += 2;
        }
    }
    if exemplar.category_lower == query_lower {
        total += 20;
    }
    for trigger in &exemplar.triggers_lower {
        if trigger == query_lower {
            total += 30;
        }
    }
    total
}

/// Returns up to [`MAX_EXEMPLAR_RESULTS`] exemplars best matching `query`,
/// formatted as Markdown with each code body in a Rust fenced block.
pub fn handle_exemplars(query: &str) -> Result<String, crate::IlluError> {
    let exemplars = exemplars()?;
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scored: Vec<(usize, &Exemplar)> = exemplars
        .iter()
        .map(|e| (score(e, &query_lower, &tokens), e))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by_key(|(s, _)| Reverse(*s));
    scored.truncate(MAX_EXEMPLAR_RESULTS);

    if scored.is_empty() {
        return Ok("No exemplars matched the query.".to_string());
    }

    let mut output = String::new();
    for (i, (match_score, exemplar)) in scored.iter().enumerate() {
        let _ = writeln!(output, "## {} — {}\n", exemplar.category, exemplar.title);
        let _ = writeln!(output, "**Slug:** `{}`  ", exemplar.slug);
        let _ = writeln!(output, "**Match score:** {match_score}  ");
        let _ = writeln!(output, "**Source:** {}  ", exemplar.source);
        if !exemplar.axioms_demonstrated.is_empty() {
            let _ = writeln!(
                output,
                "**Demonstrates axioms:** {}",
                exemplar.axioms_demonstrated.join(", ")
            );
        }
        let _ = writeln!(output, "\n{}\n", exemplar.description);
        let _ = writeln!(output, "```rust\n{}\n```\n", exemplar.code);
        if i + 1 < scored.len() {
            let _ = writeln!(output, "---\n");
        }
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_exemplar_manifest_parses() {
        let _ = exemplars().unwrap();
    }

    #[test]
    fn test_exemplar_manifest_error_variants_wire_through_illu_error() {
        // The three variants represent build-time invariants — they
        // are unreachable in a tested release build. Construct each
        // directly and verify the wrapping into [`IlluError`] preserves
        // the typed category so a future refactor cannot collapse the
        // chain into the legacy `IlluError::Other` path unnoticed.
        use std::error::Error as _;

        let parse_err: crate::IlluError = ExemplarManifestError::Parse(
            serde_json::from_str::<RawManifest>("not json").unwrap_err(),
        )
        .into();
        assert!(
            matches!(
                parse_err,
                crate::IlluError::ExemplarManifest(ExemplarManifestError::Parse(_))
            ),
            "Parse must wrap into IlluError::ExemplarManifest"
        );

        let missing: crate::IlluError =
            ExemplarManifestError::MissingSource("fake/slug".to_string()).into();
        assert!(
            matches!(
                &missing,
                crate::IlluError::ExemplarManifest(ExemplarManifestError::MissingSource(slug))
                    if slug == "fake/slug"
            ),
            "expected ExemplarManifest(MissingSource(`fake/slug`)), got: {missing:?}"
        );

        let cache: crate::IlluError = ExemplarManifestError::CacheUninit.into();
        assert!(
            matches!(
                cache,
                crate::IlluError::ExemplarManifest(ExemplarManifestError::CacheUninit)
            ),
            "CacheUninit must wrap into IlluError::ExemplarManifest"
        );

        // Source-chain check: `Parse` must expose the underlying
        // serde_json::Error via `source()`. Without this the
        // [Error Source Chain] axiom would silently regress.
        let parse_inner = ExemplarManifestError::Parse(
            serde_json::from_str::<RawManifest>("not json").unwrap_err(),
        );
        assert!(
            parse_inner.source().is_some(),
            "Parse must expose the wrapped serde_json::Error via source()"
        );
    }

    #[test]
    fn test_every_exemplar_slug_has_code() {
        for exemplar in exemplars().unwrap() {
            assert!(
                !exemplar.code.is_empty(),
                "exemplar slug `{}` has empty code body",
                exemplar.slug
            );
        }
    }

    #[test]
    fn test_no_duplicate_slugs() {
        let mut seen = std::collections::HashSet::new();
        for exemplar in exemplars().unwrap() {
            assert!(
                seen.insert(exemplar.slug.clone()),
                "duplicate slug `{}`",
                exemplar.slug
            );
        }
    }

    #[test]
    fn test_every_axiom_reference_resolves() {
        let axioms = crate::server::tools::axioms::axioms_for_test();
        let axiom_ids: std::collections::HashSet<&str> =
            axioms.iter().map(|a| a.id.as_str()).collect();
        for exemplar in exemplars().unwrap() {
            for axiom_id in &exemplar.axioms_demonstrated {
                assert!(
                    axiom_ids.contains(axiom_id.as_str()),
                    "exemplar `{}` references unknown axiom `{}`",
                    exemplar.slug,
                    axiom_id
                );
            }
        }
    }

    #[test]
    fn test_exemplars_batch_1_present() {
        let result = handle_exemplars(
            "api error hierarchy thiserror enum error source chain stable error variants",
        )
        .unwrap();
        assert!(
            result.contains("Error Design"),
            "Error Design exemplar missing in focused query"
        );

        let result = handle_exemplars(
            "Cow str example borrow when possible string normalize Cow lazy allocation",
        )
        .unwrap();
        assert!(
            result.contains("Cow Strings"),
            "Cow Strings exemplar missing in focused query"
        );

        let result = handle_exemplars(
            "drop guard example RAII cleanup scope guard pattern panic safe cleanup",
        )
        .unwrap();
        assert!(
            result.contains("RAII Drop Guard"),
            "RAII Drop Guard exemplar missing in focused query"
        );
    }

    #[test]
    fn test_exemplars_batch_2_present() {
        let result = handle_exemplars(
            "sealed trait example private supertrait closed implementation extension safe",
        )
        .unwrap();
        assert!(
            result.contains("Sealed Trait"),
            "Sealed Trait exemplar missing in focused query"
        );

        let result =
            handle_exemplars("type state builder compile time required field ZST marker builder")
                .unwrap();
        assert!(
            result.contains("Type-state Builder"),
            "Type-state Builder exemplar missing in focused query"
        );

        let result = handle_exemplars(
            "extension trait example extending foreign type sealed extension trait",
        )
        .unwrap();
        assert!(
            result.contains("Extension Trait"),
            "Extension Trait exemplar missing in focused query"
        );
    }

    #[test]
    fn test_exemplars_batch_3_present() {
        let result = handle_exemplars(
            "enum dispatch example match closed set command pattern enum no dyn dispatch",
        )
        .unwrap();
        assert!(
            result.contains("Enum Dispatch"),
            "Enum Dispatch exemplar missing in focused query"
        );

        let result = handle_exemplars(
            "MaybeUninit example raw mut init pattern field by field init incremental struct init",
        )
        .unwrap();
        assert!(
            result.contains("MaybeUninit Init"),
            "MaybeUninit Init exemplar missing in focused query"
        );

        let result = handle_exemplars(
            "FFI string wrapper extern C string ownership CStr CString example FFI safe panic",
        )
        .unwrap();
        assert!(
            result.contains("FFI Strings Example"),
            "FFI Strings Example exemplar missing in focused query"
        );
    }

    #[test]
    fn test_exemplar_demo_query_returns_new_exemplars() {
        let result = handle_exemplars(
            "idiomatic Rust integrated patterns error Cow drop guard sealed trait builder enum dispatch MaybeUninit FFI",
        )
        .unwrap();
        let expected_categories = [
            "Error Design",
            "Cow Strings",
            "RAII Drop Guard",
            "Sealed Trait",
            "Type-state Builder",
            "Extension Trait",
            "Enum Dispatch",
            "MaybeUninit Init",
            "FFI Strings Example",
        ];
        let surfaced = expected_categories
            .iter()
            .filter(|cat| result.contains(*cat))
            .count();
        assert!(
            surfaced >= 3,
            "expected at least 3 exemplar categories in demo query, got {surfaced}"
        );
    }

    /// Compile-check that every exemplar file is real Rust. Each is a
    /// separate child module so unrelated identifiers don't collide. Dead
    /// code is allowed because exemplars are demonstrations, not callable
    /// from the rest of the crate. Future exemplars that exercise
    /// `unused_imports`/`unused_variables`/`unused_mut` patterns may need
    /// to broaden the `expect` set below.
    #[expect(
        dead_code,
        reason = "exemplars are demonstrations, not callable from the rest of the crate"
    )]
    mod compile_check {
        mod errors_api_error {
            include!("../../../assets/rust_exemplars/errors/api_error.rs");
        }
        mod ownership_cow_string {
            include!("../../../assets/rust_exemplars/ownership/cow_string.rs");
        }
        mod ownership_drop_guard {
            include!("../../../assets/rust_exemplars/ownership/drop_guard.rs");
        }
        mod types_sealed_trait {
            include!("../../../assets/rust_exemplars/types/sealed_trait.rs");
        }
        mod types_typestate_builder {
            include!("../../../assets/rust_exemplars/types/typestate_builder.rs");
        }
        mod types_extension_trait {
            include!("../../../assets/rust_exemplars/types/extension_trait.rs");
        }
        mod perf_closed_dispatch {
            include!("../../../assets/rust_exemplars/perf/closed_dispatch.rs");
        }
        mod unsafe_ffi_maybe_uninit_init {
            include!("../../../assets/rust_exemplars/unsafe_ffi/maybe_uninit_init.rs");
        }
        mod unsafe_ffi_c_string_wrapper {
            include!("../../../assets/rust_exemplars/unsafe_ffi/c_string_wrapper.rs");
        }
    }
}
