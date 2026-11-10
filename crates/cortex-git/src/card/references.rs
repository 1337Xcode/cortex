//! The pull or merge request a commit message names, and the issues it says it closes, as
//! numbers and references. The patterns are the forms GitHub, GitLab and the people who write
//! commit messages use; text that matches none gives nothing.

use std::sync::LazyLock;

use regex::Regex;

use super::CardError;

/// The most issue references a card keeps.
const MAX_ISSUES: usize = 3;

/// The longest issue reference, in characters, a card keeps; a longer one is not a reference.
const MAX_REFERENCE_CHARS: usize = 100;

/// A pull request number at the end of a subject, as GitHub's squash merge writes it: `(#123)`.
const SQUASH_PATTERN: &str = r"\(#(\d+)\)$";

/// A GitHub merge commit's subject: `Merge pull request #123 from ...`.
const MERGE_PATTERN: &str = r"^Merge pull request #(\d+)";

/// The line GitLab adds to a merge commit's message: `See merge request group/project!123`.
const GITLAB_PATTERN: &str = r"See merge request [\w./-]+!(\d+)";

/// A merge request number at the end of a subject: `(!123)`.
const BANG_PATTERN: &str = r"\(!(\d+)\)$";

/// An issue a message says it closes, fixes or resolves, with the keywords GitHub and GitLab
/// act on, in any case, whole words: the reference is `#12` or `owner/repo#12`.
const ISSUE_PATTERN: &str =
    r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?):?\s+((?:[\w.-]+/[\w.-]+)?#\d+)";

/// The patterns, compiled once.
struct Patterns {
    /// [`SQUASH_PATTERN`].
    squash: Regex,
    /// [`MERGE_PATTERN`].
    merge: Regex,
    /// [`GITLAB_PATTERN`].
    gitlab: Regex,
    /// [`BANG_PATTERN`].
    bang: Regex,
    /// [`ISSUE_PATTERN`].
    issue: Regex,
}

impl Patterns {
    /// Compiles every pattern.
    fn compile() -> Result<Self, regex::Error> {
        Ok(Self {
            squash: Regex::new(SQUASH_PATTERN)?,
            merge: Regex::new(MERGE_PATTERN)?,
            gitlab: Regex::new(GITLAB_PATTERN)?,
            bang: Regex::new(BANG_PATTERN)?,
            issue: Regex::new(ISSUE_PATTERN)?,
        })
    }
}

/// The patterns, or why one did not compile.
static PATTERNS: LazyLock<Result<Patterns, regex::Error>> = LazyLock::new(Patterns::compile);

/// What a message names.
pub(super) struct References {
    /// The pull or merge request number.
    pub(super) pull_request: Option<u64>,
    /// The issues closed, as written, without repeats.
    pub(super) issues: Vec<String>,
}

/// The pull request and the issues a commit names: `subject` is its first line, trimmed, and
/// `message` the whole message, subject first.
///
/// The pull request is the first of these that matches: `(#N)` ending the subject, a subject
/// that begins `Merge pull request #N`, `See merge request group/project!N` anywhere in the
/// message, `(!N)` ending the subject. A number too long for 64 bits matches nothing.
///
/// # Errors
///
/// One of the patterns does not compile.
pub(super) fn find(subject: &str, message: &str) -> Result<References, CardError> {
    let patterns = PATTERNS
        .as_ref()
        .map_err(|error| CardError::BadReferencePattern(error.to_string()))?;
    let number = |pattern: &Regex, text: &str| -> Option<u64> {
        pattern
            .captures(text)
            .and_then(|found| found.get(1))
            .and_then(|digits| digits.as_str().parse().ok())
    };
    let pull_request = number(&patterns.squash, subject)
        .or_else(|| number(&patterns.merge, subject))
        .or_else(|| number(&patterns.gitlab, message))
        .or_else(|| number(&patterns.bang, subject));

    let mut issues: Vec<String> = Vec::new();
    for found in patterns.issue.captures_iter(message) {
        let Some(reference) = found.get(1).map(|reference| reference.as_str()) else {
            continue;
        };
        if issues.len() == MAX_ISSUES {
            break;
        }
        let is_new = !issues.iter().any(|seen| seen == reference);
        if is_new && reference.chars().count() <= MAX_REFERENCE_CHARS {
            issues.push(reference.to_owned());
        }
    }
    Ok(References {
        pull_request,
        issues,
    })
}
