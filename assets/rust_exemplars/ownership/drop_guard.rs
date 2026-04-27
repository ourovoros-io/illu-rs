// RAII drop-guard that runs an arbitrary closure when the guard is dropped,
// including via panic unwinding. Useful for paired setup/teardown where
// early returns or panics must still trigger cleanup. `Guard::dismiss`
// consumes the guard without firing the closure on the success path.

/// Runs `f` exactly once, when the guard is dropped.
pub struct Guard<F: FnOnce()> {
    cleanup: Option<F>,
}

impl<F: FnOnce()> Guard<F> {
    pub fn new(cleanup: F) -> Self {
        Self {
            cleanup: Some(cleanup),
        }
    }

    /// Consumes the guard without running the cleanup closure. Use on the
    /// success path when the cleanup is no longer required.
    pub fn dismiss(mut self) {
        self.cleanup.take();
    }
}

impl<F: FnOnce()> Drop for Guard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.cleanup.take() {
            f();
        }
    }
}
