//! The line that tells the user, once, that Cortex set the repository up on its own. The daemon
//! that set it up leaves `.cortex/adopted`; the daemon that starts on the repository takes the
//! file away and holds the line, which the next answer to a session's start or a prompt that can
//! carry a message for the user gives, never the model. A line whose answer never reached its hook
//! is held again.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use cortex_core::ADOPTED_FILE;

/// What the user is told.
pub(crate) const WELCOME: &str =
    "Cortex set up this repository for your agents; `cortex off` turns it off here.";

/// Whether the line is held for the user.
#[derive(Debug, Default)]
pub(crate) struct WelcomeLine {
    /// Whether it is held.
    is_held: AtomicBool,
}

impl WelcomeLine {
    /// The line held when the repository's Cortex folder `folder` holds `adopted`, which is taken
    /// away: from then on this daemon holds it. A file that cannot be taken away is logged, and
    /// the line held all the same: a later daemon would tell the user again, which is better than
    /// never.
    pub(crate) fn of(folder: &Path) -> Self {
        let file = folder.join(ADOPTED_FILE);
        let is_held = file.is_file();
        if is_held && let Err(error) = std::fs::remove_file(&file) {
            tracing::warn!(error = %error, "the note of the repository's adoption was not removed");
        }
        Self {
            is_held: AtomicBool::new(is_held),
        }
    }

    /// The line, when it is held, which from then on is not, unless it is put back.
    pub(crate) fn take(&self) -> Option<&'static str> {
        self.is_held
            .swap(false, Ordering::AcqRel)
            .then_some(WELCOME)
    }

    /// Holds the line again: its telling never reached the user.
    pub(crate) fn put_back(&self) {
        self.is_held.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder holding `adopted` gives the line once, and the file is gone; put back, the line is
    /// given once more. A folder without it gives none.
    #[test]
    fn the_line_is_given_once() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        assert_eq!(WelcomeLine::of(folder.path()).take(), None);
        std::fs::write(folder.path().join(ADOPTED_FILE), "")?;
        let welcome = WelcomeLine::of(folder.path());
        assert!(!folder.path().join(ADOPTED_FILE).exists());
        assert_eq!(welcome.take(), Some(WELCOME));
        assert_eq!(welcome.take(), None);
        welcome.put_back();
        assert_eq!(welcome.take(), Some(WELCOME));
        assert_eq!(welcome.take(), None);
        Ok(())
    }
}
