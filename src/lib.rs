pub mod db;
pub mod git;
pub mod indexer;
pub mod ra;
pub mod registry;
pub mod server;
pub mod status;

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
