//! A pause: while a repository holds one, nothing Cortex works out reaches an agent there, and
//! everything the agents do is still recorded, so the pause costs nothing that is learned.
//!
//! It is a file in the repository's `.cortex`, written by `cortex pause` and removed by
//! `cortex resume`, for the same reason the off marker is a file: only the daemon writes the
//! store, so a command that wrote this there would need a request of its own. The file holds
//! either the moment the pause ends or a word saying it holds until someone lifts it, so that a
//! moment never has to stand for "no end".

use std::path::Path;

use crate::adoption::MARKER;

/// The file in a repository's `.cortex` that holds a pause.
pub const PAUSED_FILE: &str = "paused";

/// What the file holds when the pause does not end by itself.
const UNTIL_LIFTED: &str = "until lifted";

/// How long a pause file may be: enough for the word or a moment, and short enough that a file
/// something else wrote is refused rather than read.
const MOST_BYTES: usize = 64;

/// A pause, as its file says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pause {
    /// It holds until `cortex resume`.
    UntilLifted,
    /// It holds until this moment, in UTC milliseconds.
    Until(i64),
}

impl Pause {
    /// What the file says, for a pause to be written.
    #[must_use]
    pub fn written(self) -> String {
        match self {
            Self::UntilLifted => UNTIL_LIFTED.to_owned(),
            Self::Until(moment) => moment.to_string(),
        }
    }

    /// The pause `text` says, or none when it says nothing this build understands: an older or
    /// newer build's word, or a file something else wrote.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        let text = text.trim();
        if text == UNTIL_LIFTED {
            return Some(Self::UntilLifted);
        }
        text.parse().ok().map(Self::Until)
    }

    /// Whether this pause still holds at `now_ms`. A pause until a moment holds up to that moment
    /// and not past it.
    #[must_use]
    pub fn holds_at(self, now_ms: i64) -> bool {
        match self {
            Self::UntilLifted => true,
            Self::Until(moment) => now_ms < moment,
        }
    }
}

/// The pause the repository at `folder` holds, or none when it holds none, the file cannot be read
/// or it says nothing this build understands.
///
/// A file this build cannot read leaves the repository unpaused rather than paused: a pause that
/// nobody can lift, because nobody can see why it holds, is worse than one that has lapsed.
#[must_use]
pub fn pause_of(folder: &Path) -> Option<Pause> {
    let path = folder.join(MARKER).join(PAUSED_FILE);
    let size = path.metadata().ok()?.len();
    if size > MOST_BYTES as u64 {
        return None;
    }
    Pause::read(&std::fs::read_to_string(&path).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pause until a moment is written as that moment and read back as it; one until it is
    /// lifted is written as words, since a moment far enough away would read as a mistake.
    #[test]
    fn a_pause_is_written_and_read_back() {
        assert_eq!(
            Pause::read(&Pause::Until(1_700_000_000_000).written()),
            Some(Pause::Until(1_700_000_000_000))
        );
        assert_eq!(
            Pause::read(&Pause::UntilLifted.written()),
            Some(Pause::UntilLifted)
        );
        assert_eq!(Pause::UntilLifted.written(), "until lifted");
    }

    /// Whitespace around what a shell or an editor wrote is ignored; anything else is not read, so
    /// a file another program left in `.cortex` does not pause a repository.
    #[test]
    fn only_what_this_build_writes_is_read() {
        assert_eq!(
            Pause::read(" 1700000000000\n"),
            Some(Pause::Until(1_700_000_000_000))
        );
        assert_eq!(Pause::read(""), None);
        assert_eq!(Pause::read("forever"), None);
        assert_eq!(Pause::read("1700000000000 and more"), None);
        assert_eq!(Pause::read("until lifted please"), None);
    }

    /// A pause until a moment holds up to it and not at it: at the moment itself the next prompt
    /// is answered again.
    #[test]
    fn a_pause_until_a_moment_lapses_at_it() {
        let pause = Pause::Until(1_000);
        assert!(pause.holds_at(999));
        assert!(!pause.holds_at(1_000));
        assert!(!pause.holds_at(1_001));
        assert!(Pause::UntilLifted.holds_at(i64::MAX));
    }
}
