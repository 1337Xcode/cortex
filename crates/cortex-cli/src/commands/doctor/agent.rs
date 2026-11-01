//! The check on Claude Code's version: at least the oldest Cortex's hooks work with, and best one
//! whose payloads Cortex's tests hold. Claude Code is asked with `claude --version`: the program
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
        .map(|adapter| version(&context.root, adapter.versions()))
        .into_iter()
        .collect()
}

/// What the version of Claude Code, the program the `PATH` names outside the repository at `root`,
/// says against the `versions` Cortex knows: one older than the oldest fails, and one newer than
/// the newest tested, none found and none said are notes. `cortex init` refuses on the failure.
pub(crate) fn version(root: &Path, versions: Versions) -> Finding {
    let Some(program) = crate::program::on_path(PROGRAMS, root) else {
        return Finding::Note(
            "Claude Code is not on the PATH, so its version cannot be asked".to_owned(),
        );
    };
    let shown = one_line(&program.display().to_string(), MAX_PATH_CHARS);
    match asked(&program) {
        Ok(answer) => classified(&answer, &shown, versions),
        Err(why) => Finding::Note(format!("{shown} did not say its version: {why}")),
    }
}

/// What `program --version` printed, run in the program's own folder so that nothing in the
/// current one is in its way; or why it printed nothing.
fn asked(program: &Path) -> Result<String, String> {
    let mut command = Command::new(program);
    command.arg("--version");
    if let Some(folder) = program.parent() {
        command.current_dir(folder);
    }
    let captured = cortex_daemon::capture(&mut command, ASK_WITHIN, MAX_ANSWER_BYTES)
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
/// the program printed reaches the terminal.
fn classified(answer: &str, program: &str, versions: Versions) -> Finding {
    let first = answer.split_whitespace().next().unwrap_or_default();
    let read = (
        CortexVersion::parse(first),
        CortexVersion::parse(versions.oldest),
        CortexVersion::parse(versions.newest_tested),
    );
    let (Ok(found), Ok(oldest), Ok(newest)) = read else {
        return Finding::Note(format!(
            "{program} gave its version as \"{}\", which Cortex cannot read",
            one_line(answer.trim(), 80)
        ));
    };
    if found < oldest {
        Finding::Fail {
            what: format!(
                "Claude Code {found} at {program} is older than {oldest}, the oldest Cortex's \
                 hooks work with"
            ),
            fix: format!("update Claude Code to {oldest} or later"),
        }
    } else if found > newest {
        Finding::Note(format!(
            "Claude Code {found} at {program} is newer than {newest}, the newest Cortex was \
             tested with; its hooks may send what Cortex does not read yet"
        ))
    } else {
        Finding::Pass(format!(
            "Claude Code {found} at {program}, a version Cortex works with"
        ))
    }
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

    /// One older than the oldest fails with an update as the fix; one between the two, either one
    /// included, passes; a newer one is a note; build metadata, terminal control sequences in it
    /// included, is read past and never shown; one that is no version is a note quoting it, cut
    /// and on one line.
    #[test]
    fn versions_are_judged_against_the_known_ones() {
        assert_eq!(
            classified("2.1.294 (Claude Code)\n", AT, VERSIONS),
            Finding::Fail {
                what: "Claude Code 2.1.294 at /usr/local/bin/claude is older than 2.1.295, the \
                       oldest Cortex's hooks work with"
                    .to_owned(),
                fix: "update Claude Code to 2.1.295 or later".to_owned(),
            }
        );
        for (answer, version) in [
            ("2.1.295 (Claude Code)", "2.1.295"),
            ("2.1.312 (Claude Code)\r\n", "2.1.312"),
            ("2.1.300+\u{1b}]0;title\u{7} (Claude Code)", "2.1.300"),
        ] {
            assert_eq!(
                classified(answer, AT, VERSIONS),
                Finding::Pass(format!(
                    "Claude Code {version} at /usr/local/bin/claude, a version Cortex works with"
                ))
            );
        }
        assert_eq!(
            classified("2.2.0 (Claude Code)", AT, VERSIONS),
            Finding::Note(
                "Claude Code 2.2.0 at /usr/local/bin/claude is newer than 2.1.312, the newest \
                 Cortex was tested with; its hooks may send what Cortex does not read yet"
                    .to_owned()
            )
        );
        assert_eq!(
            classified("unknown\nversion", AT, VERSIONS),
            Finding::Note(
                "/usr/local/bin/claude gave its version as \"unknown version\", which Cortex \
                 cannot read"
                    .to_owned()
            )
        );
    }
}
