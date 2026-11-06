//! What git leaves out of a repository of what Cortex writes there, and the lines its `.gitignore`
//! gets when git does not leave it out yet: `.cortex/` (or, when the team shares facts, all of
//! `.cortex/` but `.cortex/shared/`, which is committed), and `.claude/settings.local.json` when
//! init makes that file. Git says what it ignores (`git check-ignore`), whichever file makes it;
//! lines are only ever added, at the end, after a comment naming Cortex, by which `cortex
//! uninstall` finds them again.

use std::path::Path;

use anyhow::bail;
use cortex_core::{
    IGNORE_ALL_BUT_SHARED, IGNORE_BLOCK_HEADER as HEADER, IGNORE_CORTEX_FOLDER,
    IGNORE_LOCAL_SETTINGS as LOCAL_SETTINGS,
};

use super::git;

/// A file in Cortex's folder, which need not exist: git ignores it when it ignores the folder.
const CORTEX_PROBE: &str = ".cortex/probe";

/// A file in the folder Cortex shares with the team.
const SHARED_PROBE: &str = ".cortex/shared/probe";

/// What git ignores of what Cortex writes in a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ignored {
    /// Whether it ignores Cortex's folder.
    pub(super) is_cortex: bool,
    /// Whether it ignores the folder shared with the team.
    pub(super) is_shared: bool,
    /// Whether it ignores Claude Code's local settings.
    pub(super) is_local_settings: bool,
}

impl Ignored {
    /// Whether the team shares facts, as the ignores tell: Cortex's folder left out but for the
    /// shared one. `None` while Cortex's folder is not left out, as before anyone answered.
    pub(super) fn sharing(self) -> Option<bool> {
        self.is_cortex.then_some(!self.is_shared)
    }
}

/// What git ignores in the repository at `root`, git run as [`git::run`] runs it, outside
/// `bound`.
///
/// # Errors
///
/// Git did not run, or ended with an error.
pub(super) fn read(root: &Path, bound: &Path) -> anyhow::Result<Ignored> {
    let args = [
        "check-ignore",
        "--no-index",
        CORTEX_PROBE,
        SHARED_PROBE,
        LOCAL_SETTINGS,
    ];
    let captured = git::run(&args, root, bound).map_err(anyhow::Error::msg)?;
    // check-ignore ends with 0 when it ignores any of the paths, 1 when it ignores none, and more
    // on an error.
    if captured.status.code().is_none_or(|code| code > 1) {
        bail!(
            "git could not say what it ignores in {} (it ended with {})",
            root.display(),
            captured.status
        );
    }
    let answer = String::from_utf8_lossy(&captured.stdout);
    let is_listed = |path: &str| answer.lines().any(|line| line == path);
    Ok(Ignored {
        is_cortex: is_listed(CORTEX_PROBE),
        is_shared: is_listed(SHARED_PROBE),
        is_local_settings: is_listed(LOCAL_SETTINGS),
    })
}

/// The lines `.gitignore` needs, from what git `ignored`, whether the team shares facts
/// (`is_sharing`), and whether init makes Claude Code's local settings (`is_settings_made`);
/// none when git ignores what it should already.
pub(super) fn needed(
    ignored: Ignored,
    is_sharing: bool,
    is_settings_made: bool,
) -> Vec<&'static str> {
    let mut lines = Vec::new();
    if !ignored.is_cortex {
        if is_sharing {
            lines.extend(IGNORE_ALL_BUT_SHARED);
        } else {
            lines.push(IGNORE_CORTEX_FOLDER);
        }
    }
    if is_settings_made && !ignored.is_local_settings {
        lines.push(LOCAL_SETTINGS);
    }
    lines
}

/// The `.gitignore` that held `before` (or none) with `lines` added at its end, after a comment
/// naming Cortex, in the file's own line ending.
pub(super) fn with_lines(before: Option<&str>, lines: &[&str]) -> String {
    let before = before.unwrap_or_default();
    let ending = if before.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut text = before.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str(ending);
    }
    text.push_str(HEADER);
    text.push_str(ending);
    for line in lines {
        text.push_str(line);
        text.push_str(ending);
    }
    text
}

/// `line` without its line ending, `\n` or `\r\n`.
fn bare(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

/// The `.gitignore` `text` with the block init added taken out: the last [`HEADER`] line followed
/// by exactly `lines`, in order, each in either line ending; `None` when it holds no such block,
/// as when the user changed it. Every other line stays as it is, a block of the same lines a team
/// committed before init included.
pub(crate) fn without_lines(text: &str, lines: &[String]) -> Option<String> {
    let all: Vec<&str> = text.split_inclusive('\n').collect();
    let block = 1 + lines.len();
    let start = (0..all.len()).rev().find(|&at| {
        all.get(at..at + block).is_some_and(|found| {
            found.first().map(|line| bare(line)) == Some(HEADER)
                && found
                    .iter()
                    .skip(1)
                    .map(|line| bare(line))
                    .eq(lines.iter().map(String::as_str))
        })
    })?;
    let mut kept = String::with_capacity(text.len());
    for (at, line) in all.iter().enumerate() {
        if !(start..start + block).contains(&at) {
            kept.push_str(line);
        }
    }
    Some(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What git ignores, its fields in order: Cortex's folder, the shared folder, the settings.
    fn ignored(is_cortex: bool, is_shared: bool, is_local_settings: bool) -> Ignored {
        Ignored {
            is_cortex,
            is_shared,
            is_local_settings,
        }
    }

    /// Nothing ignored: Cortex's folder (all of it, or all but the shared folder) and the settings
    /// init makes are added; the settings only when init makes them. Cortex's folder ignored: only
    /// the settings, when not ignored already.
    #[test]
    fn only_what_git_does_not_ignore_is_added() {
        let none = ignored(false, false, false);
        assert_eq!(
            needed(none, false, true),
            [".cortex/", ".claude/settings.local.json"]
        );
        assert_eq!(needed(none, true, false), [".cortex/*", "!.cortex/shared/"]);
        let all = ignored(true, true, false);
        assert_eq!(needed(all, false, true), [".claude/settings.local.json"]);
        assert_eq!(
            needed(ignored(true, true, true), false, true),
            Vec::<&str>::new()
        );
        assert_eq!(all.sharing(), Some(false));
        assert_eq!(ignored(true, false, false).sharing(), Some(true));
        assert_eq!(none.sharing(), None);
    }

    /// Lines go at the end after a comment, on a line of their own even when the file did not end
    /// with one, in the file's own line ending; a file that did not exist is made of them.
    #[test]
    fn lines_go_at_the_end_in_the_file_s_ending() {
        assert_eq!(
            with_lines(Some("target/"), &[".cortex/"]),
            "target/\n# Cortex\n.cortex/\n"
        );
        assert_eq!(
            with_lines(Some("target/\r\n"), &[".cortex/"]),
            "target/\r\n# Cortex\r\n.cortex/\r\n"
        );
        assert_eq!(
            with_lines(None, &[".cortex/*", "!.cortex/shared/"]),
            "# Cortex\n.cortex/*\n!.cortex/shared/\n"
        );
    }

    /// The block init added comes out exactly, the comment and its lines, in either line ending,
    /// the last such block when there are more: a block from before init, as a team commits one,
    /// stays, and so do the user's lines around it. A file whose block was changed, or that holds
    /// none, gives nothing to write.
    #[test]
    fn the_block_init_added_comes_out() {
        let lines = [".cortex/".to_owned(), LOCAL_SETTINGS.to_owned()];
        let added = "target/\n# Cortex\n.cortex/\n.claude/settings.local.json\ndist/\n";
        assert_eq!(
            without_lines(added, &lines).as_deref(),
            Some("target/\ndist/\n")
        );
        let windows = "target/\r\n# Cortex\r\n.cortex/\r\n.claude/settings.local.json\r\n";
        assert_eq!(
            without_lines(windows, &lines).as_deref(),
            Some("target/\r\n")
        );
        let settings = [LOCAL_SETTINGS.to_owned()];
        let team = "# Cortex\n.cortex/\n# Cortex\n.claude/settings.local.json\n";
        assert_eq!(
            without_lines(team, &settings).as_deref(),
            Some("# Cortex\n.cortex/\n")
        );
        let folder = [".cortex/".to_owned()];
        let twice = "# Cortex\n.cortex/\nnotes/\n# Cortex\n.cortex/\n";
        assert_eq!(
            without_lines(twice, &folder).as_deref(),
            Some("# Cortex\n.cortex/\nnotes/\n")
        );
        assert_eq!(
            without_lines("# Cortex\n.cortex/", &folder).as_deref(),
            Some("")
        );
        assert_eq!(without_lines("# Cortex\n.cortex/\n", &lines), None);
        assert_eq!(without_lines("target/\n", &lines), None);
    }
}
