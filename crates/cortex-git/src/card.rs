//! A commit read as a change card: what a commit's message says about why it changed code, kept as
//! numbers and a kind rather than as text. [`read_card`] takes what `git show -s` prints with
//! [`SHOW_FORMAT`] and says who made the commit (a bot or a person, never which), whether it is a
//! mass change, a trivial one or a normal one, which pull request and issues it names, and the
//! two lines of its message worth showing: the subject and the first line of the body that says
//! something. It reads text and writes nothing; [`CommitReader`] runs the two git commands that
//! give it the text and whether the commit only moved whitespace. The subject and the reason line
//! are held in memory by the caller and read from git again when needed.

mod reason;
mod references;
mod source;
#[cfg(test)]
mod tests;
mod trivial;

pub use source::{CommitError, CommitReader};

use cortex_core::is_object_id;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// The `git show` format that [`read_card`] reads: the commit id, its parents on one line, the
/// committer time in seconds (the time `git log --since` and blame's date limit cut on), the
/// author's name, the committer's name, the subject, then the body.
pub const SHOW_FORMAT: &str = "--format=%H%n%P%n%ct%n%an%n%cn%n%s%n%b";

/// The most characters of a subject a card keeps.
pub const MAX_SUBJECT_CHARS: usize = 72;

/// The most characters of the reason line a card keeps.
pub const MAX_REASON_CHARS: usize = 100;

/// The suffix GitHub gives the name of an account that is a bot.
const BOT_SUFFIX: &str = "[bot]";

/// The trailer of a commit message that says who signed it off.
const SIGNED_OFF_BY: &str = "signed-off-by:";

/// Why the text `git show` printed could not be read as a card.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CardError {
    /// A line that [`SHOW_FORMAT`] prints is not there.
    #[error("the commit text has no {0} line")]
    MissingLine(&'static str),
    /// The commit id or a parent is not a hexadecimal object id of 40 or 64 digits.
    #[error("a commit id in the commit text is not an object id")]
    BadId,
    /// The commit time is not a whole number of seconds.
    #[error("the commit time is not a number of seconds")]
    BadTime,
    /// A bot pattern is not a glob.
    #[error("the bot pattern {0:?} is not a glob")]
    BadBotPattern(String),
    /// One of the patterns the card reads references with does not compile.
    #[error("a reference pattern does not compile: {0}")]
    BadReferencePattern(String),
}

/// How many paths a commit changed, as the history miner knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangedPaths {
    /// The paths the commit changed, counted.
    Count(usize),
    /// The commit changed more paths than the miner reads (`git.max_files_per_commit`), so it
    /// listed none.
    OverMinerLimit,
}

/// What [`read_card`] needs besides the text of the commit.
#[derive(Debug, Clone, Copy)]
pub struct CardContext<'a> {
    /// The paths the commit changed.
    pub changed_paths: ChangedPaths,
    /// The most paths a commit may change and still be a normal change
    /// (`git.max_files_per_commit`, default 30).
    pub max_files: usize,
    /// Whether `git show -w --stat` printed nothing: the commit only moved whitespace.
    pub is_cosmetic: bool,
    /// Globs, compared without case, of the names of accounts that are bots, besides every name
    /// that ends in `[bot]`.
    pub bot_patterns: &'a [String],
}

/// Whether a person or a program made a commit. The name is never kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorKind {
    /// A person, or an account that no pattern says is a bot.
    Human,
    /// An account whose name ends in `[bot]` or matches a bot pattern.
    Bot,
}

/// What kind of change a commit is, which decides whether its message is worth showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// A change whose message may give a reason.
    Normal,
    /// A change whose subject names housekeeping (a typo, formatting, a bump) or that only moved
    /// whitespace.
    Trivial,
    /// A change to more paths than a change that is about one thing touches.
    Mass,
    /// A change made by a bot.
    Bot,
}

/// A commit as a card: numbers, a kind and the two lines of its message worth showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeCard {
    /// The commit's id.
    pub oid: String,
    /// The ids of the commit's parents.
    pub parents: Vec<String>,
    /// When the commit was committed, in seconds since the Unix epoch: the committer time, which
    /// is the one a date limit on git's history cuts on, and which a rebase moves.
    pub time_s: i64,
    /// The first line of the message, at most [`MAX_SUBJECT_CHARS`] characters.
    pub subject: String,
    /// The first line of the body that says something, at most [`MAX_REASON_CHARS`] characters;
    /// none when the body holds only blank lines, trailers, links and file names.
    pub reason: Option<String>,
    /// The pull or merge request the commit came from.
    pub pull_request: Option<u64>,
    /// The issues the message says it closes, fixes or resolves, as written (`#12`,
    /// `owner/repo#12`), without repeats, at most three, in order.
    pub issues: Vec<String>,
    /// Whether the commit was made by a bot.
    pub author: AuthorKind,
    /// The kind of change.
    pub kind: ChangeKind,
}

/// The card for the commit whose `git show -s` text, printed with [`SHOW_FORMAT`], is `show`. Line
/// ends may be `\n` or `\r\n`.
///
/// A commit that is a mass change is [`ChangeKind::Mass`] whoever made it and whatever it says;
/// failing that, one a bot made is [`ChangeKind::Bot`]; failing that, one that only moved
/// whitespace or whose subject names housekeeping (see the `trivial` module) is
/// [`ChangeKind::Trivial`]; every other commit is [`ChangeKind::Normal`].
///
/// The author decides whether a bot made the commit. The committer, and a `Signed-off-by` line,
/// count only when the author's name is empty: a bot that lands or rebases a person's commits
/// must not hide that person's reasons.
///
/// # Errors
///
/// The text lacks one of the format's lines, the commit id, a parent or the time is malformed, a
/// bot pattern is not a glob, or the crate's own reference patterns do not compile.
pub fn read_card(show: &str, context: &CardContext<'_>) -> Result<ChangeCard, CardError> {
    let mut lines = show.lines();
    let mut next = |name: &'static str| lines.next().ok_or(CardError::MissingLine(name));
    let oid = next("id")?.trim();
    let parents_line = next("parents")?;
    let time_text = next("time")?.trim();
    let author_name = next("author")?;
    let committer_name = next("committer")?;
    let full_subject = next("subject")?.trim();
    let body: Vec<&str> = lines.collect();

    if !is_object_id(oid) {
        return Err(CardError::BadId);
    }
    let parents: Vec<String> = parents_line
        .split_whitespace()
        .map(|parent| {
            if is_object_id(parent) {
                Ok(parent.to_owned())
            } else {
                Err(CardError::BadId)
            }
        })
        .collect::<Result<_, _>>()?;
    let time_s = time_text.parse().map_err(|_| CardError::BadTime)?;

    let bots = bot_matcher(context.bot_patterns)?;
    // INVARIANT: no-content-stored. The names decide the author's kind here and are dropped; the
    // card has no field to hold one.
    let is_bot = if author_name.trim().is_empty() {
        is_bot_name(&bots, committer_name) || signs_off_as_bot(&bots, &body)
    } else {
        is_bot_name(&bots, author_name)
    };
    let author = if is_bot {
        AuthorKind::Bot
    } else {
        AuthorKind::Human
    };

    let message = format!("{full_subject}\n{}", body.join("\n"));
    let references = references::find(full_subject, &message)?;
    Ok(ChangeCard {
        oid: oid.to_owned(),
        parents,
        time_s,
        subject: truncate_chars(full_subject, MAX_SUBJECT_CHARS).to_owned(),
        reason: reason::first_line(&body, full_subject),
        pull_request: references.pull_request,
        issues: references.issues,
        author,
        kind: kind_of(full_subject, author, context),
    })
}

/// The kind of the commit with this subject and author: mass first, since the number of paths says
/// the message is not about one span; then bot, which names the cause more exactly than a word of
/// the subject; then trivial; otherwise normal.
fn kind_of(subject: &str, author: AuthorKind, context: &CardContext<'_>) -> ChangeKind {
    let is_mass = match context.changed_paths {
        ChangedPaths::Count(count) => count > context.max_files,
        ChangedPaths::OverMinerLimit => true,
    };
    if is_mass {
        ChangeKind::Mass
    } else if author == AuthorKind::Bot {
        ChangeKind::Bot
    } else if context.is_cosmetic || trivial::is_trivial(subject) {
        ChangeKind::Trivial
    } else {
        ChangeKind::Normal
    }
}

/// The caller's bot patterns as one matcher that ignores case.
///
/// # Errors
///
/// A pattern is not a glob.
fn bot_matcher(patterns: &[String]) -> Result<GlobSet, CardError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .map_err(|_| CardError::BadBotPattern(pattern.clone()))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|_| CardError::BadBotPattern(patterns.join(", ")))
}

/// Whether `name`, an account's name, ends in `[bot]` or matches a bot pattern.
fn is_bot_name(bots: &GlobSet, name: &str) -> bool {
    let name = name.trim();
    name.to_lowercase().ends_with(BOT_SUFFIX) || bots.is_match(name)
}

/// Whether a `Signed-off-by` trailer of `body` names a bot: the name before the address.
fn signs_off_as_bot(bots: &GlobSet, body: &[&str]) -> bool {
    body.iter().any(|line| {
        let line = line.trim();
        let Some(head) = line.get(..SIGNED_OFF_BY.len()) else {
            return false;
        };
        if !head.eq_ignore_ascii_case(SIGNED_OFF_BY) {
            return false;
        }
        let Some(signer) = line.get(SIGNED_OFF_BY.len()..) else {
            return false;
        };
        let name = signer.split('<').next().unwrap_or(signer);
        is_bot_name(bots, name)
    })
}

/// The first `max_chars` characters of `text`, cut on a character boundary, without the white
/// space the cut leaves at its end.
fn truncate_chars(text: &str, max_chars: usize) -> &str {
    let Some((end, _)) = text.char_indices().nth(max_chars) else {
        return text;
    };
    text.get(..end).map_or(text, str::trim_end)
}
