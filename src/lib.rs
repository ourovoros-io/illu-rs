//! Library support for the `illu` MCP server and CLI.
//!
//! The stable user surface is the CLI/MCP protocol plus the curated [`api`]
//! facade. Implementation modules are private so internal database, indexer,
//! server, and rust-analyzer wiring can evolve without becoming accidental
//! semver commitments.

#![deny(missing_docs)]
#![warn(unreachable_pub, broken_intra_doc_links)]

mod agents;
pub mod api;
mod db;
pub mod error;
mod git;
mod indexer;
mod ra;
mod registry;
mod server;
mod status;

pub use error::{IlluError, Result};

/// Truncate a string at a char boundary, appending "..." if truncated.
#[must_use]
pub fn truncate_at(s: &str, max_len: usize) -> std::borrow::Cow<'_, str> {
    if s.len() <= max_len {
        std::borrow::Cow::Borrowed(s)
    } else {
        let end = s.floor_char_boundary(max_len);
        std::borrow::Cow::Owned(format!("{}...", &s[..end]))
    }
}
