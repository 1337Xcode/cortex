//! The failure map: a few lines given beside a failed check's output, naming the repository lines
//! its stack ran through, innermost first, with each line's code as it is on disk and the
//! definition holding it. It is given only when it adds what the output lacks: a frame whose code
//! line the output does not show (cargo's panics, Go's and Node's frames print locations only), or
//! a frame in a file the task has not read or edited. Otherwise the output says it already, and
//! the map stays silent.
//!
//! ```text
//! [cortex] The failing test test_divide runs through these lines, as they are now:
//!   calc/ops.py:14 in divide: return a / b
//!   calc/main.py:31 in run: print(divide(x, y))
//! ```

use std::fmt::Write as _;

use cortex_core::{Ulid, estimate_tokens, is_plain};
use cortex_store::{NodeId, Queries, StoreError};

use crate::grounded_failure::{GroundedFailure, GroundedFrame};

/// The most frames a failure map names.
pub const MAX_MAPPED_FRAMES: usize = 3;

/// The most tokens a failure map takes, as estimated.
pub const MAX_FAILURE_MAP_TOKENS: usize = 120;

/// The most characters of a frame's code line shown: enough for a statement, never a whole
/// minified file.
const MAX_CODE_CHARS: usize = 120;

/// The most characters of the failing test's name shown: a parametrized test's id can run to
/// hundreds of characters, which would leave no room for the frames.
const MAX_TEST_NAME_CHARS: usize = 80;

/// The fewest characters a code line holds to be looked for in the output: a shorter one (`}`,
/// `pass`, `return`) would be found in almost any output.
const MIN_MATCHED_CODE_CHARS: usize = 8;

/// How many of a task's latest events are looked at for whether it read or edited a file.
const EVENTS_LOOKED_AT: usize = 2_000;

/// A failure map: its text and the files it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureMap {
    /// The text, one line of introduction and one line a frame.
    pub text: String,
    /// The files its frames are in, in the order it names them, each once.
    pub files: Vec<NodeId>,
}

/// The map of `failure`, given beside `output`, the text the agent was shown of the failed run, in
/// task `task`; none when it would add nothing to the output, or no frame fits its cap.
///
/// # Errors
///
/// The store's errors.
pub fn failure_map(
    queries: Queries<'_>,
    failure: &GroundedFailure,
    (output, task): (&str, Option<Ulid>),
) -> Result<Option<FailureMap>, StoreError> {
    let frames = failure
        .frames
        .iter()
        .filter(|frame| is_plain(frame.path.as_str()) && is_plain(&frame.code))
        .take(MAX_MAPPED_FRAMES);
    let mut text = match failure.test.as_deref().filter(|test| is_plain(test)) {
        Some(test) => {
            let test: String = test.chars().take(MAX_TEST_NAME_CHARS).collect();
            format!("[cortex] The failing test {test} runs through these lines, as they are now:\n")
        }
        None => "[cortex] The failure runs through these lines, as they are now:\n".to_owned(),
    };
    let mut named: Vec<&GroundedFrame> = Vec::new();
    for frame in frames {
        let line = frame_line(frame);
        // INVARIANT: budgeted (a frame that would take the map past its cap is left out, and the
        // frames after it with it)
        if estimate_tokens(&format!("{text}{line}")) > MAX_FAILURE_MAP_TOKENS {
            break;
        }
        text.push_str(&line);
        named.push(frame);
    }
    // INVARIANT: silent-when-unsure (the map is given only when a frame it names shows what the
    // output lacks or is in a file the task has not seen)
    let mut adds = false;
    for frame in &named {
        let is_unseen = match task {
            Some(task) => !queries.has_touched(task, frame.file, EVENTS_LOOKED_AT)?,
            None => true,
        };
        if is_unseen || !is_shown_in(output, &frame.code) {
            adds = true;
            break;
        }
    }
    if !adds {
        return Ok(None);
    }
    let mut files: Vec<NodeId> = Vec::new();
    for frame in named {
        if !files.contains(&frame.file) {
            files.push(frame.file);
        }
    }
    Ok(Some(FailureMap { text, files }))
}

/// One frame's line of the map: `  path:line in name: code`, the name left out when there is none
/// that may be shown, the code cut to [`MAX_CODE_CHARS`].
fn frame_line(frame: &GroundedFrame) -> String {
    let mut line = format!("  {}:{}", frame.path.as_str(), frame.line);
    if let Some((_, name)) = frame
        .symbol
        .as_ref()
        .filter(|(_, name)| is_plain(name.as_str()))
    {
        let _ = write!(line, " in {}", name.as_str());
    }
    let code: String = frame.code.chars().take(MAX_CODE_CHARS).collect();
    let _ = writeln!(line, ": {code}");
    line
}

/// Whether `output` shows the code line `code`: a line too short to tell apart (`}`, `pass`, an
/// empty one) is never taken as shown, since the output holding those characters says nothing.
fn is_shown_in(output: &str, code: &str) -> bool {
    code.chars().count() >= MIN_MATCHED_CODE_CHARS && output.contains(code)
}
