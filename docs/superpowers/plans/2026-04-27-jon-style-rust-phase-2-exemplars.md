# Jon-Style Rust Phase 2 (Exemplars Infrastructure) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Add a new `mcp__illu__exemplars` MCP tool plus 9 compile-checked Rust exemplars across 3-3-3 batches. Establish the asset directory, manifest schema, server module, MCP registration, public-API re-export, compile-check module, and validity tests in Task 1; append exemplars in Tasks 2 and 3.

**Architecture:** New module `src/server/tools/exemplars.rs` mirrors `axioms.rs`. Manifest at `assets/rust_exemplars/manifest.json`; each exemplar lives at `assets/rust_exemplars/<slug>.rs`. Code is loaded via `include_str!` mapped from slug → &'static str by a hand-maintained `lookup_code` match, so the compiler enforces that every manifest entry has a real file (the match must be updated alongside the manifest). The `handle_exemplars` tool scores queries against triggers/category/description and returns the top matches with their code body.

**Tech Stack:** Rust 2024, `serde_json`, `rmcp` macros, `cargo test` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo fmt --all -- --check`.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-2-exemplars-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-2-exemplars-design.md)

**Existing state:** 102 axioms. `src/server/tools/axioms.rs` is the canonical mirror for the new module shape (verified: `RawAxiom` → `Axiom` two-struct pattern, `OnceLock` parse-once cache, `handle_axioms` scoring, `MAX_AXIOM_RESULTS`, test module with `#[expect(clippy::unwrap_used, reason = "tests")]`). `src/server/mod.rs` registers tools via `#[tool(name = "...", description = "...")]` with a `Parameters<XxxParams>` argument; the `axioms` registration at `:934-942` is the template. `src/api.rs` re-exports each tool's `handle_*` fn under a module.

**Lint constraints:** `unwrap_used = "deny"`, `expect_used = "warn"`, `allow_attributes = "deny"` (use `#[expect(lint, reason = "...")]`). Test module uses `#[expect(clippy::unwrap_used, reason = "tests")]`. Production code in `exemplars.rs` must use `Result` propagation, never `.unwrap()` or `.expect()`.

**Drafts pre-approved by user.** Skip the user-review step inside batch tasks.

**Verification discipline.** Each exemplar must (a) compile clean under the workspace's clippy denylist, (b) actually demonstrate the axioms listed in `axioms_demonstrated`, (c) be production-useful (not pedagogical fluff). The per-batch reviewer checks all three.

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `src/server/tools/exemplars.rs` | Create | Module: types, parse-once cache, `handle_exemplars`, `lookup_code`, all tests |
| `src/server/tools/mod.rs` | Modify | Add `pub mod exemplars;` |
| `src/server/mod.rs` | Modify | Register `exemplars` MCP tool |
| `src/api.rs` | Modify | Re-export `exemplars::handle_exemplars` |
| `assets/rust_exemplars/manifest.json` | Create | Manifest with metadata for all 9 exemplars |
| `assets/rust_exemplars/errors/api_error.rs` | Create | Batch 1 exemplar |
| `assets/rust_exemplars/ownership/cow_string.rs` | Create | Batch 1 exemplar |
| `assets/rust_exemplars/ownership/drop_guard.rs` | Create | Batch 1 exemplar |
| `assets/rust_exemplars/types/sealed_trait.rs` | Create | Batch 2 exemplar |
| `assets/rust_exemplars/types/typestate_builder.rs` | Create | Batch 2 exemplar |
| `assets/rust_exemplars/types/extension_trait.rs` | Create | Batch 2 exemplar |
| `assets/rust_exemplars/perf/closed_dispatch.rs` | Create | Batch 3 exemplar |
| `assets/rust_exemplars/unsafe_ffi/maybe_uninit_init.rs` | Create | Batch 3 exemplar |
| `assets/rust_exemplars/unsafe_ffi/c_string_wrapper.rs` | Create | Batch 3 exemplar |

---

## Task 1: Infrastructure setup + Batch 1 (errors/ownership exemplars)

**Files (create):**
- `assets/rust_exemplars/manifest.json`
- `assets/rust_exemplars/errors/api_error.rs`
- `assets/rust_exemplars/ownership/cow_string.rs`
- `assets/rust_exemplars/ownership/drop_guard.rs`
- `src/server/tools/exemplars.rs`

**Files (modify):**
- `src/server/tools/mod.rs` (add `pub mod exemplars;` alphabetically next to `pub mod axioms;`)
- `src/server/mod.rs` (add `ExemplarsParams` struct and `#[tool] async fn exemplars` method, mirroring `axioms`)
- `src/api.rs` (add `pub mod exemplars { pub use crate::server::tools::exemplars::handle_exemplars; }` re-export)

- [ ] **Step 1: Create the asset directory tree and the three Batch 1 exemplar files**

Create file `assets/rust_exemplars/errors/api_error.rs`:

```rust
//! Layered API error: a public `ApiError` enum (variants are a stable contract)
//! with helpers for constructing internal/external variants and a `source`
//! chain preserved via `std::error::Error::source`. The pattern that
//! `thiserror` automates, spelled out so the underlying machinery is visible.

use std::error::Error as StdError;
use std::fmt;

/// Stable, externally-visible error variants. Adding a variant is a breaking
/// API change; callers pattern-match on this exhaustively.
#[derive(Debug)]
pub enum ApiError {
    /// User input failed validation.
    Validation { field: &'static str, detail: String },
    /// Backing store unreachable or returned an unexpected response. The
    /// `source` carries the underlying cause for log/trace correlation.
    Storage {
        kind: StorageKind,
        source: Box<dyn StdError + Send + Sync + 'static>,
    },
    /// Auth check failed.
    Unauthorized,
}

#[derive(Debug, Clone, Copy)]
pub enum StorageKind {
    Timeout,
    NotFound,
    Conflict,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation { field, detail } => {
                write!(f, "validation failed for `{field}`: {detail}")
            }
            Self::Storage { kind, .. } => write!(f, "storage error: {kind:?}"),
            Self::Unauthorized => f.write_str("unauthorized"),
        }
    }
}

impl StdError for ApiError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Storage { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl ApiError {
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        Self::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// Wraps any underlying error as a Storage variant of the given kind.
    pub fn storage<E: StdError + Send + Sync + 'static>(kind: StorageKind, source: E) -> Self {
        Self::Storage {
            kind,
            source: Box::new(source),
        }
    }
}
```

Create file `assets/rust_exemplars/ownership/cow_string.rs`:

```rust
//! `Cow<'a, str>` for sometimes-borrowed-sometimes-owned config: returns a
//! borrow when the input is already in the desired form, only allocating
//! when normalization is required. Avoids the
//! "always-clone-because-the-API-takes-String" anti-pattern.

use std::borrow::Cow;

/// Normalize a config-key string: trim whitespace and lowercase ASCII letters.
/// Returns the original slice (no allocation) when no normalization is needed.
pub fn normalize_key(input: &str) -> Cow<'_, str> {
    let trimmed = input.trim();
    let needs_lower = trimmed.bytes().any(|b| b.is_ascii_uppercase());

    if !needs_lower && trimmed.len() == input.len() {
        // Hot path: already normalized; return the input slice unchanged.
        Cow::Borrowed(input)
    } else if !needs_lower {
        // Trim-only: still a borrow, just from a shorter slice of `input`.
        Cow::Borrowed(trimmed)
    } else {
        // Slow path: must allocate the lowercased form.
        Cow::Owned(trimmed.to_ascii_lowercase())
    }
}
```

Create file `assets/rust_exemplars/ownership/drop_guard.rs`:

```rust
//! RAII drop-guard that runs an arbitrary closure when the guard is dropped,
//! including via panic unwinding. Useful for paired setup/teardown where
//! early returns or panics must still trigger cleanup. `Guard::dismiss`
//! consumes the guard without firing the closure on the success path.

/// Runs `f` exactly once, when the guard is dropped.
pub struct Guard<F: FnOnce()> {
    cleanup: Option<F>,
}

impl<F: FnOnce()> Guard<F> {
    pub fn new(cleanup: F) -> Self {
        Self {
            cleanup: Some(cleanup),
        }
    }

    /// Consumes the guard without running the cleanup closure. Use on the
    /// success path when the cleanup is no longer required.
    pub fn dismiss(mut self) {
        self.cleanup.take();
    }
}

impl<F: FnOnce()> Drop for Guard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.cleanup.take() {
            f();
        }
    }
}
```

- [ ] **Step 2: Create the manifest with all 9 entries**

Create file `assets/rust_exemplars/manifest.json` (all 9 entries upfront so the schema is locked; only Batch 1 has files at this point — Tasks 2 and 3 add files but no manifest changes):

```json
{
  "exemplars": [
    {
      "slug": "errors/api_error",
      "category": "Error Design",
      "title": "Layered API error hierarchy",
      "description": "Public ApiError enum with stable variants, internal source chain via std::error::Error::source, helper constructors. The pattern thiserror's #[derive(Error)] generates, spelled out for clarity.",
      "triggers": ["api error hierarchy", "error type design", "thiserror enum", "error source chain", "stable error variants"],
      "axioms_demonstrated": ["rust_quality_57_error_strategy", "rust_quality_58_error_design"],
      "source": "thiserror documentation; Rust for Rustaceans, ch. 4 §Error Handling"
    },
    {
      "slug": "ownership/cow_string",
      "category": "Cow Strings",
      "title": "Cow<str> for sometimes-borrowed config",
      "description": "Returns the input slice unchanged when no normalization is needed, only allocating on the slow path. Demonstrates Cow's role in avoiding the always-clone-because-API-takes-String anti-pattern.",
      "triggers": ["Cow str example", "borrow when possible", "string normalize Cow", "lazy allocation Cow"],
      "axioms_demonstrated": ["rust_quality_86_string_allocation"],
      "source": "Rust for Rustaceans, ch. 1 §Cow"
    },
    {
      "slug": "ownership/drop_guard",
      "category": "RAII Drop Guard",
      "title": "Drop-guard for paired setup/teardown",
      "description": "RAII helper that runs an arbitrary closure on drop. Carries cleanup through panic unwinding and early returns; .dismiss() consumes the guard without firing the closure on the success path.",
      "triggers": ["drop guard example", "RAII cleanup", "scope guard pattern", "panic safe cleanup"],
      "axioms_demonstrated": ["rust_quality_71_drop_order_matters"],
      "source": "Rust for Rustaceans, ch. 1 §Drop; scopeguard crate"
    },
    {
      "slug": "types/sealed_trait",
      "category": "Sealed Trait",
      "title": "Sealed trait with private supertrait",
      "description": "Public trait that can be called externally but only implemented within this crate. The seal is enforced by a private Sealed supertrait that external crates cannot name and therefore cannot satisfy.",
      "triggers": ["sealed trait example", "private supertrait", "closed implementation", "extension safe"],
      "axioms_demonstrated": ["rust_quality_76_sealed_traits"],
      "source": "Rust for Rustaceans, ch. 2 §Trait Design"
    },
    {
      "slug": "types/typestate_builder",
      "category": "Type-state Builder",
      "title": "Type-state builder with required-field tracking",
      "description": "Builder where required fields are encoded in the generic type parameters via marker types. Calling .build() before every required field is set is a compile error, not a runtime panic — no Option, no unwrap.",
      "triggers": ["type state builder", "compile time required field", "ZST marker builder", "phantom builder state"],
      "axioms_demonstrated": ["rust_quality_83_zst_markers"],
      "source": "Rust for Rustaceans, ch. 3 §Type-state; typed-builder crate"
    },
    {
      "slug": "types/extension_trait",
      "category": "Extension Trait",
      "title": "Sealed extension trait on a foreign type",
      "description": "Adds methods to a foreign type (str) via a trait. Sealed so external crates cannot add competing impls; the seal also prevents accidental shadowing of methods we add later.",
      "triggers": ["extension trait example", "extending foreign type", "sealed extension trait", "method on str"],
      "axioms_demonstrated": ["rust_quality_76_sealed_traits"],
      "source": "Rust for Rustaceans, ch. 2 §Extension Traits"
    },
    {
      "slug": "perf/closed_dispatch",
      "category": "Enum Dispatch",
      "title": "Closed-set command dispatch via enum + match",
      "description": "Counter that processes a stream of commands via an enum + match rather than Box<dyn Command>. No vtable, no indirect call; the match compiles to direct branches; Vec<Command> packs each element rather than indirecting through a fat pointer.",
      "triggers": ["enum dispatch example", "match closed set", "command pattern enum", "no dyn dispatch"],
      "axioms_demonstrated": ["rust_quality_90_enum_dispatch"],
      "source": "Rust for Rustaceans, ch. 2 §Trait Objects; enum_dispatch crate"
    },
    {
      "slug": "unsafe_ffi/maybe_uninit_init",
      "category": "MaybeUninit Init",
      "title": "Incremental MaybeUninit initialization with &raw mut",
      "description": "Builds a fixed-size buffer field-by-field through &raw mut, never materializing a &mut to the partially-initialized value. Each unsafe block is scoped to the exact unsafe operation; each has a SAFETY comment naming the invariants.",
      "triggers": ["MaybeUninit example", "raw mut init pattern", "field by field init", "incremental struct init"],
      "axioms_demonstrated": ["rust_quality_97_maybe_uninit", "rust_quality_94_unsafe_block_discipline", "rust_quality_96_unsafe_block_scope"],
      "source": "std::mem::MaybeUninit rustdoc; Rustonomicon §Working with Uninitialized Memory"
    },
    {
      "slug": "unsafe_ffi/c_string_wrapper",
      "category": "FFI Strings Example",
      "title": "FFI string borrow-and-transfer pair",
      "description": "Three extern \"C\" functions: reading the length of a caller-owned NUL-terminated C string (CStr::from_ptr borrow), building a Rust-owned C string transferred to the caller (CString::into_raw), and reclaiming such a pointer for free (CString::from_raw). Each fn body is panic-isolated via catch_unwind.",
      "triggers": ["FFI string wrapper", "extern C string ownership", "CStr CString example", "FFI safe panic"],
      "axioms_demonstrated": ["rust_quality_100_ffi_boundary", "rust_quality_102_ffi_strings", "rust_quality_94_unsafe_block_discipline", "rust_quality_95_unsafe_fn_contract"],
      "source": "std::ffi::CStr / CString rustdoc; Rust Reference §Foreign Function Interface"
    }
  ]
}
```

- [ ] **Step 3: Create `src/server/tools/exemplars.rs`**

This is the bulk of Task 1. Mirror `axioms.rs`'s shape (`RawExemplar`/`Exemplar` two-struct pattern, `OnceLock` parse-once cache, scoring `handle_exemplars`) and add the slug-to-code `lookup_code` match. The `lookup_code` match must include all 9 slugs upfront — but the files for Batches 2 and 3 don't exist yet. **Solution:** the match arms for Batches 2 and 3 are added as the files are created (Tasks 2 and 3). For Task 1, only the 3 Batch-1 arms are present; the other 6 manifest entries fail the `test_every_exemplar_slug_has_code` test until their files land. **Therefore Task 1 must defer the 6 Batches-2/3 entries from the manifest** — easier to add manifest entries per-batch alongside the files. Revising the approach:

**Revised plan: each batch task adds its 3 manifest entries together with its 3 files and 3 lookup_code arms.** Task 1 ships the manifest with only the 3 Batch-1 entries; Tasks 2 and 3 append entries.

Replace the manifest content above with **only the 3 Batch-1 entries** for Task 1. The full 9-entry manifest is reached by appending in Tasks 2 and 3.

Manifest content for Task 1:

```json
{
  "exemplars": [
    {
      "slug": "errors/api_error",
      "category": "Error Design",
      "title": "Layered API error hierarchy",
      "description": "Public ApiError enum with stable variants, internal source chain via std::error::Error::source, helper constructors. The pattern thiserror's #[derive(Error)] generates, spelled out for clarity.",
      "triggers": ["api error hierarchy", "error type design", "thiserror enum", "error source chain", "stable error variants"],
      "axioms_demonstrated": ["rust_quality_57_error_strategy", "rust_quality_58_error_design"],
      "source": "thiserror documentation; Rust for Rustaceans, ch. 4 §Error Handling"
    },
    {
      "slug": "ownership/cow_string",
      "category": "Cow Strings",
      "title": "Cow<str> for sometimes-borrowed config",
      "description": "Returns the input slice unchanged when no normalization is needed, only allocating on the slow path. Demonstrates Cow's role in avoiding the always-clone-because-API-takes-String anti-pattern.",
      "triggers": ["Cow str example", "borrow when possible", "string normalize Cow", "lazy allocation Cow"],
      "axioms_demonstrated": ["rust_quality_86_string_allocation"],
      "source": "Rust for Rustaceans, ch. 1 §Cow"
    },
    {
      "slug": "ownership/drop_guard",
      "category": "RAII Drop Guard",
      "title": "Drop-guard for paired setup/teardown",
      "description": "RAII helper that runs an arbitrary closure on drop. Carries cleanup through panic unwinding and early returns; .dismiss() consumes the guard without firing the closure on the success path.",
      "triggers": ["drop guard example", "RAII cleanup", "scope guard pattern", "panic safe cleanup"],
      "axioms_demonstrated": ["rust_quality_71_drop_order_matters"],
      "source": "Rust for Rustaceans, ch. 1 §Drop; scopeguard crate"
    }
  ]
}
```

Create file `src/server/tools/exemplars.rs`:

```rust
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

#[derive(Debug, Deserialize)]
struct RawManifest {
    exemplars: Vec<RawExemplar>,
}

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
        let triggers_lower = raw
            .triggers
            .iter()
            .map(|t| t.to_lowercase())
            .collect();
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
        _ => None,
    }
}

/// Returns the parsed exemplar corpus. Cached after first call.
fn exemplars() -> Result<&'static [Exemplar], crate::IlluError> {
    static EXEMPLARS: OnceLock<Vec<Exemplar>> = OnceLock::new();

    if let Some(parsed) = EXEMPLARS.get() {
        return Ok(parsed.as_slice());
    }

    let raw: RawManifest = serde_json::from_str(MANIFEST_JSON).map_err(|e| {
        crate::IlluError::Other(format!("failed to parse rust_exemplars/manifest.json: {e}"))
    })?;

    let mut parsed = Vec::with_capacity(raw.exemplars.len());
    for entry in raw.exemplars {
        let code = lookup_code(&entry.slug).ok_or_else(|| {
            crate::IlluError::Other(format!(
                "exemplar slug `{}` has no entry in lookup_code",
                entry.slug
            ))
        })?;
        parsed.push(Exemplar::from_raw(entry, code));
    }

    let _ = EXEMPLARS.set(parsed);
    EXEMPLARS
        .get()
        .map(Vec::as_slice)
        .ok_or_else(|| crate::IlluError::Other("exemplars OnceLock unexpectedly empty".into()))
}

/// Score a single exemplar against a tokenized query. Mirrors
/// `axioms::handle_axioms`'s scoring weights.
fn score(exemplar: &Exemplar, query_tokens: &[&str]) -> usize {
    let mut score = 0;
    for token in query_tokens {
        for trigger in &exemplar.triggers_lower {
            if trigger == token {
                score += 30;
            } else if trigger.contains(token) {
                score += 10;
            }
        }
        if exemplar.category_lower == *token {
            score += 20;
        } else if exemplar.category_lower.contains(token) {
            score += 5;
        }
        if exemplar.description_lower.contains(token) {
            score += 2;
        }
    }
    score
}

/// Returns up to `MAX_EXEMPLAR_RESULTS` exemplars best matching `query`,
/// formatted as Markdown with the code body in a Rust fenced block.
pub fn handle_exemplars(query: &str) -> Result<String, crate::IlluError> {
    let exemplars = exemplars()?;
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scored: Vec<(usize, &Exemplar)> = exemplars
        .iter()
        .map(|e| (score(e, &tokens), e))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by_key(|(s, _)| Reverse(*s));
    scored.truncate(MAX_EXEMPLAR_RESULTS);

    if scored.is_empty() {
        return Ok("No exemplars matched the query.".to_string());
    }

    let mut output = String::new();
    for (i, (score, exemplar)) in scored.iter().enumerate() {
        let _ = writeln!(
            output,
            "## {} — {}\n",
            exemplar.category, exemplar.title
        );
        let _ = writeln!(output, "**Slug:** `{}`  ", exemplar.slug);
        let _ = writeln!(output, "**Match score:** {score}  ");
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

    /// Compile-check that every exemplar file is real Rust. Each is a
    /// separate child module so unrelated identifiers don't collide. dead
    /// code is allowed because exemplars are demonstrations, not callable
    /// from the rest of the crate.
    #[expect(
        dead_code,
        unused_imports,
        unused_variables,
        unused_mut,
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
    }
}
```

Note: `axioms_for_test()` is a helper that needs to be added to `axioms.rs` to expose the parsed axioms slice for cross-reference testing without going through `handle_axioms`. Alternatively, since `axioms()` is private to `axioms.rs`, the test could call `handle_axioms("rust_quality")` and string-match — uglier but doesn't require an axioms.rs change. **Pick the test-helper approach:** add `pub(crate) fn axioms_for_test() -> &'static [Axiom]` to `axioms.rs` (with a `#[cfg(test)]` guard) so the cross-reference test can iterate axioms by ID directly. Add this helper at the end of `axioms.rs`:

```rust
#[cfg(test)]
pub(crate) fn axioms_for_test() -> &'static [Axiom] {
    axioms().expect("axioms parse for tests")
}
```

The `.expect()` here triggers `expect_used = "warn"` but that's a warn, not deny. To stay clean: use `.unwrap_or_else(|_| panic!(...))` — but `panic = "deny"`. Simplest: use the existing `#[expect(clippy::expect_used, reason = "test helper")]`. Final version:

```rust
#[cfg(test)]
#[expect(clippy::expect_used, reason = "test helper; panic on parse failure is acceptable")]
pub(crate) fn axioms_for_test() -> &'static [Axiom] {
    axioms().expect("axioms parse for tests")
}
```

- [ ] **Step 4: Add `axioms_for_test` helper to `src/server/tools/axioms.rs`**

Locate the `axioms()` function (private parse-once cache) and add the `axioms_for_test` helper right after it.

- [ ] **Step 5: Wire the new module in `src/server/tools/mod.rs`**

Add `pub mod exemplars;` alphabetically next to `pub mod axioms;`.

- [ ] **Step 6: Register the MCP tool in `src/server/mod.rs`**

Add a parameters struct after `AxiomsParams`:

```rust
#[derive(Deserialize, JsonSchema)]
struct ExemplarsParams {
    /// Search term for exemplars
    query: String,
}
```

Add a tool method after the `axioms` method (inside the `#[tool_router] impl IlluServer` block):

```rust
#[tool(
    name = "exemplars",
    description = "Query the curated Rust Exemplars database. Returns up to 4 compile-checked Rust files demonstrating idiomatic integrated patterns, with cross-references to the axioms each demonstrates. Use this when you want to see what an idiomatic solution looks like in practice, not just rule-by-rule guidance."
)]
async fn exemplars(
    &self,
    Parameters(params): Parameters<ExemplarsParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(query = %params.query, "Tool call: exemplars");
    let _guard = crate::status::StatusGuard::new(&format!("exemplars ▸ {}", params.query));
    let result = tools::exemplars::handle_exemplars(&params.query).map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

- [ ] **Step 7: Re-export in `src/api.rs`**

Add a re-export module after the existing `pub mod axioms { ... }`:

```rust
pub mod exemplars {
    pub use crate::server::tools::exemplars::handle_exemplars;
}
```

- [ ] **Step 8: Run the failing batch-1 test, expect failure**

Run: `cargo test --lib -- test_exemplars_batch_1_present`. Expected: should pass since the manifest, files, and lookup_code are all in place. (TDD red-step is moot here because the infrastructure and content land together — the test exists alongside the code from the start.)

- [ ] **Step 9: Run the cargo gauntlet**

```bash
cargo build
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

All must pass. The compile-check sub-modules trigger clippy on the exemplar files — any exemplar that doesn't pass clippy is a real bug that must be fixed in the exemplar before commit.

- [ ] **Step 10: Commit Task 1**

```bash
git add assets/rust_exemplars src/server/tools/exemplars.rs src/server/tools/mod.rs src/server/tools/axioms.rs src/server/mod.rs src/api.rs
git commit -m "$(cat <<'EOF'
exemplars: add infrastructure + batch 1 (errors/ownership)

Phase 2 — first MCP-tool addition since Phase 0. Introduces
assets/rust_exemplars/ as a corpus of compile-checked Rust files
demonstrating integrated idiomatic patterns, with a manifest carrying
metadata and forward-references to the axioms each exemplar demonstrates.

Infrastructure:
- assets/rust_exemplars/manifest.json — 3-entry initial manifest.
- src/server/tools/exemplars.rs — RawExemplar/Exemplar two-struct pattern,
  OnceLock parse-once cache, scoring handle_exemplars, slug-to-code
  lookup_code match, MAX_EXEMPLAR_RESULTS = 4.
- src/server/tools/mod.rs — declare new module.
- src/server/mod.rs — register `exemplars` MCP tool with single `query`
  parameter, mirroring `axioms`.
- src/api.rs — re-export handle_exemplars.
- src/server/tools/axioms.rs — add axioms_for_test() helper for the
  cross-reference test.
- 5 new tests: manifest parse, slug-has-code, no-duplicate-slugs,
  axiom-reference-resolves, batch-1-focused-query. Plus a compile_check
  sub-module that include!s each batch-1 exemplar so cargo clippy catches
  rot in exemplar code.

Batch 1 exemplars:
- errors/api_error.rs — layered ApiError enum with std::error::Error
  source chain (axioms 57, 58).
- ownership/cow_string.rs — Cow<str> normalize_key returning input slice
  unchanged on the hot path (axiom 86).
- ownership/drop_guard.rs — RAII Guard<F: FnOnce()> with .dismiss()
  escape hatch (axiom 71).
EOF
)"
```

---

## Task 2: Batch 2 — Type system patterns

**Files (create):**
- `assets/rust_exemplars/types/sealed_trait.rs`
- `assets/rust_exemplars/types/typestate_builder.rs`
- `assets/rust_exemplars/types/extension_trait.rs`

**Files (modify):**
- `assets/rust_exemplars/manifest.json` (append 3 entries)
- `src/server/tools/exemplars.rs` (add 3 arms to `lookup_code`, 3 sub-modules to `compile_check`, add `test_exemplars_batch_2_present`)

- [ ] **Step 1: Create the three Batch 2 exemplar files**

Create file `assets/rust_exemplars/types/sealed_trait.rs`:

```rust
// Sealed trait pattern: a public trait that external code can call but
// only types in this crate can implement. The seal is enforced by a
// private `Sealed` supertrait that external crates cannot name and
// therefore cannot satisfy. Adding new format variants is reserved for
// future versions of this crate.
//
// Each `mod private { pub trait Sealed {} }` is local to the file that
// owns the public trait it seals; collapsing two seals into one shared
// module weakens both because either crate's authors could then satisfy
// the other's bound by accident.

mod private {
    /// Sealed marker. External crates cannot name this trait, so they
    /// cannot satisfy the `Format: private::Sealed` bound and therefore
    /// cannot implement `Format`.
    pub trait Sealed {}
}

/// Unix-epoch timestamp in seconds — the value that `Format` impls render.
#[derive(Clone, Copy, Debug)]
pub struct Timestamp(pub u64);

/// Renders a timestamp into a human-readable string. The seal protects
/// the *set of formats* the library exposes; callers may use any
/// implementor freely, but only this crate may add new ones.
pub trait Format: private::Sealed {
    fn format(&self, ts: Timestamp) -> String;
}

pub struct Rfc3339;
pub struct EpochSeconds;

impl private::Sealed for Rfc3339 {}
impl private::Sealed for EpochSeconds {}

impl Format for Rfc3339 {
    fn format(&self, ts: Timestamp) -> String {
        // Real implementations would derive year/month/day from ts.0;
        // this exemplar focuses on the seal pattern, not date arithmetic.
        format!("rfc3339:{}", ts.0)
    }
}

impl Format for EpochSeconds {
    fn format(&self, ts: Timestamp) -> String {
        ts.0.to_string()
    }
}
```

Create file `assets/rust_exemplars/types/typestate_builder.rs`:

```rust
//! Type-state builder: required fields are encoded in the generic type
//! parameters via marker types. Calling `.build()` before every required
//! field is set is a compile error, not a runtime panic — the
//! `RequestBuilder<Url, Method>` impl block carries the values, and
//! `RequestBuilder<Unset, Unset>` does not.

pub struct Unset;
pub struct Url(String);
pub struct Method(&'static str);

pub struct RequestBuilder<U, M> {
    url: U,
    method: M,
}

impl RequestBuilder<Unset, Unset> {
    pub fn new() -> Self {
        Self {
            url: Unset,
            method: Unset,
        }
    }
}

impl Default for RequestBuilder<Unset, Unset> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> RequestBuilder<Unset, M> {
    pub fn url(self, url: impl Into<String>) -> RequestBuilder<Url, M> {
        RequestBuilder {
            url: Url(url.into()),
            method: self.method,
        }
    }
}

impl<U> RequestBuilder<U, Unset> {
    pub fn method(self, method: &'static str) -> RequestBuilder<U, Method> {
        RequestBuilder {
            url: self.url,
            method: Method(method),
        }
    }
}

// `build` only exists when both fields are typed (i.e. set). Calling it on
// any other state — even after partial setup — is a compile error.
impl RequestBuilder<Url, Method> {
    pub fn build(self) -> Request {
        Request {
            url: self.url.0,
            method: self.method.0,
        }
    }
}

pub struct Request {
    pub url: String,
    pub method: &'static str,
}
```

Create file `assets/rust_exemplars/types/extension_trait.rs`:

```rust
//! Extension trait that adds methods to a foreign type. Sealed so external
//! crates cannot add their own impls and therefore cannot accidentally
//! shadow methods we add later.

mod private {
    pub trait Sealed {}
}

/// Adds split-on-first/split-on-last helpers to any `&str`.
pub trait StrExt: private::Sealed {
    /// Splits at the first occurrence of `sep`, returning `(before, after)`
    /// with the separator excluded.
    fn split_first(&self, sep: char) -> Option<(&str, &str)>;

    /// Splits at the last occurrence of `sep`, returning `(before, after)`
    /// with the separator excluded.
    fn split_last(&self, sep: char) -> Option<(&str, &str)>;
}

impl private::Sealed for str {}

impl StrExt for str {
    fn split_first(&self, sep: char) -> Option<(&str, &str)> {
        let idx = self.find(sep)?;
        let (before, rest) = self.split_at(idx);
        Some((before, &rest[sep.len_utf8()..]))
    }

    fn split_last(&self, sep: char) -> Option<(&str, &str)> {
        let idx = self.rfind(sep)?;
        let (before, rest) = self.split_at(idx);
        Some((before, &rest[sep.len_utf8()..]))
    }
}
```

- [ ] **Step 2: Append 3 entries to the manifest**

Append the 3 type-system entries (sealed_trait, typestate_builder, extension_trait — see full list in the spec, or copy the entries from the manifest examples in Task 1's earlier "all 9 upfront" attempt) before the closing `]`:

```json
    ,
    {
      "slug": "types/sealed_trait",
      "category": "Sealed Trait",
      "title": "Sealed trait with private supertrait",
      "description": "Public trait that can be called externally but only implemented within this crate. The seal is enforced by a private Sealed supertrait that external crates cannot name and therefore cannot satisfy.",
      "triggers": ["sealed trait example", "private supertrait", "closed implementation", "extension safe"],
      "axioms_demonstrated": ["rust_quality_76_sealed_traits"],
      "source": "Rust for Rustaceans, ch. 2 §Trait Design"
    },
    {
      "slug": "types/typestate_builder",
      "category": "Type-state Builder",
      "title": "Type-state builder with required-field tracking",
      "description": "Builder where required fields are encoded in the generic type parameters via marker types. Calling .build() before every required field is set is a compile error, not a runtime panic — no Option, no unwrap.",
      "triggers": ["type state builder", "compile time required field", "ZST marker builder", "phantom builder state"],
      "axioms_demonstrated": ["rust_quality_83_zst_markers"],
      "source": "Rust for Rustaceans, ch. 3 §Type-state; typed-builder crate"
    },
    {
      "slug": "types/extension_trait",
      "category": "Extension Trait",
      "title": "Sealed extension trait on a foreign type",
      "description": "Adds methods to a foreign type (str) via a trait. Sealed so external crates cannot add competing impls; the seal also prevents accidental shadowing of methods we add later.",
      "triggers": ["extension trait example", "extending foreign type", "sealed extension trait", "method on str"],
      "axioms_demonstrated": ["rust_quality_76_sealed_traits"],
      "source": "Rust for Rustaceans, ch. 2 §Extension Traits"
    }
```

- [ ] **Step 3: Add 3 arms to `lookup_code` in `src/server/tools/exemplars.rs`**

Insert before the `_ => None,` catch-all:

```rust
        "types/sealed_trait" => Some(include_str!(
            "../../../assets/rust_exemplars/types/sealed_trait.rs"
        )),
        "types/typestate_builder" => Some(include_str!(
            "../../../assets/rust_exemplars/types/typestate_builder.rs"
        )),
        "types/extension_trait" => Some(include_str!(
            "../../../assets/rust_exemplars/types/extension_trait.rs"
        )),
```

- [ ] **Step 4: Add 3 sub-modules to `compile_check` in `src/server/tools/exemplars.rs`**

Inside `mod compile_check { ... }`, append:

```rust
        mod types_sealed_trait {
            include!("../../../assets/rust_exemplars/types/sealed_trait.rs");
        }
        mod types_typestate_builder {
            include!("../../../assets/rust_exemplars/types/typestate_builder.rs");
        }
        mod types_extension_trait {
            include!("../../../assets/rust_exemplars/types/extension_trait.rs");
        }
```

- [ ] **Step 5: Add `test_exemplars_batch_2_present` to the test module**

Append to `mod tests`:

```rust
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

        let result = handle_exemplars(
            "type state builder compile time required field ZST marker builder",
        )
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
```

- [ ] **Step 6: Run the cargo gauntlet**

```bash
cargo test --lib -- test_exemplars_batch_2_present
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 7: Commit Task 2**

```bash
git add assets/rust_exemplars src/server/tools/exemplars.rs
git commit -m "$(cat <<'EOF'
exemplars: add batch 2 (type system patterns)

Three new exemplars demonstrating type-level discipline:

- types/sealed_trait.rs — public trait gated by a private Sealed
  supertrait so external crates can call but not implement (axioms 76,
  77).
- types/typestate_builder.rs — builder where required fields are encoded
  in generic type parameters; calling .build() before all fields are set
  is a compile error, not a runtime panic; no Option, no unwrap (axiom
  83).
- types/extension_trait.rs — sealed StrExt that adds split_first /
  split_last to any &str, with the seal preventing accidental shadowing
  by external impls (axiom 76).

Manifest grows to 6 entries; lookup_code, compile_check sub-modules, and
batch-2 focused-query test added.
EOF
)"
```

---

## Task 3: Batch 3 — Performance + unsafe + integrated patterns + demo test

**Files (create):**
- `assets/rust_exemplars/perf/closed_dispatch.rs`
- `assets/rust_exemplars/unsafe_ffi/maybe_uninit_init.rs`
- `assets/rust_exemplars/unsafe_ffi/c_string_wrapper.rs`

**Files (modify):**
- `assets/rust_exemplars/manifest.json` (append 3 entries)
- `src/server/tools/exemplars.rs` (3 lookup_code arms, 3 compile_check sub-modules, `test_exemplars_batch_3_present` and `test_exemplar_demo_query_returns_new_exemplars`)

- [ ] **Step 1: Create the three Batch 3 exemplar files**

Create file `assets/rust_exemplars/perf/closed_dispatch.rs`:

```rust
//! Closed-set dispatch via enum + match, contrasted with `Box<dyn Trait>`.
//! When the variant set is fixed at the library boundary, an enum compiles
//! to direct branches (or a jump table for larger sets) — no vtable, no
//! indirect call, exhaustive matches surface missed variants at compile
//! time, and `Vec<Command>` packs each element into one variant slot
//! rather than indirecting through a fat pointer.

#[derive(Debug, Clone, Copy)]
pub enum Command {
    Increment(u32),
    Reset,
    Set(u32),
}

#[derive(Debug, Default)]
pub struct Counter {
    value: u32,
}

impl Counter {
    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Increment(by) => self.value = self.value.saturating_add(by),
            Command::Reset => self.value = 0,
            Command::Set(v) => self.value = v,
        }
    }

    pub fn run(&mut self, batch: &[Command]) {
        for cmd in batch {
            self.apply(*cmd);
        }
    }

    pub fn value(&self) -> u32 {
        self.value
    }
}
```

Create file `assets/rust_exemplars/unsafe_ffi/maybe_uninit_init.rs`:

```rust
//! Incremental initialization of a non-Default struct using MaybeUninit and
//! `&raw mut`. The pattern is needed when the value is filled in stages
//! and you cannot construct it via struct-literal syntax — common at FFI
//! boundaries where C fills in a buffer field-by-field.
//!
//! Each unsafe block is the smallest possible scope (axiom 96) with a
//! SAFETY comment naming the invariants (axiom 94).

use std::mem::MaybeUninit;

pub struct Frame {
    pub seq: u32,
    pub payload: [u8; 64],
    pub timestamp_ns: u64,
}

/// Build a Frame by initializing each field through a raw pointer.
///
/// `source` is copied into `payload` (truncated to 64 bytes if longer; the
/// remainder of `payload` is zeroed).
pub fn build_frame(seq: u32, source: &[u8], timestamp_ns: u64) -> Frame {
    let mut uninit = MaybeUninit::<Frame>::uninit();
    let ptr = uninit.as_mut_ptr();

    // SAFETY: &raw mut yields a place pointer to `seq` without materializing
    // a `&mut` to the partially-initialized Frame (which would be UB).
    // .write() initializes the field through that raw pointer.
    unsafe {
        (&raw mut (*ptr).seq).write(seq);
    }

    // Initialize payload byte-by-byte.
    let payload_ptr: *mut u8 = unsafe { (&raw mut (*ptr).payload).cast() };
    let copy_len = source.len().min(64);
    for (i, byte) in source.iter().copied().take(copy_len).enumerate() {
        // SAFETY: payload is `[u8; 64]`; payload_ptr is its first element.
        // i < copy_len <= 64, so payload_ptr.add(i) is within the field.
        unsafe {
            payload_ptr.add(i).write(byte);
        }
    }
    for i in copy_len..64 {
        // SAFETY: same bounds reasoning; i < 64.
        unsafe {
            payload_ptr.add(i).write(0);
        }
    }

    // SAFETY: see above for the field-pointer rationale.
    unsafe {
        (&raw mut (*ptr).timestamp_ns).write(timestamp_ns);
    }

    // SAFETY: every field of Frame has now been initialized — `seq`,
    // `payload`'s 64 bytes, and `timestamp_ns`. assume_init requires Frame
    // to be in a valid state for `Frame`; that holds.
    unsafe { uninit.assume_init() }
}
```

Create file `assets/rust_exemplars/unsafe_ffi/c_string_wrapper.rs`:

```rust
//! Sound FFI wrappers for C-string interop: borrow (length read), Rust→C
//! ownership transfer (string build), and reclamation (string free). Each
//! `extern "C" fn` body is panic-isolated via `catch_unwind` so a Rust
//! panic returns a sentinel value rather than aborting the process
//! (axiom 100). Ownership is documented in each fn's rustdoc; the seller-
//! reclaimer pair must be matched (axiom 102).

use std::ffi::{CStr, CString, c_char};
use std::panic::catch_unwind;

/// Returns the length of a NUL-terminated C string in bytes (excluding NUL).
/// Returns 0 if `s` is null or any internal panic occurs.
///
/// # Safety
/// `s` must be a valid pointer to a NUL-terminated C string for the duration
/// of the call, and no other mutator may modify the string while this runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ffi_string_len(s: *const c_char) -> usize {
    if s.is_null() {
        return 0;
    }
    catch_unwind(|| {
        // SAFETY: caller-documented non-null and NUL-terminated precondition.
        unsafe { CStr::from_ptr(s) }.to_bytes().len()
    })
    .unwrap_or(0)
}

/// Builds a Rust-owned C string and transfers ownership to the caller. The
/// caller MUST release the returned pointer with `ffi_string_free` and no
/// other deallocator. Returns null on internal failure (allocation failure,
/// embedded NUL, panic).
#[unsafe(no_mangle)]
pub extern "C" fn ffi_string_make() -> *mut c_char {
    catch_unwind(|| {
        CString::new("hello from rust").map_or(std::ptr::null_mut(), CString::into_raw)
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Reclaims and frees a string previously returned by `ffi_string_make`.
///
/// # Safety
/// `s` must be a pointer obtained from `ffi_string_make` and not yet freed
/// by any other path. Passing a pointer from any other source is UB.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ffi_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller-documented provenance precondition.
    drop(unsafe { CString::from_raw(s) });
}
```

- [ ] **Step 2: Append 3 entries to the manifest**

Append before the closing `]`:

```json
    ,
    {
      "slug": "perf/closed_dispatch",
      "category": "Enum Dispatch",
      "title": "Closed-set command dispatch via enum + match",
      "description": "Counter that processes a stream of commands via an enum + match rather than Box<dyn Command>. No vtable, no indirect call; the match compiles to direct branches; Vec<Command> packs each element rather than indirecting through a fat pointer.",
      "triggers": ["enum dispatch example", "match closed set", "command pattern enum", "no dyn dispatch"],
      "axioms_demonstrated": ["rust_quality_90_enum_dispatch"],
      "source": "Rust for Rustaceans, ch. 2 §Trait Objects; enum_dispatch crate"
    },
    {
      "slug": "unsafe_ffi/maybe_uninit_init",
      "category": "MaybeUninit Init",
      "title": "Incremental MaybeUninit initialization with &raw mut",
      "description": "Builds a fixed-size buffer field-by-field through &raw mut, never materializing a &mut to the partially-initialized value. Each unsafe block is scoped to the exact unsafe operation; each has a SAFETY comment naming the invariants.",
      "triggers": ["MaybeUninit example", "raw mut init pattern", "field by field init", "incremental struct init"],
      "axioms_demonstrated": ["rust_quality_97_maybe_uninit", "rust_quality_94_unsafe_block_discipline", "rust_quality_96_unsafe_block_scope"],
      "source": "std::mem::MaybeUninit rustdoc; Rustonomicon §Working with Uninitialized Memory"
    },
    {
      "slug": "unsafe_ffi/c_string_wrapper",
      "category": "FFI Strings Example",
      "title": "FFI string borrow-and-transfer pair",
      "description": "Three extern \"C\" functions: reading the length of a caller-owned NUL-terminated C string (CStr::from_ptr borrow), building a Rust-owned C string transferred to the caller (CString::into_raw), and reclaiming such a pointer for free (CString::from_raw). Each fn body is panic-isolated via catch_unwind.",
      "triggers": ["FFI string wrapper", "extern C string ownership", "CStr CString example", "FFI safe panic"],
      "axioms_demonstrated": ["rust_quality_100_ffi_boundary", "rust_quality_102_ffi_strings", "rust_quality_94_unsafe_block_discipline", "rust_quality_95_unsafe_fn_contract"],
      "source": "std::ffi::CStr / CString rustdoc; Rust Reference §Foreign Function Interface"
    }
```

- [ ] **Step 3: Add 3 arms to `lookup_code`**

```rust
        "perf/closed_dispatch" => Some(include_str!(
            "../../../assets/rust_exemplars/perf/closed_dispatch.rs"
        )),
        "unsafe_ffi/maybe_uninit_init" => Some(include_str!(
            "../../../assets/rust_exemplars/unsafe_ffi/maybe_uninit_init.rs"
        )),
        "unsafe_ffi/c_string_wrapper" => Some(include_str!(
            "../../../assets/rust_exemplars/unsafe_ffi/c_string_wrapper.rs"
        )),
```

- [ ] **Step 4: Add 3 sub-modules to `compile_check`**

```rust
        mod perf_closed_dispatch {
            include!("../../../assets/rust_exemplars/perf/closed_dispatch.rs");
        }
        mod unsafe_ffi_maybe_uninit_init {
            include!("../../../assets/rust_exemplars/unsafe_ffi/maybe_uninit_init.rs");
        }
        mod unsafe_ffi_c_string_wrapper {
            include!("../../../assets/rust_exemplars/unsafe_ffi/c_string_wrapper.rs");
        }
```

- [ ] **Step 5: Add `test_exemplars_batch_3_present` and `test_exemplar_demo_query_returns_new_exemplars`**

```rust
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
```

- [ ] **Step 6: Run the cargo gauntlet**

```bash
cargo test --lib -- test_exemplars
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 7: Commit Task 3**

```bash
git add assets/rust_exemplars src/server/tools/exemplars.rs
git commit -m "$(cat <<'EOF'
exemplars: add batch 3 (perf + unsafe + ffi) + demo test

Final three exemplars closing Phase 2:

- perf/closed_dispatch.rs — enum + match command dispatch on a Counter,
  the closed-set alternative to Box<dyn Command> (axioms 88, 90).
- unsafe_ffi/maybe_uninit_init.rs — incremental Frame initialization
  through &raw mut, with per-block SAFETY comments and smallest-scope
  unsafe blocks (axioms 94, 96, 97).
- unsafe_ffi/c_string_wrapper.rs — borrow / Rust→C transfer / Rust→C
  reclaim trio of extern "C" fns, each panic-isolated via catch_unwind
  (axioms 94, 95, 100, 102).

Manifest grows to 9 entries; lookup_code complete; compile_check sub-
modules cover all 9 files; batch-3 focused-query test plus a demo test
asserting >= 3 of the 9 exemplar categories surface for a broad query
join the test suite.

Phase 2 (exemplars infrastructure) closes here.
EOF
)"
```

---

## Task 4: End-to-End Verification

- [ ] **Step 1: Full cargo gauntlet**

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 2: Plan-reconciliation pass before final review** — if any content fix-ups landed during execution, ensure the plan's draft sections were updated to match the post-fix files.

---

## Verification Summary

After all tasks: 9 exemplars across 3 batches; full infrastructure (module + manifest + MCP tool + re-export); 8 new tests (5 validity + 3 batch + 1 demo) plus a 9-submodule compile_check; cargo gauntlet clean. Phase 2 closes the exemplars-infrastructure milestone.

## Risks Realized During Execution

- **`include_str!` path arithmetic.** Three-level relative paths from `src/server/tools/` are easy to miscount; verify with a build before each commit.
- **Clippy on exemplar code.** Exemplars are real Rust under the workspace's clippy denylist; expect clippy issues to fail Task 1's gauntlet on first attempt and require exemplar fixes.
- **Cross-reference axiom IDs.** The manifest references axioms by ID — ID typos fail `test_every_axiom_reference_resolves`. Reviewers should verify the listed axiom IDs match the actual corpus.
- **Plan drift** between draft exemplar code and post-fix files (recurring across all prior slices). Reconcile before final review.
