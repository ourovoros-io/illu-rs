// Extension trait that adds methods to a foreign type. Sealed so external
// crates cannot add their own impls and therefore cannot accidentally
// shadow methods we add later.

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
