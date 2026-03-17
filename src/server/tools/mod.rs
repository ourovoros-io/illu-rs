pub mod context;
pub mod docs;
pub mod impact;
pub mod overview;
pub mod query;
pub mod tree;

/// Truncate a string at a char boundary, appending "..." if truncated.
pub(crate) fn truncate_snippet(s: &str, max_len: usize) -> std::borrow::Cow<'_, str> {
    if s.len() <= max_len {
        std::borrow::Cow::Borrowed(s)
    } else {
        let end = s.floor_char_boundary(max_len);
        std::borrow::Cow::Owned(format!("{}...", &s[..end]))
    }
}
