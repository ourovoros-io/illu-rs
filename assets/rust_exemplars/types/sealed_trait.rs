// Sealed trait pattern: a public trait that external code can call but
// only types in this crate can implement. The seal is enforced by a
// private `Sealed` supertrait that external crates cannot name and
// therefore cannot satisfy. Adding new impls is reserved for future
// versions of this crate.

mod private {
    /// Sealed marker. External crates cannot name this trait, so they
    /// cannot satisfy the `Format: private::Sealed` bound and therefore
    /// cannot implement `Format`.
    pub trait Sealed {}
}

/// Public trait — external crates may call its methods on values of types
/// that implement it, but cannot add their own implementations.
pub trait Format: private::Sealed {
    fn format(&self) -> String;
}

pub struct Rfc3339;
pub struct Iso8601Date;

impl private::Sealed for Rfc3339 {}
impl private::Sealed for Iso8601Date {}

impl Format for Rfc3339 {
    fn format(&self) -> String {
        "1970-01-01T00:00:00Z".to_string()
    }
}

impl Format for Iso8601Date {
    fn format(&self) -> String {
        "1970-01-01".to_string()
    }
}
