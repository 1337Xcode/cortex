//! Why-lines: under a span's code, the reason git history gives for how those lines look, read
//! from the commits that last changed them. The daemon keeps who last changed which lines of a
//! file's content (its blame cache) and a card of each such commit in the store, and the commits'
//! subjects and reason lines in memory only; this module reads the store for the spans a work order
//! lists ([`look_up`]), asks the daemon for what it does not have yet ([`WhyHistory`], never
//! waiting), and the work order writes the lines from what was looked up
//! (`crate::render::render_with`), as [`choose`] decides them.
//!
//! A blame is looked up by the file's path and its content as the index read it, and a span's code
//! is shown only while the file on disk is that content (`crate::render`): the line numbers are
//! those of the code shown, and a reason is the commit that last changed those lines in that
//! content. A blame not kept yet, or a commit whose text is not held, gives no line this time; the
//! daemon is asked for it, so that a later prompt finds it, at most [`MAX_TEXT_ASKS`] texts a
//! retrieval.

mod choose;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use cortex_core::{ContentHash, LineRange, RepoPath};
use cortex_store::{BlameRecord, CardKind, Queries, StoreError};

use crate::render::Item;
use crate::spans::MAX_SPANS_PER_FILE;

pub use choose::MAX_REASON_TOKENS;

/// The most commit texts one retrieval asks the daemon for when they are not held: each is a job
/// of the daemon's history worker, whose queue a few prompts share, and a text missed now is
/// asked for again by the next prompt that needs it.
pub const MAX_TEXT_ASKS: usize = 4;
pub(crate) use choose::{choose, day_of, quoted, references};

/// The two lines of a commit's message worth showing, as the daemon holds them: untrusted text
/// from the repository, written into a work order only as quoted data with its control characters
/// taken out.
#[derive(Clone, PartialEq, Eq)]
pub struct CommitText {
    /// The subject, at most 72 characters.
    pub subject: String,
    /// The first line of the body that gives a reason, at most 100 characters, if one does.
    pub reason: Option<String>,
}

impl fmt::Debug for CommitText {
    /// Says nothing of the text, which a person wrote, so that it cannot reach a log this way.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("CommitText").finish_non_exhaustive()
    }
}

/// What the daemon holds besides the store: the commits' texts, and where to ask for a text or a
/// blame not held yet. Each answers at once; none runs git.
pub trait WhyHistory {
    /// The subject and reason line of the commit `oid`, when held.
    fn commit_text(&self, oid: &str) -> Option<CommitText>;

    /// Asks for the subject and reason line of the commit `oid`, for a later prompt.
    fn ask_for_text(&self, oid: &str);

    /// Asks for the blame of `lines` of the file at `path`, whose content at HEAD hashes to
    /// `file_hash`, for a later prompt.
    fn ask_for_blame(&self, path: &RepoPath, file_hash: ContentHash, lines: LineRange);
}

/// A retrieval's why-lines: what holds the texts and the caps (`history.max_reasons`,
/// `history.max_tokens`).
#[derive(Clone, Copy)]
pub struct WhyAsk<'a> {
    /// The texts and the blames asked for.
    pub history: &'a dyn WhyHistory,
    /// The most reasons a span gets, from 1 to 5.
    pub max_reasons: u32,
    /// The most tokens of reasons a span gets.
    pub max_tokens: u32,
}

impl fmt::Debug for WhyAsk<'_> {
    /// The caps; the history has nothing to show.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WhyAsk")
            .field("max_reasons", &self.max_reasons)
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

/// The blame of a span as kept, made from the content the index read: the lines of the span each
/// commit last changed, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlamedSpan {
    /// Each run of the span's lines, with the commit that last changed them.
    pub runs: Vec<(LineRange, String)>,
}

/// A commit that changed lines of a span, as a why-line reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasonCommit {
    /// When it was committed, in UTC seconds.
    pub committed_at_s: i64,
    /// What kind of change its card says it is.
    pub kind: CardKind,
    /// The pull or merge request it came from.
    pub pull_request: Option<u64>,
    /// The issues its message says it closes, fixes or resolves, as written.
    pub issues: Vec<String>,
    /// Whether its message has a reason line.
    pub has_reason: bool,
    /// The path of the first test file it changed that the index still holds, if any.
    pub test: Option<RepoPath>,
    /// Its subject and reason line, when the daemon holds them.
    pub text: Option<CommitText>,
}

/// What a work order's why-lines are written from: each span's blame, by its item's position and
/// its own, the commits those blames name, and the caps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WhyLines {
    /// The blame of each span that has one kept, by item and span position.
    pub spans: BTreeMap<(usize, usize), BlamedSpan>,
    /// The commits the blames name that could be a reason, by id.
    pub commits: BTreeMap<String, ReasonCommit>,
    /// The most reasons a span gets.
    pub max_reasons: u32,
    /// The most tokens of reasons a span gets.
    pub max_tokens: u32,
}

/// What `queries` keeps of the history of `items`' spans: each span's blame made from its file's
/// content as the index read it, when one covers the span, its runs cut to the span, and the cards
/// of the commits owning its lines that could be a reason, with their texts from `ask`'s history;
/// the first [`MAX_TEXT_ASKS`] texts not held are asked for.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn look_up(
    queries: Queries<'_>,
    items: &[Item],
    ask: &WhyAsk<'_>,
) -> Result<WhyLines, StoreError> {
    let mut lines = WhyLines {
        max_reasons: ask.max_reasons,
        max_tokens: ask.max_tokens,
        ..WhyLines::default()
    };
    for (at, item) in items.iter().enumerate() {
        let Some(content) = item.content else {
            continue;
        };
        for (span_at, span) in item.spans.iter().enumerate().take(MAX_SPANS_PER_FILE) {
            let kept = queries.cached_blames(
                (&item.path, &content),
                span.lines.start_line(),
                span.lines.end_line(),
            )?;
            if let Some(blamed) = covering(&kept, span.lines) {
                lines.spans.insert((at, span_at), blamed);
            }
        }
    }
    let oids: Vec<String> = lines
        .spans
        .values()
        .flat_map(|blamed| blamed.runs.iter().map(|(_, oid)| oid.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut asked = 0;
    for card in queries.change_cards(&oids)? {
        let test_file = match card.tests.first() {
            Some(node) => queries.node_file(*node)?.map(|(_, path)| path),
            None => None,
        };
        let could_be = card.kind == CardKind::Normal
            && (card.has_reason
                || card.pull_request.is_some()
                || !card.issues.is_empty()
                || test_file.is_some());
        if !could_be {
            continue;
        }
        let text = ask.history.commit_text(&card.oid);
        if text.is_none() && asked < MAX_TEXT_ASKS {
            ask.history.ask_for_text(&card.oid);
            asked += 1;
        }
        let commit = ReasonCommit {
            committed_at_s: card.committed_at_s,
            kind: card.kind,
            pull_request: card.pull_request,
            issues: card.issues,
            has_reason: card.has_reason,
            test: test_file,
            text,
        };
        lines.commits.insert(card.oid, commit);
    }
    Ok(lines)
}

/// Asks `ask`'s history for the blame of each span shown, `shown` as the rendered work order gives
/// them, that `lines` has none of, for a later prompt.
pub(crate) fn ask_missing(
    ask: &WhyAsk<'_>,
    items: &[Item],
    shown: &[(usize, usize, LineRange)],
    lines: &WhyLines,
) {
    for (at, span_at, _) in shown {
        if lines.spans.contains_key(&(*at, *span_at)) {
            continue;
        }
        let Some(item) = items.get(*at) else {
            continue;
        };
        let (Some(content), Some(span)) = (item.content, item.spans.get(*span_at)) else {
            continue;
        };
        ask.history.ask_for_blame(&item.path, content, span.lines);
    }
}

/// Of the blames `kept`, the narrowest that covers every line of `span`, the first by its first
/// line among equals; its runs that meet the span, cut to it, as lines and commits.
pub(crate) fn covering(kept: &[BlameRecord], span: LineRange) -> Option<BlamedSpan> {
    let record = kept
        .iter()
        .filter(|record| {
            record.first_line <= span.start_line() && record.last_line >= span.end_line()
        })
        .min_by_key(|record| (record.last_line - record.first_line, record.first_line))?;
    let runs = record
        .runs
        .iter()
        .filter_map(|run| {
            let start = run.start.max(span.start_line());
            let end = run.end.min(span.end_line());
            Some((LineRange::new(start, end).ok()?, run.oid.clone()))
        })
        .collect();
    Some(BlamedSpan { runs })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use cortex_core::RepoId;
    use cortex_store::{BlameRunRecord, ChangeCardRecord, Writer, WriterOptions};

    use super::*;
    use crate::reasons::Reason;
    use crate::render::ItemSpan;

    /// A daemon's history that holds no text, counting the texts and blames asked for.
    #[derive(Default)]
    struct Counting {
        /// The texts asked for, in order.
        texts: RefCell<Vec<String>>,
        /// How many blames were asked for.
        blames: Cell<usize>,
    }

    impl WhyHistory for Counting {
        fn commit_text(&self, _oid: &str) -> Option<CommitText> {
            None
        }

        fn ask_for_text(&self, oid: &str) {
            self.texts.borrow_mut().push(oid.to_owned());
        }

        fn ask_for_blame(&self, _path: &RepoPath, _file_hash: ContentHash, _lines: LineRange) {
            self.blames.set(self.blames.get() + 1);
        }
    }

    /// The commit numbered `number`, an id of 40 digits.
    fn oid(number: u32) -> String {
        format!("{number:040}")
    }

    /// A store holding the blame of lines 1 to 500 of `a.py`, one run of 50 lines for each of ten
    /// commits (1 to 50 the first's, 51 to 100 the second's, ...), and a normal card with a reason
    /// line for each.
    fn store(
        root: &std::path::Path,
        file_hash: ContentHash,
    ) -> Result<Writer, Box<dyn std::error::Error>> {
        let mut writer = Writer::open(&WriterOptions {
            folder: &root.join(".cortex"),
            repo_id: &RepoId::of(root),
            mmap_bytes: 0,
            now_ms: 1,
        })?;
        let mut transaction = writer.transaction()?;
        let runs = (0..10)
            .map(|at| BlameRunRecord {
                start: at * 50 + 1,
                end: at * 50 + 50,
                source_start: 1,
                oid: oid(at),
            })
            .collect();
        transaction.put_blame(&BlameRecord {
            path: RepoPath::new("a.py")?,
            file_hash,
            first_line: 1,
            last_line: 500,
            runs,
            computed_at_ms: 1,
        })?;
        for at in 0..10 {
            transaction.put_change_card(
                &ChangeCardRecord {
                    oid: oid(at),
                    first_parent: None,
                    committed_at_s: 1,
                    kind: CardKind::Normal,
                    pull_request: None,
                    issues: Vec::new(),
                    has_reason: true,
                    files: Vec::new(),
                    symbols: Vec::new(),
                    tests: Vec::new(),
                    carded_at_ms: 1,
                },
                &[],
            )?;
        }
        transaction.commit()?;
        Ok(writer)
    }

    /// `a.py` with one span of `lines`.
    fn item(
        file_hash: ContentHash,
        (first, last): (u32, u32),
    ) -> Result<Item, Box<dyn std::error::Error>> {
        Ok(Item {
            path: RepoPath::new("a.py")?,
            content: Some(file_hash),
            spans: vec![ItemSpan {
                lines: LineRange::new(first, last)?,
                name: None,
                scope: None,
            }],
            reason: Reason::EditedHere,
        })
    }

    /// A span of lines 60 to 70 meets the second commit's run alone: only its card is looked at,
    /// its run cut to the span, and only its text is asked for. A span of lines 1 to 500 meets all
    /// ten, and asks for the first four texts only (`MAX_TEXT_ASKS`).
    #[test]
    fn a_span_reads_only_the_commits_of_its_lines_and_asks_for_few_texts()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let file_hash = ContentHash::of(b"a");
        let writer = store(root.path(), file_hash)?;
        let counting = Counting::default();
        let ask = WhyAsk {
            history: &counting,
            max_reasons: 2,
            max_tokens: 120,
        };
        let small = look_up(writer.queries(), &[item(file_hash, (60, 70))?], &ask)?;
        let runs: Vec<_> = small
            .spans
            .values()
            .map(|blamed| blamed.runs.clone())
            .collect();
        assert_eq!(runs, [vec![(LineRange::new(60, 70)?, oid(1))]]);
        assert_eq!(small.commits.keys().cloned().collect::<Vec<_>>(), [oid(1)]);
        assert_eq!(*counting.texts.borrow(), [oid(1)]);

        let all = Counting::default();
        let ask = WhyAsk {
            history: &all,
            ..ask
        };
        let whole = look_up(writer.queries(), &[item(file_hash, (1, 500))?], &ask)?;
        assert_eq!(whole.commits.len(), 10);
        assert_eq!(all.texts.borrow().len(), MAX_TEXT_ASKS);
        assert_eq!(MAX_TEXT_ASKS, 4);
        Ok(())
    }
}
