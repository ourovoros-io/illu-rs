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
