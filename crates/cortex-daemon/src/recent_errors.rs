//! The daemon's latest errors, kept for its status: what failed, when it last did, and how many
//! times. A message says what the daemon was doing and why it failed, in the daemon's or the
//! store's words, never with file contents, prompt text or tool output.

use std::collections::VecDeque;

use cortex_core::ipc::RecentError;
use cortex_core::one_line;

/// How many errors are kept, the oldest forgotten first: enough to see a pattern on one screen of
/// status.
const KEPT: usize = 10;

/// The longest message kept, in characters: a store error naming a long path stays readable, and
/// the status answer stays far under a frame's limit.
const MAX_MESSAGE_CHARS: usize = 1_000;

/// The latest errors.
#[derive(Debug, Default)]
pub(crate) struct RecentErrors {
    /// The errors, oldest first.
    kept: VecDeque<RecentError>,
}

impl RecentErrors {
    /// Keeps `message`, met at `at_ms` in UTC milliseconds, as one line of at most
    /// [`MAX_MESSAGE_CHARS`] characters, so that it prints on one line and cannot steer a
    /// terminal. A message already kept counts once more and becomes the latest, at the new time:
    /// failures that repeat, however they alternate, take one place each, and the rest stay.
    pub(crate) fn note(&mut self, message: &str, at_ms: i64) {
        let message = one_line(message, MAX_MESSAGE_CHARS);
        let count = match self.kept.iter().position(|kept| kept.message == message) {
            Some(place) => self
                .kept
                .remove(place)
                .map_or(1, |kept| kept.count.saturating_add(1)),
            None => 1,
        };
        if self.kept.len() == KEPT {
            self.kept.pop_front();
        }
        self.kept.push_back(RecentError {
            at_ms,
            count,
            message,
        });
    }

    /// The errors kept, oldest first.
    pub(crate) fn list(&self) -> Vec<RecentError> {
        self.kept.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each error as its message, count and time.
    fn summary(errors: &RecentErrors) -> Vec<(String, u64, i64)> {
        errors
            .list()
            .into_iter()
            .map(|error| (error.message, error.count, error.at_ms))
            .collect()
    }

    /// `a`, `a`, `b`, `a` is `b` once at 3, then `a` three times, last at 4: an error met again
    /// is kept once, as the latest.
    #[test]
    fn an_error_met_again_is_kept_once_as_the_latest() {
        let mut errors = RecentErrors::default();
        for (message, at_ms) in [("a", 1), ("a", 2), ("b", 3), ("a", 4)] {
            errors.note(message, at_ms);
        }
        assert_eq!(
            summary(&errors),
            [("b".to_owned(), 1, 3), ("a".to_owned(), 3, 4)]
        );
    }

    /// One store failure, then two other errors alternating a hundred times: the store failure is
    /// still kept, oldest, and the others are kept once each, 50 times apiece, `x` last at 99 and
    /// `y` at 100.
    #[test]
    fn alternating_errors_do_not_push_others_out() {
        let mut errors = RecentErrors::default();
        errors.note("store", 0);
        for number in 0..100 {
            let message = if number % 2 == 0 { "x" } else { "y" };
            errors.note(message, number + 1);
        }
        assert_eq!(
            summary(&errors),
            [
                ("store".to_owned(), 1, 0),
                ("x".to_owned(), 50, 99),
                ("y".to_owned(), 50, 100)
            ]
        );
    }

    /// Twelve different errors keep the latest ten, `e2` to `e11`, oldest first.
    #[test]
    fn only_the_latest_errors_are_kept() {
        let mut errors = RecentErrors::default();
        for number in 0..12 {
            errors.note(&format!("e{number}"), number);
        }
        let kept: Vec<String> = errors
            .list()
            .into_iter()
            .map(|error| error.message)
            .collect();
        let expected: Vec<String> = (2..12).map(|number| format!("e{number}")).collect();
        assert_eq!(kept, expected);
    }

    /// A message is kept as one line of at most 1,000 characters: `a\nb\u{1b}` and 1,500 letters
    /// keep `a b ` and 996 letters.
    #[test]
    fn a_message_is_kept_as_one_bounded_line() {
        let mut errors = RecentErrors::default();
        errors.note(&format!("a\nb\u{1b}{}", "é".repeat(1_500)), 1);
        let messages: Vec<String> = errors
            .list()
            .into_iter()
            .map(|error| error.message)
            .collect();
        assert_eq!(messages, [format!("a b {}", "é".repeat(996))]);
    }
}
