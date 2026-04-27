// `Cow<'a, str>` for sometimes-borrowed-sometimes-owned config: returns a
// borrow when the input is already in the desired form, only allocating
// when normalization is required. Avoids the
// "always-clone-because-the-API-takes-String" anti-pattern.

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
