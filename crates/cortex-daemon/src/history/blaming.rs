//! One job carried out: a span blamed at HEAD, the commits it names carded, the commits other than
//! normal ones looked through, and what to write; or a commit read again for its card or its text.
//! Each runs git through the worker's handles and reads the store through its reader; it writes
//! nothing itself. Before each git process it asks whether it may go on, so that a hook request
//! arriving meanwhile, or a stop, is waited for between two of them.
//!
//! A blame is kept under the hash of the file's content as the index read it, the file on disk,
//! and is made only when that file is the file at HEAD: the same bytes, or the same once the
//! disk's CRLF line ends are made LF, as git's end-of-line conversion leaves a checkout. Then a
//! line of HEAD is the same line on disk. A file that is not is not blamed at that HEAD.
//!
//! One job cards at most [`MAX_CARDS_A_JOB`] commits, those owning most lines first; the others
//! are left to card jobs of their own, so that one write stays a few rows, and the span is looked
//! through once more after them (a job of its own, queued behind them), since a commit not carded
//! could not be known as one to look through. That second look leaves no further one.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{ContentHash, is_object_id};
use cortex_git::blame::{Blame, BlameError, BlameRequest, MIN_SINCE_S};
use cortex_index::{is_secret, read_whole};
use cortex_store::{BlameRecord, ChangeCardRecord, Queries, StoreError};

use super::carding::{self, CardRun, CardSettings, NotCarded};
use super::jobs::Jobs;
use super::queue::BlameJob;
use super::repository::Git;
use super::walk::{self, Owner};
use super::write::HistoryWrite;

/// The seconds in a day.
const DAY_S: i64 = 24 * 60 * 60;

/// The most commits one job cards: a span of 500 lines can name hundreds, each up to three git
/// processes; the rest are carded by jobs of their own, one at a time.
pub(super) const MAX_CARDS_A_JOB: usize = 16;

/// What a job is carried out with besides git and the store.
pub(super) struct Context<'a> {
    /// How a blame is run and how commits are carded.
    pub(super) settings: &'a JobSettings,
    /// The time the job runs at, in UTC milliseconds, from the worker's clock.
    pub(super) now_ms: i64,
    /// Whether the job may start its next git process now: false once the worker is told to
    /// stop. It waits while hook requests are served.
    pub(super) may_go_on: &'a dyn Fn() -> bool,
    /// Where the texts of the commits carded go, and what the worker remembers.
    pub(super) jobs: &'a Jobs,
}

/// The settings a job runs with.
#[derive(Debug, Clone)]
pub(super) struct JobSettings {
    /// `history.blame_timeout_ms`: how long one blame may take.
    pub(super) blame_timeout_ms: u64,
    /// `git.max_age_days`: a blame reads no history older than this.
    pub(super) max_age_days: u64,
    /// `index.max_file_bytes`: a larger file is not blamed.
    pub(super) max_file_bytes: u64,
    /// The most commits one job cards, [`MAX_CARDS_A_JOB`].
    pub(super) max_cards: usize,
    /// How commits are carded.
    pub(super) card: CardSettings,
}

/// Why a job ended without a write.
#[derive(Debug)]
pub(super) enum Halt {
    /// The worker was told to stop.
    Stopped,
    /// The span's blame failed: it is not asked of git again.
    Blame(BlameError),
    /// Something else failed, in words that name no text.
    Failed(String),
}

impl From<StoreError> for Halt {
    /// A store that cannot be read ends the job.
    fn from(error: StoreError) -> Self {
        Self::Failed(format!("reading the store: {error}"))
    }
}

/// The write for `job`'s blame at HEAD, read with `git` and `queries`: none when the store keeps
/// that blame already (unless this is the job's second look, `is_again`), the file on disk is no
/// longer the one the job names, or it is not the file at HEAD (remembered, for that HEAD).
///
/// # Errors
///
/// A stop, a failed blame, or a store that cannot be read.
pub(super) fn blame(
    git: &Git<'_>,
    queries: Queries<'_>,
    (job, is_again): (&BlameJob, bool),
    context: &Context<'_>,
) -> Result<Option<HistoryWrite>, Halt> {
    let is_kept = queries
        .cached_blames((&job.path, &job.file_hash), job.first_line, job.last_line)?
        .iter()
        .any(|kept| kept.first_line == job.first_line && kept.last_line == job.last_line);
    if (is_kept && !is_again) || context.jobs.is_unblameable_at(job, git.head) {
        return Ok(None);
    }
    match on_disk_is_head(git, job, context.settings.max_file_bytes)? {
        AtHead::Same => {}
        AtHead::Other => {
            context.jobs.remember_unblameable(job, git.head);
            return Ok(None);
        }
        AtHead::Gone => return Ok(None),
    }
    let mut cards = Cards::default();
    let mut blamed = blame_at(git, job, &[], context)?;
    cards.add(git, queries, &blamed, context)?;
    let ignored = walk::ignored(&blamed, &cards.owners);
    if !ignored.is_empty() {
        // One step past the commits not worth showing: a step that fails leaves the runs to them.
        match blame_at(git, job, &ignored, context) {
            Ok(looked_through) => {
                cards.add(git, queries, &looked_through, context)?;
                blamed = looked_through;
            }
            Err(Halt::Blame(error)) => {
                tracing::debug!(error = %error, "a blame looking through commits failed");
            }
            Err(halt) => return Err(halt),
        }
    }
    if cards.left_for_later && !is_again {
        context.jobs.ask_look_again(job.clone());
    }
    let runs = walk::kept_runs(&blamed, &cards.owners);
    Ok(Some(HistoryWrite {
        cards: cards.made,
        blame: Some(BlameRecord {
            path: job.path.clone(),
            file_hash: job.file_hash,
            first_line: job.first_line,
            last_line: job.last_line,
            runs,
            computed_at_ms: context.now_ms,
        }),
    }))
}

/// The write for the commit `oid` read again: its card when the store keeps none, and its text
/// for the cache either way; none to write when the store keeps its card, the id is not one, or
/// the commit could not be carded before.
///
/// # Errors
///
/// A stop, git that cannot read the commit, or a store that cannot be read.
pub(super) fn card(
    git: &Git<'_>,
    queries: Queries<'_>,
    oid: &str,
    context: &Context<'_>,
) -> Result<Option<HistoryWrite>, Halt> {
    if !is_object_id(oid) || context.jobs.is_uncardable(oid) {
        return Ok(None);
    }
    if !(context.may_go_on)() {
        return Err(Halt::Stopped);
    }
    if !queries.change_cards(&[oid.to_owned()])?.is_empty() {
        // A text that cannot be read (the commit gone, a history rewritten and collected) would be
        // asked for by every prompt that shows its lines: it is remembered, and not asked again.
        let text = carding::text(git, oid, &context.settings.card).map_err(|why| {
            context.jobs.remember_uncardable(oid);
            Halt::Failed(why)
        })?;
        context.jobs.put_text(oid, text);
        return Ok(None);
    }
    let made = match carding::card(git, queries, (oid, false), &card_run(context)) {
        Ok(Some(made)) => made,
        Ok(None) => return Ok(None),
        Err(NotCarded::Stopped) => return Err(Halt::Stopped),
        Err(NotCarded::Failed(why)) => {
            context.jobs.remember_uncardable(oid);
            return Err(Halt::Failed(why));
        }
    };
    context.jobs.put_text(oid, made.text);
    Ok(Some(HistoryWrite {
        cards: vec![(made.record, made.tokens)],
        blame: None,
    }))
}

/// What carding runs with in `context`.
fn card_run<'a>(context: &'a Context<'a>) -> CardRun<'a> {
    CardRun {
        settings: &context.settings.card,
        now_ms: context.now_ms,
        may_go_on: context.may_go_on,
    }
}

/// What the file of a job is on disk against HEAD.
enum AtHead {
    /// The file the job names, and the file at HEAD up to end-of-line conversion.
    Same,
    /// The file the job names, and not the file at HEAD.
    Other,
    /// No longer the file the job names, or not a file Cortex reads: the job is stale.
    Gone,
}

/// What `job`'s file is on disk against HEAD's commit: a regular file of at most
/// `max_file_bytes` whose bytes hash to the job's hash, compared with the file at HEAD.
fn on_disk_is_head(git: &Git<'_>, job: &BlameJob, max_file_bytes: u64) -> Result<AtHead, Halt> {
    if is_secret(&job.path) {
        return Ok(AtHead::Gone);
    }
    let Ok(on_disk) = read_whole(&job.path.to_native(git.root), max_file_bytes) else {
        return Ok(AtHead::Gone);
    };
    if ContentHash::of(&on_disk) != job.file_hash {
        return Ok(AtHead::Gone);
    }
    let committed = git
        .history
        .committed(&job.path, max_file_bytes)
        .map_err(|error| Halt::Failed(format!("reading {} at HEAD: {error}", job.path)))?;
    Ok(match committed {
        // INVARIANT: true-at-render. A blame is kept under the content on disk only when each of
        // its lines is HEAD's line: the same bytes, or the same once CRLF line ends are made LF.
        Some(blob) if is_same_file(&on_disk, &blob) => AtHead::Same,
        _ => AtHead::Other,
    })
}

/// Whether `on_disk` is the committed `blob` as a checkout writes it: the same bytes, or the same
/// once every CRLF of `on_disk` is made LF.
pub(super) fn is_same_file(on_disk: &[u8], blob: &[u8]) -> bool {
    if on_disk == blob {
        return true;
    }
    let mut converted = Vec::with_capacity(on_disk.len());
    let mut bytes = on_disk.iter().peekable();
    while let Some(byte) = bytes.next() {
        if *byte == b'\r' && bytes.peek() == Some(&&b'\n') {
            continue;
        }
        converted.push(*byte);
    }
    converted == blob
}

/// The blame of `job`'s span at HEAD, looking through the commits `ignored`, within the history
/// window and the time the settings give.
fn blame_at(
    git: &Git<'_>,
    job: &BlameJob,
    ignored: &[String],
    context: &Context<'_>,
) -> Result<Blame, Halt> {
    if !(context.may_go_on)() {
        return Err(Halt::Stopped);
    }
    let request = BlameRequest {
        path: &job.path,
        first_line: job.first_line,
        last_line: job.last_line,
        at: git.head,
        since_s: since_s(context.now_ms, context.settings.max_age_days),
        timeout_ms: context.settings.blame_timeout_ms,
        ignore_revs: ignored,
    };
    git.blamer.blame(&request).map_err(Halt::Blame)
}

/// The oldest commit time a blame at `now_ms` reads, `max_age_days` back, as the miner's window
/// has it; none when that is before the times git reads as dates, where no limit reads the same
/// history.
fn since_s(now_ms: i64, max_age_days: u64) -> Option<i64> {
    let max_age_s = i64::try_from(max_age_days)
        .unwrap_or(i64::MAX)
        .saturating_mul(DAY_S);
    let since_s = (now_ms / 1000).saturating_sub(max_age_s);
    (since_s >= MIN_SINCE_S).then_some(since_s)
}

/// The owners a job learned of and the cards it made.
#[derive(Default)]
struct Cards {
    /// Each commit's kind and whether it has a parent, by id, from the store or a card made.
    owners: BTreeMap<String, Owner>,
    /// The cards made, with their terms, in the order made.
    made: Vec<(ChangeCardRecord, Vec<String>)>,
    /// The commits looked at, carded or not.
    seen: BTreeSet<String>,
    /// Whether commits were left to card jobs past the job's cap.
    left_for_later: bool,
}

impl Cards {
    /// Learns the owners of `blame`'s runs not known yet: from the store's cards, and by carding,
    /// the commits owning most lines first, each the store has none of, up to
    /// the job's most cards ([`MAX_CARDS_A_JOB`]); the others are asked for as card jobs. A boundary is carded
    /// only when it is a root. A commit that cannot be carded is logged, remembered and left
    /// unknown, which neither looks through it nor keeps it as a root.
    fn add(
        &mut self,
        git: &Git<'_>,
        queries: Queries<'_>,
        blame: &Blame,
        context: &Context<'_>,
    ) -> Result<(), Halt> {
        let unknown: Vec<String> = by_lines_owned(blame)
            .into_iter()
            .filter(|oid| self.seen.insert(oid.clone()))
            .collect();
        for stored in queries.change_cards(&unknown)? {
            let owner = Owner {
                kind: stored.kind,
                has_parent: stored.first_parent.is_some(),
            };
            self.owners.insert(stored.oid, owner);
        }
        for oid in unknown {
            if self.owners.contains_key(&oid) || context.jobs.is_uncardable(&oid) {
                continue;
            }
            if self.made.len() >= context.settings.max_cards {
                context.jobs.ask_card(&oid);
                self.left_for_later = true;
                continue;
            }
            let is_boundary = blame
                .commits
                .get(&oid)
                .is_some_and(|commit| commit.is_boundary);
            match carding::card(git, queries, (&oid, is_boundary), &card_run(context)) {
                Ok(Some(made)) => {
                    let owner = Owner {
                        kind: made.record.kind,
                        has_parent: made.record.first_parent.is_some(),
                    };
                    self.owners.insert(oid.clone(), owner);
                    context.jobs.put_text(&oid, made.text);
                    self.made.push((made.record, made.tokens));
                }
                Ok(None) => {}
                Err(NotCarded::Stopped) => return Err(Halt::Stopped),
                Err(NotCarded::Failed(why)) => {
                    tracing::warn!(error = %why, "a commit could not be carded");
                    context.jobs.remember_uncardable(&oid);
                }
            }
        }
        Ok(())
    }
}

/// The commits owning `blame`'s runs, those owning most lines first, then by id.
fn by_lines_owned(blame: &Blame) -> Vec<String> {
    let mut lines: BTreeMap<&str, u32> = BTreeMap::new();
    for run in &blame.runs {
        *lines.entry(run.oid.as_str()).or_default() += run.end.saturating_sub(run.start) + 1;
    }
    let mut owners: Vec<(&str, u32)> = lines.into_iter().collect();
    owners.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    owners.into_iter().map(|(oid, _)| oid.to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked by hand: at 2026-01-01T00:00:00Z (1,767,225,600 s) a window of 730 days reaches
    /// back 63,072,000 s, to 1,704,153,600 s; a window of 20,000 days would reach before 1973,
    /// where git reads no dates, so no limit is given.
    #[test]
    fn the_window_reaches_back_its_days_from_the_clock() {
        assert_eq!(since_s(1_767_225_600_000, 730), Some(1_704_153_600));
        assert_eq!(since_s(1_767_225_600_000, 20_000), None);
        assert_eq!(since_s(1_767_225_600_000, u64::MAX), None);
    }

    /// The file on disk is HEAD's when its bytes are the blob's, or are once each CRLF is made LF;
    /// a lone CR stays, and a changed line, or LF on disk against CRLF in the blob, is another
    /// file.
    #[test]
    fn a_checkout_s_line_ends_are_the_same_file() {
        assert!(is_same_file(b"a\nb\n", b"a\nb\n"));
        assert!(is_same_file(b"a\r\nb\r\n", b"a\nb\n"));
        assert!(is_same_file(b"a\r\nb", b"a\nb"));
        assert!(!is_same_file(b"a\rb\n", b"ab\n"));
        assert!(!is_same_file(b"a\r\nc\r\n", b"a\nb\n"));
        assert!(!is_same_file(b"a\nb\n", b"a\r\nb\r\n"));
    }

    /// The commits of a blame come those owning most lines first: `b` owns 3, `a` and `c` one
    /// each, `a` before `c` by id.
    #[test]
    fn commits_owning_most_lines_are_carded_first() {
        let run = |start, end, letter: char| cortex_git::blame::BlameRun {
            start,
            end,
            oid: letter.to_string().repeat(40),
            source_start: start,
        };
        let blame = Blame {
            runs: vec![run(1, 1, 'c'), run(2, 4, 'b'), run(5, 5, 'a')],
            commits: BTreeMap::new(),
        };
        let ids: Vec<String> = ['b', 'a', 'c']
            .iter()
            .map(|letter| letter.to_string().repeat(40))
            .collect();
        assert_eq!(by_lines_owned(&blame), ids);
    }
}
