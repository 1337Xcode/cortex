//! The note a check gets the moment before it runs again: the same command line, in the same
//! working folder, already ran in the session with no source change since, so the agent is told
//! what that run gave before it spends the time to run it again.
//!
//! ```text
//! [cortex] `pytest tests/test_ops.py` ran earlier in this session with no source change since:
//! exit 1, failing: test_divide_by_zero.
//! ```
//!
//! (one line, broken here to fit). The failing tests are the ones the run's output named, at most
//! [`MAX_NAMED_FAILING`] of them and then how many more; a run that named none, or passed, gets its
//! exit code alone. The note says no time ("earlier in this session"), so the same run gives the same note. The command
//! is shown in backquotes only when it is plain text of at most [`MAX_SHOWN_COMMAND_CHARS`]
//! characters with no backquote of its own; otherwise the note says "This command".

use std::fmt::Write as _;

use cortex_core::{estimate_tokens, is_plain};

/// The most tokens the note takes, as estimated.
pub const MAX_REDUNDANT_NOTE_TOKENS: usize = 60;

/// The longest command, in characters, the note shows in backquotes.
pub const MAX_SHOWN_COMMAND_CHARS: usize = 80;

/// The most failing tests the note names.
pub const MAX_NAMED_FAILING: usize = 3;

/// What an earlier run of a command gave, as the note tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EarlierRun {
    /// The run's exit code.
    pub exit_code: i32,
    /// The first failing tests the run's output named, in the order it named them: at most
    /// [`MAX_NAMED_FAILING`] are named, a name that is not plain text never.
    pub failing: Vec<String>,
    /// How many failing tests the output named in all, [`EarlierRun::failing`] among them.
    pub failing_count: usize,
}

/// The note the command `command` gets before it runs again, its earlier run having given `run`:
/// none only when even the note without failing tests would not fit
/// [`MAX_REDUNDANT_NOTE_TOKENS`].
#[must_use]
pub fn redundant_note(command: &str, run: &EarlierRun) -> Option<String> {
    let subject = if is_shown(command) {
        format!("`{command}`")
    } else {
        "This command".to_owned()
    };
    let names: Vec<&str> = if run.exit_code == 0 {
        Vec::new()
    } else {
        run.failing
            .iter()
            .take(MAX_NAMED_FAILING)
            .map(String::as_str)
            .filter(|name| is_plain(name))
            .collect()
    };
    let mut shown = names.len();
    loop {
        let text = note_text(&subject, run, names.get(..shown).unwrap_or_default());
        // INVARIANT: budgeted (fewer failing tests are named until the note fits its cap)
        if estimate_tokens(&text) <= MAX_REDUNDANT_NOTE_TOKENS {
            return Some(text);
        }
        shown = shown.checked_sub(1)?;
    }
}

/// Whether `command` is shown as it is: plain text, no backquote that would end its quoting, at
/// most [`MAX_SHOWN_COMMAND_CHARS`] characters, and something other than spaces.
fn is_shown(command: &str) -> bool {
    is_plain(command)
        && !command.contains('`')
        && command.chars().count() <= MAX_SHOWN_COMMAND_CHARS
        && !command.trim().is_empty()
}

/// The note about `subject`, its run `run` naming `shown` of its failing tests.
fn note_text(subject: &str, run: &EarlierRun, shown: &[&str]) -> String {
    let mut text = format!(
        "[cortex] {subject} ran earlier in this session with no source change since: exit {}",
        run.exit_code
    );
    if !shown.is_empty() {
        let _ = write!(text, ", failing: {}", shown.join(", "));
        let more = run.failing_count.saturating_sub(shown.len());
        if more > 0 {
            let _ = write!(text, " and {more} more");
        }
    }
    text.push_str(".\n");
    text
}
