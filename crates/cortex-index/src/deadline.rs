//! The time limit on reading one file, checked from tree-sitter's progress callbacks so that a
//! parse or a query that runs too long stops where it is.

use std::cell::Cell;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

/// The moment reading a file must stop, and whether a check has found it passed.
#[derive(Debug)]
pub(crate) struct Deadline {
    /// The moment; `None` when the allowance is too long to represent, so it never passes.
    at: Option<Instant>,
    /// Whether a check found the moment passed.
    has_passed: Cell<bool>,
}

impl Deadline {
    /// The deadline `allowance` from now.
    pub(crate) fn after(allowance: Duration) -> Self {
        Self {
            at: Instant::now().checked_add(allowance),
            has_passed: Cell::new(false),
        }
    }

    /// `Break`, which tree-sitter reads as "stop", once the deadline has passed; the deadline
    /// remembers that it did.
    pub(crate) fn check(&self) -> ControlFlow<()> {
        if self.at.is_some_and(|at| Instant::now() >= at) {
            self.has_passed.set(true);
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    /// Whether a check has found the deadline passed.
    pub(crate) fn has_passed(&self) -> bool {
        self.has_passed.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no time allowed, the first check stops and is remembered; before it, nothing is.
    #[test]
    fn no_time_stops_at_once() {
        let deadline = Deadline::after(Duration::ZERO);
        assert!(!deadline.has_passed());
        assert_eq!(deadline.check(), ControlFlow::Break(()));
        assert!(deadline.has_passed());
    }

    /// With time left, a check lets the work go on, even for an allowance too long to represent.
    #[test]
    fn time_left_continues() {
        for allowance in [Duration::from_secs(3600), Duration::MAX] {
            let deadline = Deadline::after(allowance);
            assert_eq!(deadline.check(), ControlFlow::Continue(()));
            assert!(!deadline.has_passed());
        }
    }
}
