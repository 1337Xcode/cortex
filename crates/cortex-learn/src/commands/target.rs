//! Finding, in a line the shell reader read, the simple command that ran a check, and what stands
//! around it: whether it runs inside a group and whether a pipe feeds it.

use crate::commands::shell::{Command, List, Word};
use crate::commands::{CONFIGURED, Check, argv, runners};

/// A simple command of a line, and where it stands.
#[derive(Debug)]
pub(super) struct Located<'a> {
    /// Its words, the assignments before it included.
    pub(super) words: &'a [Word],
    /// Whether it runs inside parentheses or braces.
    pub(super) is_grouped: bool,
    /// Whether a pipe feeds it: it is not the first command of its pipeline.
    pub(super) is_piped_into: bool,
}

/// The simple command of `list` that ran `check`. When the line exits as `check` failing
/// (`is_failure`), that is the line's last simple command, whose exit the line's is; a check of
/// the user's patterns, which only the patterns tell apart, is taken to be it. Otherwise it is
/// the one simple command that runs `check`: `None` when several do, or none the runners tell.
pub(super) fn locate(list: &List, check: Check, is_failure: bool) -> Option<Located<'_>> {
    let mut found = Vec::new();
    collect(list, false, &mut found);
    if is_failure {
        let final_command = found.pop()?;
        let ran = check_of(final_command.words);
        let is_check = ran == Some(check) || (ran.is_none() && check.runner == CONFIGURED);
        return is_check.then_some(final_command);
    }
    let mut running = found
        .into_iter()
        .filter(|located| check_of(located.words) == Some(check));
    let only = running.next()?;
    running.next().is_none().then_some(only)
}

/// Adds every simple command of `list` to `found`, in the order of the line. The reader caps how
/// deep groups nest, which bounds the recursion.
fn collect<'a>(list: &'a List, is_grouped: bool, found: &mut Vec<Located<'a>>) {
    let pipelines = list.items.iter().flat_map(|and_or| &and_or.pipelines);
    for pipeline in pipelines {
        for (at, command) in pipeline.commands.iter().enumerate() {
            match command {
                Command::Simple(words) => found.push(Located {
                    words,
                    is_grouped,
                    is_piped_into: at > 0,
                }),
                Command::Group { list, .. } => collect(list, true, found),
            }
        }
    }
}

/// The check of a runner Cortex knows that the simple command `words` runs, if any.
fn check_of(words: &[Word]) -> Option<Check> {
    argv::invocation(words).and_then(|invocation| runners::check(&invocation))
}
