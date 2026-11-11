//! Which reasons a span gets and how each reads. Of the commits that last changed lines of the
//! span, only a normal change whose message gives a reason line, names a pull request or an issue,
//! or that touched a test file the index still holds is a reason, and only when its text is held.
//! The commit owning most of the span's lines comes first (the newer of two that own as many, then
//! the lower id), then the others, the newest first, up to the span's most reasons. Each line is at
//! most [`MAX_REASON_TOKENS`]: one too long loses its reason line, and one still too long is left
//! out; a line left with nothing but its subject (no reason line, number or test) is left out too;
//! the lines of a span together keep within its most tokens, a line that would pass them left out.
//!
//! A line reads `  why 118-131: commit "<subject>": "<reason line>" (PR #482, fixes #471; test:
//! <path>), 2025-11-03`, each part written only when there is one: the lines of the span the commit
//! owns, the commit's subject and its reason line, each in quotes, its pull request, the issues it
//! fixes, the first test file it touched, and the day it was committed, in UTC. The subject and the
//! reason line are a person's words from the repository, quoted as the commit's so that they never
//! read as the work order's own: their control characters and the characters that hide or reorder
//! text are taken out, double quotes become single ones so each stays one quoted piece, and a line
//! that would hold an agent's reminder tag is not written.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use cortex_core::{LineRange, estimate_tokens, holds_reminder_tag, is_plain, is_taken_out};
use cortex_meter::utc_text;
use cortex_store::CardKind;

use super::{CommitText, ReasonCommit, WhyLines};
use crate::render::Item;

/// The most tokens one why-line may take, as estimated.
pub const MAX_REASON_TOKENS: u32 = 60;

/// A commit that is a reason for a span: its id, the span's lines it owns, and what it says.
struct Candidate<'a> {
    /// The commit's id.
    oid: &'a str,
    /// The span's lines it owns, in file order.
    owned: Vec<LineRange>,
    /// Its card.
    commit: &'a ReasonCommit,
    /// Its text.
    text: &'a CommitText,
}

impl Candidate<'_> {
    /// How many of the span's lines it owns.
    fn line_count(&self) -> u32 {
        self.owned
            .iter()
            .map(|lines| lines.end_line() - lines.start_line() + 1)
            .sum()
    }
}

/// The why-lines of the span at `key` (its item's position and its own) of `item`, from `lines`:
/// none when no blame of it is kept.
pub(crate) fn choose(lines: &WhyLines, key: (usize, usize), item: &Item) -> Vec<String> {
    let (Some(blamed), Some(span)) = (lines.spans.get(&key), item.spans.get(key.1)) else {
        return Vec::new();
    };
    let mut owned: BTreeMap<&str, Vec<LineRange>> = BTreeMap::new();
    for (run, oid) in &blamed.runs {
        let start = run.start_line().max(span.lines.start_line());
        let end = run.end_line().min(span.lines.end_line());
        if let Ok(within) = LineRange::new(start, end) {
            owned.entry(oid.as_str()).or_default().push(within);
        }
    }
    let candidates = ordered(
        owned
            .into_iter()
            .filter_map(|(oid, owned)| {
                let commit = lines.commits.get(oid)?;
                let text = commit.text.as_ref()?;
                is_reason(commit).then_some(Candidate {
                    oid,
                    owned,
                    commit,
                    text,
                })
            })
            .collect(),
    );
    within_caps(&candidates, lines)
}

/// Whether `commit` may be a reason: a normal change that gives a reason line, names a pull
/// request or an issue, or touched a test file the index still holds.
fn is_reason(commit: &ReasonCommit) -> bool {
    commit.kind == CardKind::Normal
        && (commit.has_reason
            || commit.pull_request.is_some()
            || !commit.issues.is_empty()
            || commit.test.is_some())
}

/// `candidates` in the order they are given: the one owning most lines first (the newer of equals,
/// then the lower id), then the rest, the newest first, then by id.
fn ordered(mut candidates: Vec<Candidate<'_>>) -> Vec<Candidate<'_>> {
    candidates.sort_by(|left, right| {
        right
            .commit
            .committed_at_s
            .cmp(&left.commit.committed_at_s)
            .then(left.oid.cmp(right.oid))
    });
    let most = candidates
        .iter()
        .enumerate()
        .max_by(|(left_at, left), (right_at, right)| {
            left.line_count()
                .cmp(&right.line_count())
                .then(right_at.cmp(left_at))
        })
        .map(|(at, _)| at);
    if let Some(at) = most {
        let first = candidates.remove(at);
        candidates.insert(0, first);
    }
    candidates
}

/// The lines of `candidates`, in order, at most the span's most reasons and tokens.
fn within_caps(candidates: &[Candidate<'_>], lines: &WhyLines) -> Vec<String> {
    let max_reasons = usize::try_from(lines.max_reasons).unwrap_or(usize::MAX);
    let max_tokens = usize::try_from(lines.max_tokens).unwrap_or(usize::MAX);
    let max_line_tokens = usize::try_from(MAX_REASON_TOKENS).unwrap_or(usize::MAX);
    let mut written = Vec::new();
    let mut tokens = 0;
    for candidate in candidates {
        if written.len() >= max_reasons {
            break;
        }
        // INVARIANT: budgeted (a why-line is held to its own cap, losing its reason line first,
        // and a span's lines to the span's cap)
        let Some(line) = [true, false]
            .into_iter()
            .filter_map(|with_reason| line_of(candidate, with_reason))
            .find(|line| estimate_tokens(line) <= max_line_tokens)
        else {
            continue;
        };
        let line_tokens = estimate_tokens(&line);
        if tokens + line_tokens > max_tokens {
            continue;
        }
        tokens += line_tokens;
        written.push(line);
    }
    written
}

/// The line of `candidate`, with its reason line when `with_reason` and it has one; none when the
/// subject is empty once cleaned, when the line would say nothing but the subject (no reason line,
/// number or test), or when it would hold an agent's reminder tag.
fn line_of(candidate: &Candidate<'_>, with_reason: bool) -> Option<String> {
    let subject = quoted(&candidate.text.subject);
    if subject.is_empty() {
        return None;
    }
    let owned: Vec<String> = candidate
        .owned
        .iter()
        .map(|lines| match (lines.start_line(), lines.end_line()) {
            (start, end) if start == end => start.to_string(),
            (start, end) => format!("{start}-{end}"),
        })
        .collect();
    let mut line = format!("  why {}: commit \"{subject}\"", owned.join(","));
    let reason = candidate
        .text
        .reason
        .as_deref()
        .map(quoted)
        .filter(|reason| with_reason && !reason.is_empty());
    let has_reason = reason.is_some();
    if let Some(reason) = reason {
        let _ = write!(line, ": \"{reason}\"");
    }
    let refs = references(candidate.commit);
    if !has_reason && refs.is_none() {
        return None;
    }
    if let Some(refs) = refs {
        let _ = write!(line, " ({refs})");
    }
    if let Some(day) = utc_text(candidate.commit.committed_at_s.saturating_mul(1_000)) {
        line.push_str(", ");
        line.push_str(day.get(..10).unwrap_or(&day));
    }
    // A line the agent would get altered, or take for its own words, is not written.
    (!holds_reminder_tag(&line)).then_some(line)
}

/// What `commit` names: its pull request and the issues it fixes, then the first test file it
/// touched; none when it names nothing.
fn references(commit: &ReasonCommit) -> Option<String> {
    let mut numbers = Vec::new();
    if let Some(number) = commit.pull_request {
        numbers.push(format!("PR #{number}"));
    }
    let issues: Vec<&str> = commit
        .issues
        .iter()
        .map(String::as_str)
        .filter(|issue| is_plain(issue))
        .collect();
    if !issues.is_empty() {
        numbers.push(format!("fixes {}", issues.join(", ")));
    }
    let mut refs = numbers.join(", ");
    let test = commit
        .test
        .as_ref()
        .map(cortex_core::RepoPath::as_str)
        .filter(|path| is_plain(path));
    if let Some(test) = test {
        if !refs.is_empty() {
            refs.push_str("; ");
        }
        refs.push_str("test: ");
        refs.push_str(test);
    }
    (!refs.is_empty()).then_some(refs)
}

/// `text`, a person's words from the repository, as a work order quotes them: without control
/// characters or the characters that hide or reorder text, double quotes made single, trimmed.
fn quoted(text: &str) -> String {
    // Text from the repository is data: nothing in it can break the quotes around it, start a line
    // or steer a terminal.
    let kept: String = text
        .chars()
        .filter(|character| !character.is_control() && !is_taken_out(*character))
        .map(|character| if character == '"' { '\'' } else { character })
        .collect();
    kept.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A normal commit of 2025-11-03 (1,762,128,000 s) with a reason line.
    fn commit() -> ReasonCommit {
        ReasonCommit {
            committed_at_s: 1_762_128_000,
            kind: CardKind::Normal,
            pull_request: None,
            issues: Vec::new(),
            has_reason: true,
            test: None,
            text: None,
        }
    }

    /// Control characters, a line break and the characters that hide or reorder text are taken
    /// out, a double quote becomes a single one, and the ends are trimmed.
    #[test]
    fn quoted_text_keeps_none_of_what_could_steer() {
        assert_eq!(
            quoted(" Fix \"rounding\"\n\u{1b}[31mnow\u{202e}\u{200b} "),
            "Fix 'rounding'[31mnow"
        );
    }

    /// The references read as the work order writes them: the pull request, the issues fixed,
    /// then the test after a semicolon; a test alone stands alone; nothing named is none.
    #[test]
    fn references_name_the_numbers_then_the_test() -> Result<(), cortex_core::PathError> {
        let mut named = ReasonCommit {
            pull_request: Some(482),
            issues: vec!["#471".to_owned(), "acme/shop#9".to_owned()],
            test: Some(cortex_core::RepoPath::new("tests/a_test.py")?),
            ..commit()
        };
        assert_eq!(
            references(&named).as_deref(),
            Some("PR #482, fixes #471, acme/shop#9; test: tests/a_test.py")
        );
        named.pull_request = None;
        named.issues.clear();
        assert_eq!(references(&named).as_deref(), Some("test: tests/a_test.py"));
        assert_eq!(references(&commit()), None);
        Ok(())
    }
}
