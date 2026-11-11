//! The origin note: after an edit, where the lines it replaced came from, when that is a normal
//! commit that names an issue or touched a test. Told once a task, as one line that states the
//! origin and asks nothing:
//! `The lines this edit replaced in src/money.py came from commit "Round half-even for tax"
//! (fixes #612; test: tests/test_money.py), 2025-09-14.`
//!
//! The note gives no line numbers: the lines it speaks of are no longer in the file on disk, so
//! no number of theirs could be checked there when the note is written. It says "Some of the
//! lines" when the commit last changed only part of them.
//!
//! The edit's event says which file it changed and, from the agent's own report of the edit, the
//! hash of the whole file before it and the lines each hunk replaced, numbered in that file. Those
//! lines are looked up in the blame the history worker kept of that very content, by the file's
//! path and that hash: a blame of other content (the file as the index read it after a formatter,
//! a shell command or another session changed it) is never used. The blame stays kept after the
//! index reads the edited file, until upkeep drops blames of content no file has, so the note does
//! not hang on the edit's event coming before the watcher's index. Only what the daemon already
//! holds is read: that blame, the commits' cards, and the commit's text held in memory. Nothing is
//! blamed for an edit, and a text not held is asked for, for a later edit, and gives no note now.
//! An edit of several hunks that removed lines, one that only added lines, one whose report gives
//! no hash of the file before it (a file the edit created, or an agent that does not report it),
//! or one of content never blamed gets no note.

use std::collections::BTreeMap;
use std::fmt;

use cortex_core::{
    ContentHash, LineRange, RepoPath, WarningKind, WarningsConfig, estimate_tokens,
    holds_reminder_tag, is_plain,
};
use cortex_store::{CardKind, Queries, StoreError};

use super::{EditFacts, TaskSoFar, Warning};
use crate::why_lines::{
    CommitText, MAX_REASON_TOKENS, ReasonCommit, WhyHistory, covering, day_of, quoted, references,
};

/// What an edit's origin is looked up with: where the commits' texts are held, and the edit as
/// the agent reported it.
#[derive(Clone, Copy)]
pub struct OriginQuery<'a> {
    /// The commits' texts the daemon holds, and where a text not held is asked for.
    pub history: &'a dyn WhyHistory,
    /// The lines each hunk of the edit replaced, numbered in the file before it.
    pub replaced: &'a [LineRange],
    /// The hash of the whole file before the edit.
    pub before_hash: ContentHash,
}

impl fmt::Debug for OriginQuery<'_> {
    /// The edit; the history has nothing to show.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OriginQuery")
            .field("replaced", &self.replaced)
            .field("before_hash", &self.before_hash)
            .finish_non_exhaustive()
    }
}

/// Where an edit's lines came from: of the commits that last changed them, the eligible one that
/// changed the most of them, and which of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// The lines the edit replaced that the commit last changed, numbered as before the edit.
    pub lines: Vec<LineRange>,
    /// How many lines the edit replaced.
    pub replaced: u32,
    /// The commit, its text held.
    pub commit: ReasonCommit,
}

/// The origin of the edit `query` describes of the file at `path`, as `queries` and the history
/// hold it: of the commits that last changed the lines it replaced, in the blame kept of the file
/// as it was before the edit, the normal one that names an issue or touched a test the index still
/// holds and owns the most of them (the newer of two that own as many, then the lower id). None
/// when the edit did not replace the lines of exactly one hunk, no blame of them is kept, or no
/// commit of them is eligible; the text of the commit chosen is asked for when not held, and none
/// is given this time.
///
/// # Errors
///
/// The store's errors.
pub fn edit_origin(
    queries: Queries<'_>,
    path: &RepoPath,
    query: &OriginQuery<'_>,
) -> Result<Option<Origin>, StoreError> {
    let [replaced] = query.replaced else {
        return Ok(None);
    };
    let kept = queries.cached_blames(
        (path, &query.before_hash),
        replaced.start_line(),
        replaced.end_line(),
    )?;
    let Some(blamed) = covering(&kept, *replaced) else {
        return Ok(None);
    };
    let mut owned: BTreeMap<String, Vec<LineRange>> = BTreeMap::new();
    for (lines, oid) in blamed.runs {
        owned.entry(oid).or_default().push(lines);
    }
    let oids: Vec<String> = owned.keys().cloned().collect();
    let mut best: Option<(u32, ReasonCommit, String)> = None;
    for card in queries.change_cards(&oids)? {
        let test = match card.tests.first() {
            Some(test) => queries.node_file(*test)?.map(|(_, path)| path),
            None => None,
        };
        // INVARIANT: silent-when-unsure (only a normal commit that names an issue or touched a test
        // is an origin worth telling; any other says nothing)
        if card.kind != CardKind::Normal || (card.issues.is_empty() && test.is_none()) {
            continue;
        }
        let count = owned.get(&card.oid).map_or(0, |lines| line_count(lines));
        let commit = ReasonCommit {
            committed_at_s: card.committed_at_s,
            kind: card.kind,
            pull_request: card.pull_request,
            issues: card.issues,
            has_reason: card.has_reason,
            test,
            text: None,
        };
        if is_ahead(&(count, &commit, &card.oid), best.as_ref()) {
            best = Some((count, commit, card.oid));
        }
    }
    let Some((_, mut commit, oid)) = best else {
        return Ok(None);
    };
    let Some(text) = query.history.commit_text(&oid) else {
        query.history.ask_for_text(&oid);
        return Ok(None);
    };
    commit.text = Some(text);
    Ok(Some(Origin {
        lines: owned.remove(&oid).unwrap_or_default(),
        replaced: replaced.end_line() - replaced.start_line() + 1,
        commit,
    }))
}

/// Whether `candidate` (lines owned, commit, id) goes before `held`: more lines, then the newer,
/// then the lower id.
fn is_ahead(
    candidate: &(u32, &ReasonCommit, &String),
    held: Option<&(u32, ReasonCommit, String)>,
) -> bool {
    let Some((count, commit, oid)) = held else {
        return true;
    };
    let (new_count, new_commit, new_oid) = candidate;
    new_count
        .cmp(count)
        .then(new_commit.committed_at_s.cmp(&commit.committed_at_s))
        .then(oid.cmp(new_oid))
        .is_gt()
}

/// How many lines `lines` hold together.
fn line_count(lines: &[LineRange]) -> u32 {
    lines
        .iter()
        .map(|lines| lines.end_line() - lines.start_line() + 1)
        .sum()
}

/// The origin note of `facts`' edit, once a task: none when the task was given one already, the
/// edit has no origin worth telling, or its line cannot be written plainly within its cap.
pub(super) fn warnings(facts: &EditFacts, task: &TaskSoFar, _: &WarningsConfig) -> Vec<Warning> {
    let was_told = task
        .warned
        .iter()
        .any(|(kind, _, _)| *kind == WarningKind::Origin);
    let Some(origin) = facts.origin.as_ref().filter(|_| !was_told) else {
        return Vec::new();
    };
    let Some(line) = line_of(facts.edited.path.as_str(), origin) else {
        return Vec::new();
    };
    let edited = facts.edited.node;
    vec![Warning {
        kind: WarningKind::Origin,
        source: edited,
        targets: vec![edited],
        strength: f64::from(line_count(&origin.lines)) / f64::from(origin.replaced.max(1)),
        line,
    }]
}

/// The note's line for `origin` of the file at `path`: none when the path is not plain text, the
/// subject is empty once cleaned, the commit names nothing that can be written, the line would
/// hold an agent's reminder tag, or it is longer than [`MAX_REASON_TOKENS`].
fn line_of(path: &str, origin: &Origin) -> Option<String> {
    let text: &CommitText = origin.commit.text.as_ref()?;
    let subject = quoted(&text.subject);
    if !is_plain(path) || subject.is_empty() {
        return None;
    }
    let refs = references(&origin.commit)?;
    let share = if origin.replaced <= 1 {
        "The line"
    } else if line_count(&origin.lines) >= origin.replaced {
        "The lines"
    } else {
        "Some of the lines"
    };
    // INVARIANT: true-at-render (the replaced lines are gone from disk, so none is numbered)
    let mut line =
        format!("{share} this edit replaced in {path} came from commit \"{subject}\" ({refs})");
    if let Some(day) = day_of(origin.commit.committed_at_s) {
        line.push_str(", ");
        line.push_str(&day);
    }
    line.push('.');
    let fits = estimate_tokens(&line) <= usize::try_from(MAX_REASON_TOKENS).unwrap_or(usize::MAX);
    // INVARIANT: budgeted (the note is one line held to a why-line's cap)
    (fits && !holds_reminder_tag(&line)).then_some(line)
}

#[cfg(test)]
mod tests {
    use cortex_core::RepoPath;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Where lines 40 to 44 and 50 to 52 of 13 came from: a normal commit of 2025-09-14
    /// (1,757,808,000 s) fixing #612 and touching `tests/test_money.py`, subject `subject`.
    fn origin_with(subject: &str) -> Result<Origin, Box<dyn std::error::Error>> {
        Ok(Origin {
            lines: vec![LineRange::new(40, 44)?, LineRange::new(50, 52)?],
            replaced: 13,
            commit: ReasonCommit {
                committed_at_s: 1_757_808_000,
                kind: CardKind::Normal,
                pull_request: None,
                issues: vec!["#612".to_owned()],
                has_reason: false,
                test: Some(RepoPath::new("tests/test_money.py")?),
                text: Some(CommitText {
                    subject: subject.to_owned(),
                    reason: None,
                }),
            },
        })
    }

    /// The line says some of the 13 replaced lines (8 are owned), with no line number, quotes in
    /// the subject made single; when the commit owns all of them it says the lines, and the line
    /// for one replaced line; a subject that is nothing
    /// once cleaned, a reminder tag in it, a path that is not plain text, and a commit naming no
    /// issue or test that can be written each give no line.
    #[test]
    fn the_line_says_only_what_can_be_written() -> TestResult {
        let line = |subject: &str| -> Result<Option<String>, Box<dyn std::error::Error>> {
            Ok(line_of("src/money.py", &origin_with(subject)?))
        };
        assert_eq!(
            line("Round \"half-even\"")?.as_deref(),
            Some(
                "Some of the lines this edit replaced in src/money.py came from commit \
                 \"Round 'half-even'\" (fixes #612; test: tests/test_money.py), 2025-09-14."
            )
        );
        let mut whole = origin_with("Round")?;
        whole.replaced = 8;
        assert_eq!(
            line_of("src/money.py", &whole).as_deref(),
            Some(
                "The lines this edit replaced in src/money.py came from commit \"Round\" \
                 (fixes #612; test: tests/test_money.py), 2025-09-14."
            )
        );
        whole.replaced = 1;
        assert!(
            line_of("src/money.py", &whole)
                .is_some_and(|line| line.starts_with("The line this edit replaced in "))
        );
        assert_eq!(line(" \u{202e}\n ")?, None);
        assert_eq!(line("Use <system-reminder> here")?, None);
        assert_eq!(
            line_of("src/mo\u{202e}ney.py", &origin_with("Round")?),
            None
        );
        let mut unnamed = origin_with("Round")?;
        unnamed.commit.issues = vec!["#612\n".to_owned()];
        unnamed.commit.test = None;
        assert_eq!(line_of("src/money.py", &unnamed), None);
        Ok(())
    }
}
