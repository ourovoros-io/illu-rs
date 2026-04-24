//! Library support for the `illu` MCP server and CLI.
//!
//! The stable user surface is the CLI/MCP protocol. Rust modules remain public
//! where the sibling binary and integration tests need them, but new modules
//! should prefer private implementation details plus explicit facade re-exports.

#![warn(unreachable_pub, broken_intra_doc_links)]

pub mod agents;
pub mod db;
pub mod error;
pub mod git;
pub mod indexer;
pub mod ra;
pub mod registry;
pub mod server;
pub mod status;

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
