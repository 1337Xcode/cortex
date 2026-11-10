//! The line of a commit's body that gives a reason: the first one that says something. A blank
//! line, a trailer, a link, a list of file names, a bare reference to an issue, a line git wrote
//! itself (a revert's or a cherry-pick's) and the subject over again say nothing a reader could
//! use.

use super::{MAX_REASON_CHARS, truncate_chars};

/// The keys of the trailers commit messages carry, in lower case, that a trailer's key is known
/// by. A key of any other trailer is recognised by its shape: letters and digits joined by
/// hyphens, with at least one hyphen (`Change-Id`, `X-Reviewed-By`).
const TRAILER_KEYS: &[&str] = &[
    "signed-off-by",
    "co-authored-by",
    "reviewed-by",
    "acked-by",
    "tested-by",
    "reported-by",
    "suggested-by",
    "helped-by",
    "cc",
    "change-id",
    "fixes",
    "closes",
    "resolves",
    "bug",
    "refs",
    "link",
];

/// What starts an item of a bulleted list.
const BULLETS: &[char] = &['-', '*', '+', '\u{2022}'];

/// The words that say a line closes an issue, in lower case.
const ISSUE_KEYWORDS: &[&str] = &[
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

/// What git itself writes at the start of a line of a message: the line a revert adds and the one
/// `git cherry-pick -x` adds.
const GIT_LINES: &[&str] = &["This reverts commit ", "(cherry picked from commit "];

/// The first line of `body`, trimmed to [`MAX_REASON_CHARS`] characters, that says something: not
/// blank, a trailer, a link alone, a bullet that lists only file names, an issue keyword and
/// references alone (`Fixes #12`), a line git wrote, or `subject` again, bullet or not; none when
/// every line is one of those.
pub(super) fn first_line(body: &[&str], subject: &str) -> Option<String> {
    body.iter()
        .map(|line| line.trim())
        .find(|line| says_something(line, subject))
        .map(|line| truncate_chars(line, MAX_REASON_CHARS).to_owned())
}

/// Whether the trimmed `line` of a body says something its `subject` does not.
fn says_something(line: &str, subject: &str) -> bool {
    !line.is_empty()
        && !is_trailer(line)
        && !is_link(line)
        && !lists_files(line)
        && !is_issue_line(line)
        && !GIT_LINES.iter().any(|written| line.starts_with(written))
        && !repeats(line, subject)
}

/// Whether `line`, after a leading bullet, is `subject`.
fn repeats(line: &str, subject: &str) -> bool {
    let text = line.strip_prefix(BULLETS).map_or(line, str::trim_start);
    text == subject
}

/// Whether `line` is an issue keyword and references only: `Fixes #12`, `closes: a/b#3, #4.`
fn is_issue_line(line: &str) -> bool {
    let mut words = line
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|word| !word.is_empty());
    let Some(keyword) = words.next() else {
        return false;
    };
    let keyword = keyword.trim_end_matches(':').to_ascii_lowercase();
    if !ISSUE_KEYWORDS.contains(&keyword.as_str()) {
        return false;
    }
    let references: Vec<&str> = words.filter(|word| *word != "and").collect();
    !references.is_empty()
        && references
            .iter()
            .all(|word| is_reference(word.trim_end_matches(['.', ';'])))
}

/// Whether `word` is an issue reference: `#12`, or `owner/repo#12`.
fn is_reference(word: &str) -> bool {
    let Some((repository, number)) = word.split_once('#') else {
        return false;
    };
    let is_name = |name: &str| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_alphanumeric() || "_.-".contains(character))
    };
    let is_repository = repository.is_empty()
        || repository
            .split_once('/')
            .is_some_and(|(owner, name)| is_name(owner) && is_name(name));
    is_repository && !number.is_empty() && number.chars().all(|digit| digit.is_ascii_digit())
}

/// Whether `line` is a trailer: `Key: value` or `Key:` where the key is known or shaped like one.
fn is_trailer(line: &str) -> bool {
    let Some((key, rest)) = line.split_once(':') else {
        return false;
    };
    let is_key_shaped = !key.is_empty()
        && key.contains('-')
        && key.starts_with(|first: char| first.is_ascii_alphabetic())
        && key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    let is_known = TRAILER_KEYS.contains(&key.to_ascii_lowercase().as_str());
    (is_key_shaped || is_known) && (rest.is_empty() || rest.starts_with([' ', '\t']))
}

/// Whether `text` is one link and nothing else, bare or in angle brackets.
fn is_link(text: &str) -> bool {
    let url = text.trim_matches(['<', '>']);
    let has_scheme = url.starts_with("http://") || url.starts_with("https://");
    has_scheme && !url.contains(char::is_whitespace)
}

/// Whether `line` is a bullet whose every word is a file name.
fn lists_files(line: &str) -> bool {
    let Some(item) = line
        .strip_prefix(BULLETS)
        .and_then(|rest| rest.strip_prefix([' ', '\t']))
    else {
        return false;
    };
    let mut words = item
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|word| !word.is_empty())
        .peekable();
    words.peek().is_some() && words.all(is_file_name)
}

/// Whether `word` reads as a file name: it holds a `/`, or ends in a dot and an extension of at
/// most eight letters and digits with a letter among them, after a name.
fn is_file_name(word: &str) -> bool {
    let word = word.trim_matches(['`', '"', '\'', ';', ':']);
    if word.contains('/') {
        return word.len() > 1;
    }
    let Some((name, extension)) = word.rsplit_once('.') else {
        return false;
    };
    !name.is_empty()
        && (1..=8).contains(&extension.len())
        && extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        && extension
            .chars()
            .any(|character| character.is_ascii_alphabetic())
}
