//! How a runner was launched: the harmless variables before it, the words that run it for the
//! agent (`python -m`, `uv run`, `npx`, `pnpm exec`) and the arguments after it. A launcher is
//! kept only when it is one of the known shapes and every word of it is plain, so that writing it
//! into another command line is safe and runs the same program.

use crate::commands::argv::{self, is_python};
use crate::commands::shell::Word;

/// The variables a line may set before the runner and the narrow command still carries: they only
/// say how a run looks, never what it reaches.
const HARMLESS_VARIABLES: &[&str] = &[
    "CI",
    "FORCE_COLOR",
    "NO_COLOR",
    "PYTHONDONTWRITEBYTECODE",
    "RUST_BACKTRACE",
];

/// The programs that run the program their arguments name after `run`.
const RUN_WRAPPERS: &[&str] = &["uv", "poetry", "pipenv", "pdm", "hatch", "rye"];

/// The words, before the runner's own, that launch it as an agent writes them.
const PACKAGE_LAUNCHERS: &[&[&str]] = &[
    &["npx"],
    &["bunx"],
    &["pnpx"],
    &["npm", "exec"],
    &["npm", "exec", "--"],
    &["pnpm", "exec"],
    &["yarn", "exec"],
    &["pnpm"],
    &["yarn"],
];

/// A runner's launch, read from a simple command's words.
#[derive(Debug)]
pub(super) struct Launch {
    /// The variable assignments, as written.
    pub(super) env: Vec<String>,
    /// The launcher words and the runner's own word, as written.
    pub(super) words: Vec<String>,
    /// The words after the runner's.
    pub(super) args: Vec<String>,
}

impl Launch {
    /// The launch of one of `programs` in the simple command `words`. `None` when a word on the
    /// way is not literal or plain, a variable is not harmless, or the words before the runner are
    /// not a launcher this module knows.
    pub(super) fn read(words: &[Word], programs: &[&str]) -> Option<Self> {
        let rest = argv::after_assignments(words);
        let assigned = words.len().checked_sub(rest.len())?;
        let env = words
            .get(..assigned)?
            .iter()
            .map(|word| harmless(&word.text))
            .collect::<Option<Vec<String>>>()?;
        let at = rest.iter().position(|word| {
            word.literal()
                .is_some_and(|text| programs.contains(&argv::program_name(text).as_str()))
        })?;
        let before = plain_texts(rest.get(..at)?)?;
        if !is_known_launcher(&before) {
            return None;
        }
        let runner = plain_texts(rest.get(at..=at)?)?;
        let args = rest
            .get(at + 1..)?
            .iter()
            .map(|word| word.literal().map(str::to_owned))
            .collect::<Option<Vec<String>>>()?;
        let mut launcher = before;
        launcher.extend(runner);
        Some(Self {
            env,
            words: launcher,
            args,
        })
    }
}

/// `assignment` when it sets a harmless variable to a short plain value.
fn harmless(assignment: &str) -> Option<String> {
    let (name, value) = assignment.split_once('=')?;
    let is_value = !value.is_empty()
        && value.len() <= 16
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_');
    (HARMLESS_VARIABLES.contains(&name) && is_value).then(|| assignment.to_owned())
}

/// The texts of `words`, when each is literal and plain.
fn plain_texts(words: &[Word]) -> Option<Vec<String>> {
    words
        .iter()
        .map(|word| {
            word.literal()
                .filter(|text| is_plain(text))
                .map(str::to_owned)
        })
        .collect()
}

/// Whether `text` is safe to write into a command line unquoted and runs the same program from
/// anywhere in the repository: letters, digits and `_./+=-`, not an absolute path.
fn is_plain(text: &str) -> bool {
    let is_plain_byte =
        |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'+' | b'=' | b'-');
    !text.is_empty() && !text.starts_with('/') && text.bytes().all(is_plain_byte)
}

/// Whether `before`, the words ahead of the runner's, are a launcher: nothing, a package
/// launcher, or a Python module launch, behind a wrapper's `run` or not.
fn is_known_launcher(before: &[String]) -> bool {
    if PACKAGE_LAUNCHERS
        .iter()
        .any(|shape| shape.iter().copied().eq(before.iter().map(String::as_str)))
    {
        return true;
    }
    let texts: Vec<&str> = before.iter().map(String::as_str).collect();
    let after_wrapper = match texts.as_slice() {
        [wrapper, "run", rest @ ..] if RUN_WRAPPERS.contains(wrapper) => rest,
        all => all,
    };
    match after_wrapper {
        [] => true,
        [python, "-m"] => is_python(&argv::program_name(python)),
        _ => false,
    }
}
