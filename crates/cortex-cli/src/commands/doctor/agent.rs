//! The check on Claude Code's version: at least the oldest Cortex's hooks work with; any newer
//! one works, as Cortex ignores what a newer version adds to its hooks. Claude Code is asked with `claude --version`: the program
//! the user's `PATH` names, as a shell finds it (on Windows `claude.exe`, the native installer's,
//! or `claude.cmd`, npm's), but never one inside the repository, run in its own folder for at most
//! 5 s.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use cortex_core::ipc::CortexVersion;
use cortex_core::one_line;
use cortex_hook::Versions;

use super::{Context, Finding};

/// How long Claude Code has to say its version.
const ASK_WITHIN: Duration = Duration::from_secs(5);

/// The most of Claude Code's answer read: a version is a few dozen bytes.
const MAX_ANSWER_BYTES: u64 = 4_096;

/// The most of a program's path shown, past any real installation's.
const MAX_PATH_CHARS: usize = 300;

/// The names Claude Code's program may have on this platform, in the order tried in each folder.
const PROGRAMS: &[&str] = if cfg!(windows) {
    &["claude.exe", "claude.cmd"]
} else {
    &["claude"]
};

/// Claude Code is a version Cortex's hooks work with: the check in doctor's table.
pub(super) fn check(context: &Context) -> Vec<Finding> {
    cortex_hook::adapter("claude")
        .and_then(|adapter| adapter.versions())
        .and_then(|versions| version(&context.root, versions))
        .into_iter()
        .collect()
}

/// What the version of Claude Code, the program the `PATH` names outside the repository at `root`,
/// says against the `versions` Cortex knows: one older than the oldest fails, any other passes.
/// Nothing is said when no Claude Code is found or it gives no version Cortex can read (it may run
/// from an editor, outside the `PATH`): nothing is known to be wrong. `cortex init` refuses on
/// the failure.
pub(crate) fn version(root: &Path, versions: Versions) -> Option<Finding> {
    let program = cortex_core::find_program(PROGRAMS, root)?;
    let shown = one_line(&program.display().to_string(), MAX_PATH_CHARS);
    classified(&asked(&program).ok()?, &shown, versions)
}

/// What `program --version` printed, run in the program's own folder so that nothing in the
/// current one is in its way; or why it printed nothing.
fn asked(program: &Path) -> Result<String, String> {
    let mut command = Command::new(program);
    command.arg("--version");
    if let Some(folder) = program.parent() {
        command.current_dir(folder);
    }
    let captured = cortex_core::capture(&mut command, ASK_WITHIN, MAX_ANSWER_BYTES)
        .map_err(|error| error.to_string())?;
    if captured.status.success() {
        Ok(String::from_utf8_lossy(&captured.stdout).into_owned())
    } else {
        Err(format!("it ended with {}", captured.status))
    }
}

/// What the `answer` to `--version` of Claude Code's `program` says of it against the `versions`
/// Cortex knows: `2.1.312 (Claude Code)` reads as 2.1.312. Claude Code's versions are semantic
/// versions, ordered as Cortex's own are, and shown as Cortex reads them, so that nothing else
/// the program printed reaches the terminal. None when the answer holds no version.
fn classified(answer: &str, program: &str, versions: Versions) -> Option<Finding> {
    let first = answer.split_whitespace().next().unwrap_or_default();
    let (Ok(found), Ok(oldest)) = (
        CortexVersion::parse(first),
        CortexVersion::parse(versions.oldest),
    ) else {
        return None;
    };
    Some(if found < oldest {
        Finding::Fail {
            what: format!(
                "Claude Code {found} at {program} is older than {oldest}, the oldest Cortex's \
                 hooks work with"
            ),
            fix: format!("update Claude Code to {oldest} or later"),
        }
    } else {
        Finding::Pass(format!("Claude Code {found} at {program}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The versions the tests know.
    const VERSIONS: Versions = Versions {
        oldest: "2.1.295",
        newest_tested: "2.1.312",
    };

    /// Where the tests' Claude Code is.
    const AT: &str = "/usr/local/bin/claude";

    /// One older than the oldest fails with an update as the fix; the oldest and every later one,
    /// newer than the newest tested included, passes; build metadata, terminal control sequences
    /// in it included, is read past and never shown; an answer that is no version says nothing.
    #[test]
    fn versions_are_judged_against_the_known_ones() {
        assert_eq!(
            classified("2.1.294 (Claude Code)\n", AT, VERSIONS),
            Some(Finding::Fail {
                what: "Claude Code 2.1.294 at /usr/local/bin/claude is older than 2.1.295, the \
                       oldest Cortex's hooks work with"
                    .to_owned(),
                fix: "update Claude Code to 2.1.295 or later".to_owned(),
            })
        );
        for (answer, version) in [
            ("2.1.295 (Claude Code)", "2.1.295"),
            ("2.1.312 (Claude Code)\r\n", "2.1.312"),
            ("2.1.300+\u{1b}]0;title\u{7} (Claude Code)", "2.1.300"),
            ("2.2.0 (Claude Code)", "2.2.0"),
        ] {
            assert_eq!(
                classified(answer, AT, VERSIONS),
                Some(Finding::Pass(format!(
                    "Claude Code {version} at /usr/local/bin/claude"
                )))
            );
        }
        assert_eq!(classified("unknown\nversion", AT, VERSIONS), None);
    }
}
