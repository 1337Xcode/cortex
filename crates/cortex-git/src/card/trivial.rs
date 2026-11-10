//! Whether a commit's subject names housekeeping: a typo, formatting, a lint warning, a version
//! bump, a release, a merge of a branch. A word counts only where the subject begins with it
//! (after the `type(scope)!:` prefix of a conventional commit, which is read first), so that a
//! fix that mentions a format, a lock to release or a lint rule is not housekeeping.

/// One word, or kind of word, of a phrase.
enum Part {
    /// This word.
    Word(&'static str),
    /// One of these words.
    OneOf(&'static [&'static str]),
    /// A version: a word that is a number or `v` and a number, such as `1` or `v2`, which is what
    /// the first word of `1.4.0` or `v2.0.1` is once the subject is cut into words.
    Version,
}

/// The conventional commit types that make a commit trivial, in lower case.
const TRIVIAL_TYPES: &[&str] = &["style", "wip", "release", "bump"];

/// The conventional commit scopes that make a commit trivial, in lower case.
const TRIVIAL_SCOPES: &[&str] = &["release", "deps"];

/// The words typos are called.
const TYPOS: &[&str] = &["typo", "typos"];

/// The words formatting is called.
const FORMATS: &[&str] = &["fmt", "format", "formatting", "reformat", "reformatted"];

/// The words a linter is called.
const LINTS: &[&str] = &["lint", "linting", "linter"];

/// The formatters whose run is housekeeping.
const FORMATTERS: &[&str] = &["rustfmt", "black", "prettier", "gofmt"];

/// The phrases that make a commit trivial when the subject begins with one, whole words, without
/// regard to case or punctuation between them. A revert of a revert is `revert revert`, since
/// git writes the subject of a revert as `Revert "<subject>"`.
const PHRASES: &[&[Part]] = &[
    &[Part::Word("wip")],
    &[Part::OneOf(TYPOS)],
    &[Part::Word("fix"), Part::OneOf(TYPOS)],
    &[Part::OneOf(FORMATS)],
    &[Part::Word("fix"), Part::Word("formatting")],
    &[Part::OneOf(LINTS)],
    &[
        Part::Word("fix"),
        Part::OneOf(&["lint", "linter"]),
        Part::Word("warnings"),
    ],
    &[Part::Word("whitespace")],
    &[Part::Word("fix"), Part::Word("whitespace")],
    &[Part::Word("run"), Part::OneOf(FORMATTERS)],
    &[Part::Word("bump")],
    &[Part::Word("release"), Part::Version],
    &[Part::Word("merge"), Part::Word("branch")],
    &[
        Part::Word("merge"),
        Part::Word("remote"),
        Part::Word("tracking"),
        Part::Word("branch"),
    ],
    &[Part::Word("merge"), Part::Word("tag")],
    &[Part::Word("revert"), Part::Word("revert")],
];

/// Whether the subject names housekeeping: its conventional commit type is one of
/// [`TRIVIAL_TYPES`] or its scope one of [`TRIVIAL_SCOPES`], or what follows the prefix begins
/// with one of [`PHRASES`].
pub(super) fn is_trivial(subject: &str) -> bool {
    let subject = subject.trim();
    let Some(prefix) = split_prefix(subject) else {
        return begins_with_phrase(subject);
    };
    let is_type = TRIVIAL_TYPES.contains(&prefix.kind.to_ascii_lowercase().as_str());
    let is_scope = prefix
        .scope
        .is_some_and(|scope| TRIVIAL_SCOPES.contains(&scope.to_ascii_lowercase().as_str()));
    is_type || is_scope || begins_with_phrase(prefix.rest)
}

/// A conventional commit's prefix, `type(scope)!:`, and what follows it.
struct Prefix<'a> {
    /// The type, letters only.
    kind: &'a str,
    /// The scope in the parentheses, when there is one.
    scope: Option<&'a str>,
    /// The subject after the colon, without the white space that follows it.
    rest: &'a str,
}

/// The prefix of `subject` when it begins as a conventional commit does: letters, an optional
/// scope in parentheses, an optional `!`, a colon.
fn split_prefix(subject: &str) -> Option<Prefix<'_>> {
    let (head, rest) = subject.split_once(':')?;
    let head = head.strip_suffix('!').unwrap_or(head);
    let (kind, scope) = match head.split_once('(') {
        Some((kind, scope)) => (kind, Some(scope.strip_suffix(')')?)),
        None => (head, None),
    };
    let is_kind = !kind.is_empty() && kind.chars().all(|letter| letter.is_ascii_alphabetic());
    is_kind.then_some(Prefix {
        kind,
        scope,
        rest: rest.trim_start(),
    })
}

/// Whether the words of `text`, in lower case, begin with the words of one of [`PHRASES`].
fn begins_with_phrase(text: &str) -> bool {
    let words: Vec<String> = text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    PHRASES.iter().any(|phrase| {
        phrase.len() <= words.len()
            && phrase
                .iter()
                .zip(&words)
                .all(|(part, word)| part.matches(word))
    })
}

impl Part {
    /// Whether the lower-case `word` is this part.
    fn matches(&self, word: &str) -> bool {
        match self {
            Self::Word(wanted) => word == *wanted,
            Self::OneOf(wanted) => wanted.contains(&word),
            Self::Version => {
                let digits = word.strip_prefix('v').unwrap_or(word);
                !digits.is_empty() && digits.chars().all(|digit| digit.is_ascii_digit())
            }
        }
    }
}
