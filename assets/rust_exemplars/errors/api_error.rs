// Layered API error: a public `ApiError` enum (variants are a stable contract)
// with helpers for constructing internal/external variants and a `source`
// chain preserved via `std::error::Error::source`. The pattern that
// `thiserror` automates, spelled out so the underlying machinery is visible.

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
