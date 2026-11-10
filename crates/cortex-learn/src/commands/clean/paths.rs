//! Paths written from the top of a machine in a kept command's arguments: absolute
//! (`/home/dev/repo/tests/test_ops.py`, `C:\Users\dev\repo\tests`, `C:/Users/dev/repo/tests`),
//! from a home folder (`~/repo/tests`) or on another machine (`\\server\share\repo`). A folder
//! above the repository can hold a user's name, so no such path is kept as written. One inside the
//! repository is kept as the way to it from the folder the command ran in, `/` between names
//! (`tests/test_ops.py`, `../tests`, `.`), which runs the same; any other makes the command
//! uncertain, and so does one in a folder the line does not tell.
//!
//! Such a path is rewritten where an argument starts, after the first `=` of an argument
//! (`--rootdir=/x`, `ROOT=/x`) and after a one-letter option glued to it (`-c/x`), up to a test
//! id after `::`. A path starting anywhere else in a word (after a comma, a colon, a quote or a
//! second `=`, or in a test id) cannot be rewritten without guessing where it ends, and a `file:`
//! URL holds one, so either makes the command uncertain. A path counts as starting after the
//! word's start or a byte that separates values (one of [`SEPARATORS`] or a space), so the `/` of
//! `tests/test_ops.py` or of a subtest filter (`'^TestX$/^y$'`) starts none, and nothing in a
//! URL's text after its `://` is read for one (a single letter before `://` is a drive, not a
//! URL's scheme). A drive with no separator after it (`C:..\x`, `C:tests`) is read from that
//! drive's own working folder, which the line does not tell, so it is never placed.
//!
//! A relative path is kept as written, but one whose `..` climbs above the repository's top, read
//! from the folder the command ran in (`../../dev/repo/tests` from the top), names folders outside
//! it, so it makes the command uncertain, as does any `..` read from a folder the line does not
//! tell. A `..` that stays inside (`../tests` from `calc`) is kept.
//!
//! The repository's top is compared as the platform's file systems compare names: on Windows,
//! where a command line may spell a folder in other letters' case than the agent did, in any case.

use std::path::Path;

use cortex_core::RepoPath;

use super::Unsure;
use super::detectors::run_end;
use super::test_path::{TEST_ID_MARK, is_path_byte};
use super::urls;
use crate::commands::folder::{self, Folder};
use crate::commands::shell::{List, Word};

/// The bytes after which a path may start in a word, besides its start and spaces.
const SEPARATORS: &[u8] = b"=:,;\"'([{<>|@";

/// What starts a URL that names a file on the machine.
const FILE_URL: &str = "file:/";

/// Where the paths of one command are placed: the repository's top, and the repository folder
/// the command ran in.
pub(super) struct Places<'a> {
    /// The repository's top folder.
    root: &'a Path,
    /// The folder the command ran in, `None` when the line does not tell it or it lies outside
    /// the repository.
    ran_in: Option<RepoPath>,
}

impl<'a> Places<'a> {
    /// The places of the simple command `target` of `list`, for a line started in `cwd` of the
    /// repository whose top is `root`: the folder it ran in is `cwd` moved by each `cd` before it.
    pub(super) fn new(list: &List, target: &[Word], root: &'a Path, cwd: &Path) -> Self {
        let mut folder = Folder::new(cwd);
        let ran_in = folder::walk(list, target, &mut folder)
            .then(|| folder.path().and_then(|path| in_repository(root, path)))
            .flatten();
        Self { root, ran_in }
    }

    /// The way from the folder the command ran in to `written`, a path from a machine's top, `/`
    /// between names; `None` when it lies outside the repository, cannot be placed on this
    /// platform, or the way holds anything but letters, digits and `_-./`.
    fn way_to(&self, written: &str) -> Option<String> {
        let path = Path::new(written);
        if written.starts_with('~') || !path.is_absolute() {
            return None;
        }
        let target = in_repository(self.root, &folder::normalized(path)?)?;
        let way = way_between(self.ran_in.as_ref()?, &target);
        way.bytes().all(is_path_byte).then_some(way)
    }

    /// Whether the relative `path`, read from the folder the command ran in, climbs above the
    /// repository's top: a `..` with no name left to take off. A path with a `..` read from a
    /// folder the line does not tell might, so it counts as climbing.
    fn is_climbing(&self, path: &str) -> bool {
        let parts: Vec<&str> = path.split(['/', '\\']).collect();
        if !parts.contains(&"..") {
            return false;
        }
        let Some(ran_in) = &self.ran_in else {
            return true;
        };
        let mut depth = names(ran_in).len();
        for part in parts {
            match part {
                ".." => match depth.checked_sub(1) {
                    Some(up) => depth = up,
                    None => return true,
                },
                "" | "." => {}
                _ => depth += 1,
            }
        }
        false
    }
}

/// `words` with each path written from a machine's top rewritten as the way to it from the folder
/// the command ran in. A word the shell does not pass as written is left as it is.
///
/// # Errors
///
/// [`Unsure`] when a word holds such a path outside the repository or not placed, or one where
/// it cannot be rewritten.
pub(super) fn relativized(words: Vec<Word>, places: &Places<'_>) -> Result<Vec<Word>, Unsure> {
    words
        .into_iter()
        .map(|word| match word.literal() {
            Some(text) => Ok(Word {
                text: rewritten(text, places)?,
                is_literal: true,
            }),
            None => Ok(word),
        })
        .collect()
}

/// `text` with the path from a machine's top it holds rewritten, or as it is when it holds none.
fn rewritten(text: &str, places: &Places<'_>) -> Result<String, Unsure> {
    if text.to_ascii_lowercase().contains(FILE_URL) || climbs_out(text, places) {
        return Err(Unsure);
    }
    let starts = path_starts(text);
    let Some(&start) = starts.first() else {
        return Ok(text.to_owned());
    };
    let end = text
        .get(start..)
        .and_then(|rest| rest.find(TEST_ID_MARK))
        .map_or(text.len(), |at| start + at);
    if !is_rewritable_at(text, start) || starts.iter().any(|&at| at >= end) {
        return Err(Unsure);
    }
    let way = places
        .way_to(text.get(start..end).ok_or(Unsure)?)
        .ok_or(Unsure)?;
    let before = text.get(..start).ok_or(Unsure)?;
    let after = text.get(end..).ok_or(Unsure)?;
    Ok(format!("{before}{way}{after}"))
}

/// Where in `text`, outside its URLs, a value may start: at the word's start, after a separator,
/// or after a one-letter option glued to it.
fn value_starts(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let in_urls = urls::spans(text);
    (0..bytes.len())
        .filter(|at| !in_urls.iter().any(|span| span.contains(at)))
        .filter(|&at| {
            let before = at.checked_sub(1).and_then(|before| bytes.get(before));
            before.is_none_or(|&byte| is_separator(byte)) || is_after_short_option(bytes, at)
        })
        .collect()
}

/// Where in `text` a path from a machine's top starts: at each place a value may start that
/// [`is_path_start`] says begins one.
fn path_starts(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    value_starts(text)
        .into_iter()
        .filter(|&at| bytes.get(at..).is_some_and(is_path_start))
        .collect()
}

/// Whether a path from a machine's top starts `rest`: `/`, `\` or `~`, or a drive (`C:\`, `C:/`,
/// and `C:` alone or before a name, which Windows reads from that drive's own working folder),
/// but not a single letter before `::`, a path's separator from its test id.
fn is_path_start(rest: &[u8]) -> bool {
    match rest {
        [b'/' | b'\\' | b'~', ..] => true,
        [drive, b':', after @ ..] => drive.is_ascii_alphabetic() && after.first() != Some(&b':'),
        _ => false,
    }
}

/// Whether a relative path in `text` climbs above the repository's top by its `..`, read from
/// the folder the command ran in: each value in it that is not a path from a machine's top, up
/// to the separator after it.
fn climbs_out(text: &str, places: &Places<'_>) -> bool {
    let bytes = text.as_bytes();
    value_starts(text)
        .into_iter()
        .filter(|&at| !bytes.get(at..).is_some_and(is_path_start))
        .any(|at| {
            let end = run_end(bytes, at, |byte| !is_separator(byte));
            places.is_climbing(text.get(at..end).unwrap_or_default())
        })
}

/// Whether `byte` separates one value from the next in a word: a space or one of [`SEPARATORS`].
fn is_separator(byte: u8) -> bool {
    byte.is_ascii_whitespace() || SEPARATORS.contains(&byte)
}

/// Whether byte `at` of `bytes` follows a one-letter option glued to its value, as `-c/x`.
fn is_after_short_option(bytes: &[u8], at: usize) -> bool {
    at == 2 && matches!(bytes, [b'-', letter, ..] if letter.is_ascii_alphabetic())
}

/// Whether a path starting at byte `at` of `text` can be rewritten: it starts the word, the
/// value after the word's first `=`, or the value of a one-letter option glued to it.
fn is_rewritable_at(text: &str, at: usize) -> bool {
    at == 0
        || text.find('=').is_some_and(|equals| equals + 1 == at)
        || is_after_short_option(text.as_bytes(), at)
}

/// The way from the repository folder `from` to the repository path `to`, `/` between names:
/// a `..` for each name of `from` not on the way to `to`, then the rest of `to`; `.` when they
/// are the same place.
fn way_between(from: &RepoPath, to: &RepoPath) -> String {
    let from_names: Vec<&str> = names(from);
    let to_names: Vec<&str> = names(to);
    let shared = from_names
        .iter()
        .zip(&to_names)
        .take_while(|(from_name, to_name)| from_name == to_name)
        .count();
    let mut way = vec![".."; from_names.len().saturating_sub(shared)];
    way.extend(to_names.get(shared..).unwrap_or_default());
    if way.is_empty() {
        ".".to_owned()
    } else {
        way.join("/")
    }
}

/// The names of `path`, top first; none for the repository's top.
fn names(path: &RepoPath) -> Vec<&str> {
    path.as_str()
        .split('/')
        .filter(|name| !name.is_empty())
        .collect()
}

/// The repository path of the absolute `path`, without `.` or `..`, when it lies inside `root`,
/// the root written in any letters' case on Windows.
fn in_repository(root: &Path, path: &Path) -> Option<RepoPath> {
    RepoPath::from_native_in_any_case(root, path)
}
