//! Git restores: the commands that put files back as git holds them, with the paths they name.
//! `git restore` (unless it changes only the index, with `--staged` and no `--worktree`),
//! `git checkout -- <paths>` and `git checkout <commit> -- <paths>` restore the paths named;
//! `git reset --hard` restores everything. A `git checkout` without `--` is not taken for one: its
//! word may name a branch. `-p` restores only parts of files, and is not taken either. Git's own
//! options before the command are passed over, `-C` moving the folder the paths are named from; a
//! command naming another repository (`--git-dir`, `--work-tree`) is not taken.

use std::collections::BTreeSet;
use std::path::Path;

use cortex_core::RepoPath;

use super::Restore;
use super::folder::{Folder, Named};
use super::shell::Word;
use super::tail;

/// Git's options before its command that take a value as the next word.
const GIT_VALUE_OPTIONS: &[&str] = &["-c", "--namespace", "--config-env"];

/// Git's options that name another repository, or another folder as its top.
const OTHER_REPOSITORY: &[&str] = &["--git-dir", "--work-tree"];

/// The restore `args`, the words after `git`, run from `folder` in the repository whose top is
/// `root`, if they are one: `None` for any other git command, and for a restore none of whose paths
/// can be resolved inside the repository.
pub(super) fn restore(args: &[Word], folder: &Folder, root: &Path) -> Option<Restore> {
    let mut folder = folder.clone();
    let mut at = 0;
    while let Some(word) = args.get(at) {
        let text = word.literal()?;
        if !text.starts_with('-') {
            break;
        }
        let option = text.split_once('=').map_or(text, |(name, _)| name);
        if OTHER_REPOSITORY.contains(&option) {
            return None;
        }
        if text == "-C" {
            folder = Folder::new(&folder.path_of(args.get(at + 1)?)?);
            at += 2;
        } else {
            at += if GIT_VALUE_OPTIONS.contains(&text) {
                2
            } else {
                1
            };
        }
    }
    let rest = args.get(at + 1..)?;
    let paths = match args.get(at)?.literal()? {
        "restore" => restore_paths(rest)?,
        "checkout" => checkout_paths(rest)?,
        "reset" => {
            return rest
                .iter()
                .any(|word| word.literal() == Some("--hard"))
                .then_some(Restore::Everything);
        }
        _ => return None,
    };
    restored(&paths, &folder, root)
}

/// The paths `git restore` with `args` puts back in the work tree; `None` when it only changes the
/// index, restores parts of files, or reads its paths from a file.
fn restore_paths(args: &[Word]) -> Option<Vec<&Word>> {
    let (mut is_staged, mut is_worktree) = (false, false);
    let mut paths = Vec::new();
    let mut at = 0;
    while let Some(word) = args.get(at) {
        at += 1;
        let Some(text) = word.literal() else {
            paths.push(word);
            continue;
        };
        match text {
            "--" => {
                paths.extend(tail(args, at));
                break;
            }
            "-s" | "--source" => at += 1,
            "-S" | "--staged" => is_staged = true,
            "-W" | "--worktree" => is_worktree = true,
            "-p" | "--patch" => return None,
            _ if text.starts_with("--pathspec-from-file") => return None,
            _ if text.starts_with("--") => {}
            _ if text.starts_with('-') && text.len() > 1 => {
                // Short options together, such as `-SW`; `s` takes the rest as its value.
                for flag in text[1..].chars() {
                    match flag {
                        'S' => is_staged = true,
                        'W' => is_worktree = true,
                        'p' => return None,
                        's' => break,
                        _ => {}
                    }
                }
            }
            _ => paths.push(word),
        }
    }
    (!is_staged || is_worktree).then_some(paths)
}

/// The paths `git checkout` with `args` restores: those after `--`, with at most one commit
/// before it; `None` without `--`, for parts of files, and for a branch made or a HEAD detached.
fn checkout_paths(args: &[Word]) -> Option<Vec<&Word>> {
    let separator = args.iter().position(|word| word.literal() == Some("--"))?;
    let mut commits = 0;
    for word in args.get(..separator)? {
        let text = word.literal()?;
        match text {
            "-p" | "--patch" | "-b" | "-B" | "--orphan" | "--detach" => return None,
            _ if text.starts_with("--pathspec-from-file") => return None,
            _ if text.starts_with('-') => {}
            _ => commits += 1,
        }
    }
    (commits <= 1).then(|| tail(args, separator + 1).iter().collect())
}

/// What restoring `paths` from `folder` restores in the repository whose top is `root`: everything
/// when a path names the top, else the paths resolved inside it, folders standing for what they
/// hold; `None` when none is.
fn restored(paths: &[&Word], folder: &Folder, root: &Path) -> Option<Restore> {
    let mut resolved: BTreeSet<RepoPath> = BTreeSet::new();
    for word in paths {
        match folder.named(word, root) {
            Some(Named::Everything) => return Some(Restore::Everything),
            Some(Named::Path(path)) => {
                resolved.insert(path);
            }
            None => {}
        }
    }
    (!resolved.is_empty()).then(|| Restore::Paths(resolved.into_iter().collect()))
}
