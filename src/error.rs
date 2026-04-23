//! Crate-wide error type.
//!
//! `IlluError` unifies failure modes across indexer, server, docs, git, and
//! agent subsystems. It is `Send + Sync + 'static` so errors propagate across
//! `tokio::spawn_blocking` without losing their typed `source()` chain — the
//! original motivation for migrating off the non-`Send` `Box<dyn Error>`.
//!
//! Note on `format_error_chain` (`src/server/mod.rs`): that helper still
//! exists, but it is no longer a `Send` workaround. It flattens an error's
//! `source()` chain into a single `String` for `McpError::internal_error`,
//! whose payload is `String`-typed — plain `Display` would drop the causal
//! chain and MCP clients would only see the outermost message. Callers
//! pass an `IlluError` reference to it; the helper no longer has to
//! special-case `Box<dyn Error>`.
//!
//! ## Variants
//!
//! Transparent variants (`#[from]`) wrap foreign errors whose `Display` /
//! `source()` are already informative. Domain variants carry a human-readable
//! message for cases where no typed source exists (e.g. a parser rejects an
//! attribute and we want to surface the offending text, not a lower-level
//! rusqlite error).
//!
//! `Other(String)` is the escape hatch used during the migration from the
//! prior `Box<dyn std::error::Error>` scheme; prefer a domain variant when
//! adding new error sites. Several domain variants are defined but not yet
//! plumbed through every call site — follow-up work will route them (see
//! the tracking memo in user memory).
//!
//! ## Axioms
//!
//! - `#[non_exhaustive]` so downstream pattern matches stay forward-compatible
//!   when new variants land. Construction via `#[from]` / `From` impls is
//!   unaffected because those are inherent to `IlluError` itself.
//! - `thiserror` handles `std::error::Error` impls and `source()` chaining.
//! - Every variant is `Send + Sync` — verified by the `_assert_traits`
//!   compile-time check at the bottom of this module.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, IlluError>;

/// Errors produced by any illu-rs subsystem.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IlluError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Db(#[from] rusqlite::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    TomlDe(#[from] toml::de::Error),

    #[error(transparent)]
    Walk(#[from] walkdir::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Url(#[from] url::ParseError),

    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),

    #[error(transparent)]
    FromUtf8(#[from] std::string::FromUtf8Error),

    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),

    #[error(transparent)]
    Fmt(#[from] std::fmt::Error),

    #[error(transparent)]
    Dialoguer(#[from] dialoguer::Error),

    /// Boxed because `rmcp::service::ServerInitializeError` is large (>500B);
    /// storing it inline would balloon every `Result<_, IlluError>` and trip
    /// clippy's `result_large_err` lint.
    #[error(transparent)]
    RmcpInit(Box<rmcp::service::ServerInitializeError>),

    #[error(transparent)]
    Ra(#[from] crate::ra::error::RaError),

    /// Indexing pipeline failure (tree-sitter parse failed, workspace layout
    /// rejected, file skipped for a non-IO reason).
    #[error("indexing: {0}")]
    Indexing(String),

    /// Cargo / tsconfig / pyproject workspace discovery or resolution failed.
    #[error("workspace: {0}")]
    Workspace(String),

    /// Generic parser-layer error that lacks a typed source.
    #[error("parser: {0}")]
    Parse(String),

    /// Agent detection, selection, or file-writing error.
    #[error("agent: {0}")]
    Agent(String),

    /// Dashboard (axum) failure.
    #[error("dashboard: {0}")]
    Dashboard(String),

    /// Git subprocess or repository-state error.
    #[error("git: {0}")]
    Git(String),

    /// Docs fetcher or renderer error.
    #[error("docs: {0}")]
    Docs(String),

    /// Untyped escape hatch. Prefer a domain variant when adding new sites;
    /// this is retained for the one-shot string-error sites that would
    /// otherwise need a variant per call.
    #[error("{0}")]
    Other(String),
}

impl From<String> for IlluError {
    fn from(s: String) -> Self {
        IlluError::Other(s)
    }
}

impl From<&str> for IlluError {
    fn from(s: &str) -> Self {
        IlluError::Other(s.to_string())
    }
}

impl From<rmcp::service::ServerInitializeError> for IlluError {
    fn from(e: rmcp::service::ServerInitializeError) -> Self {
        IlluError::RmcpInit(Box::new(e))
    }
}

impl From<toml_edit::TomlError> for IlluError {
    fn from(e: toml_edit::TomlError) -> Self {
        // toml_edit's error type is not `Error + Send + Sync` in older minor
        // versions; stringifying preserves the caller-visible message.
        IlluError::Other(e.to_string())
    }
}

impl From<regex_lite::Error> for IlluError {
    fn from(e: regex_lite::Error) -> Self {
        IlluError::Other(e.to_string())
    }
}

// Compile-time assertion that `IlluError: Send + Sync + 'static` so it can
// cross `tokio::spawn_blocking` boundaries without stringification. Placed in
// a `const _` block so it's evaluated unconditionally at compile time and the
// compiler does not consider the helper functions dead code.
const _: () = {
    const fn assert_send<T: Send>() {}
    const fn assert_sync<T: Sync>() {}
    const fn assert_static<T: 'static>() {}
    assert_send::<IlluError>();
    assert_sync::<IlluError>();
    assert_static::<IlluError>();
};
