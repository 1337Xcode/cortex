//! The hook's promise to the agent: whatever goes wrong inside it, the agent sees exit code 0 and
//! nothing printed. A panic anywhere in the hook, on any of its threads, is silenced and caught.

use std::panic::{self, UnwindSafe};

/// Runs `body` with every panic silenced, and reports whether it finished without one.
pub(crate) fn guard(body: impl FnOnce() + UnwindSafe) -> bool {
    // INVARIANT: fail-open. The default panic hook would print the panic to stderr, which the
    // agent may show or treat as hook output; this one prints nothing, for every thread.
    panic::set_hook(Box::new(|_| {}));
    // INVARIANT: fail-open. A panic ends here instead of in an exit code the agent could act on.
    panic::catch_unwind(body).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body that panics is caught and reported; one that returns is reported as finished.
    #[test]
    fn a_panic_is_caught_and_reported() {
        #[allow(clippy::panic, reason = "the panic is what the guard must catch")]
        let panicked = !guard(|| panic!("the hook broke"));
        assert!(panicked);
        assert!(guard(|| {}));
        // Other tests in this binary report their own panics.
        drop(panic::take_hook());
    }
}
