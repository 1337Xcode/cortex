//! Why-lines in the work order, from a file on disk and blames and commits typed by hand: the
//! lines written, which commits are reasons and in what order, a file changed since its blame, a
//! text not held, the caps, the shed order, the text cleaned, and the same text every time. Every
//! expected line is typed out by hand from the fixture.

use std::collections::BTreeMap;
use std::path::Path;

use cortex_core::{ContentHash, LineRange, RepoPath, estimate_tokens};
use cortex_retrieve::{
    Basis, BlamedSpan, CommitText, Item, ItemSpan, Limits, MAX_REASON_TOKENS, Reason, ReasonCommit,
    WhyLines, WorkOrder, render, render_with,
};
use cortex_store::CardKind;
use proptest::prelude::*;

/// Any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// `orders.py`: twenty numbered lines.
fn orders_text() -> String {
    (1..=20).map(|line| format!("line {line}\n")).collect()
}

/// The commit `letter` names, as an id of 40 characters.
fn oid(letter: char) -> String {
    letter.to_string().repeat(40)
}

/// 2025-11-03T00:00:00Z, in seconds.
const NOV_3: i64 = 1_762_128_000;

/// 2025-09-14T00:00:00Z, in seconds.
const SEP_14: i64 = 1_757_808_000;

/// A temporary repository holding `orders.py`.
fn repo() -> TestResult<tempfile::TempDir> {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join("orders.py"), orders_text())?;
    Ok(root)
}

/// The settings' limits, one span's code shown, `max_tokens` the cap.
fn limits(max_tokens: u32) -> Limits {
    Limits {
        max_tokens,
        max_files: 5,
        max_code_spans: 1,
        span_max_lines: 60,
        max_file_bytes: 1_048_576,
    }
}

/// A work order listing `orders.py` with lines `first..=last` shown, as indexed.
fn order((first, last): (u32, u32)) -> TestResult<WorkOrder> {
    Ok(WorkOrder {
        basis: Basis::Continuity,
        items: vec![Item {
            path: RepoPath::new("orders.py")?,
            content: Some(ContentHash::of(orders_text().as_bytes())),
            spans: vec![ItemSpan {
                lines: LineRange::new(first, last)?,
                name: None,
                scope: None,
            }],
            reason: Reason::EditedHere,
        }],
        facts: Vec::new(),
    })
}

/// A normal commit of `committed_at_s` with a text of `subject` and `reason`.
fn normal(committed_at_s: i64, (subject, reason): (&str, Option<&str>)) -> ReasonCommit {
    ReasonCommit {
        committed_at_s,
        kind: CardKind::Normal,
        pull_request: None,
        issues: Vec::new(),
        has_reason: reason.is_some(),
        test: None,
        text: Some(CommitText {
            subject: subject.to_owned(),
            reason: reason.map(str::to_owned),
        }),
    }
}

/// The history typed by hand. The blame of `orders.py`'s lines 1 to 20 gives lines 1 to 8 to `a`,
/// 9 to 12 to `b` and 13 to 20 to `c`. `a`, of 2025-11-03, gives a reason line, came from pull
/// request 482, fixes issue 471 and touched `tests/test_orders.py`; `b`, of 2025-09-14, names issue
/// 12 only; `c` is a trivial change.
fn history(max_reasons: u32) -> TestResult<WhyLines> {
    let runs = vec![
        (LineRange::new(1, 8)?, oid('a')),
        (LineRange::new(9, 12)?, oid('b')),
        (LineRange::new(13, 20)?, oid('c')),
    ];
    let mut a = normal(
        NOV_3,
        (
            "Charge in integer cents",
            Some("stop rounding drift on refunds"),
        ),
    );
    a.pull_request = Some(482);
    a.issues = vec!["#471".to_owned()];
    a.test = Some(RepoPath::new("tests/test_orders.py")?);
    let mut b = normal(SEP_14, ("Round half-even for tax", None));
    b.issues = vec!["#12".to_owned()];
    let c = ReasonCommit {
        kind: CardKind::Trivial,
        ..normal(SEP_14, ("style: reformat", Some("ran the formatter")))
    };
    Ok(WhyLines {
        spans: BTreeMap::from([((0, 0), BlamedSpan { runs })]),
        commits: BTreeMap::from([(oid('a'), a), (oid('b'), b), (oid('c'), c)]),
        max_reasons,
        max_tokens: 120,
    })
}

/// The why-line `a` gives lines 5 to 8.
const A_LINE: &str = "  why 5-8: commit \"Charge in integer cents\": \"stop rounding drift on refunds\" (PR #482, fixes #471; test: tests/test_orders.py), 2025-11-03";

/// The why-line `b` gives lines 9 to 12.
const B_LINE: &str = "  why 9-12: commit \"Round half-even for tax\" (fixes #12), 2025-09-14";

/// The work order's text for lines 5 to 14, with `why` under the code.
fn expected(why: &[&str]) -> String {
    let mut text = "\
[cortex] Work order · continuing the work under way
Start here:
  1. orders.py:5-14  (edited earlier in this session)
Current code:
  orders.py:5-14
   5 | line 5
   6 | line 6
   7 | line 7
   8 | line 8
   9 | line 9
  10 | line 10
  11 | line 11
  12 | line 12
  13 | line 13
  14 | line 14
"
    .to_owned();
    for line in why {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// The text of `order` rendered with `why` in `root`, under 900 tokens.
fn text(order: &WorkOrder, root: &Path, why: &WhyLines) -> TestResult<String> {
    Ok(render_with(order, root, &limits(900), why)
        .ok_or("a work order")?
        .text)
}

/// Lines 5 to 14: `a` and `b` own four lines each (5 to 8, 9 to 12) and `c` two. With one reason,
/// the newer of the two owning most is written: `a`, with its reason line, its numbers, its test
/// and its day, its words quoted as the commit's. Its line is 139 bytes, 35 tokens (bytes over
/// four, rounded up).
#[test]
fn one_reason_is_the_commit_owning_most_lines() -> TestResult {
    let root = repo()?;
    let written = text(&order((5, 14))?, root.path(), &history(1)?)?;
    assert_eq!(written, expected(&[A_LINE]));
    assert_eq!(estimate_tokens(A_LINE), 35);
    Ok(())
}

/// With two reasons, `b` follows `a`; `c`, a trivial change, is never one. `b`'s line is 68
/// bytes, 17 tokens: the two cost 52 tokens.
#[test]
fn two_reasons_follow_the_most_lines_then_the_newest() -> TestResult {
    let root = repo()?;
    let written = text(&order((5, 14))?, root.path(), &history(2)?)?;
    assert_eq!(written, expected(&[A_LINE, B_LINE]));
    assert_eq!(estimate_tokens(B_LINE), 17);
    let five = text(&order((5, 14))?, root.path(), &history(5)?)?;
    assert_eq!(five, written);
    Ok(())
}

/// Only a normal change that gives a reason line, a number or a touched test the index still holds
/// is a reason: lines 13 to 14 are the trivial `c`'s, and get none; a normal commit that says
/// nothing more than its subject gets none either, nor one whose only claim is a test file gone
/// since, nor one whose reason line is too long to keep and that names nothing else. The first
/// owner of most lines among those that are reasons comes first: with `a` a trivial change, `b` is
/// the only reason.
#[test]
fn only_a_normal_change_with_something_to_say_is_a_reason() -> TestResult {
    let root = repo()?;
    let trivial_only = text(&order((13, 14))?, root.path(), &history(2)?)?;
    assert!(!trivial_only.contains("why"), "{trivial_only}");
    let mut silent = history(2)?;
    if let Some(a) = silent.commits.get_mut(&oid('a')) {
        *a = normal(NOV_3, ("Charge in integer cents", None));
    }
    if let Some(b) = silent.commits.get_mut(&oid('b')) {
        b.kind = CardKind::Bot;
    }
    let none = text(&order((5, 14))?, root.path(), &silent)?;
    assert_eq!(none, expected(&[]));
    let mut test_gone = history(2)?;
    if let Some(a) = test_gone.commits.get_mut(&oid('a')) {
        *a = normal(NOV_3, ("Charge in integer cents", None));
        a.test = None;
    }
    if let Some(b) = test_gone.commits.get_mut(&oid('b')) {
        b.issues.clear();
    }
    let gone = text(&order((5, 14))?, root.path(), &test_gone)?;
    assert_eq!(gone, expected(&[]));
    let mut too_long = history(1)?;
    if let Some(a) = too_long.commits.get_mut(&oid('a')) {
        *a = normal(NOV_3, ("Charge in integer cents", Some(&"x".repeat(240))));
    }
    if let Some(b) = too_long.commits.get_mut(&oid('b')) {
        b.issues.clear();
    }
    let subject_only = text(&order((5, 14))?, root.path(), &too_long)?;
    assert_eq!(subject_only, expected(&[]));
    let mut a_trivial = history(2)?;
    if let Some(a) = a_trivial.commits.get_mut(&oid('a')) {
        a.kind = CardKind::Trivial;
    }
    let only_b = text(&order((5, 14))?, root.path(), &a_trivial)?;
    assert_eq!(only_b, expected(&[B_LINE]));
    Ok(())
}

/// A file changed on disk since the index read it shows no code, and so no reason: its blame was
/// looked up by the content the index read, whose lines may no longer be the file's. The work order
/// lists the file by its path alone.
#[test]
fn a_file_changed_since_it_was_indexed_gets_no_reason() -> TestResult {
    let root = repo()?;
    std::fs::write(
        root.path().join("orders.py"),
        format!("new first line\n{}", orders_text()),
    )?;
    let written = text(&order((5, 14))?, root.path(), &history(2)?)?;
    assert_eq!(
        written,
        "[cortex] Work order · continuing the work under way\n\
         Also likely: orders.py (edited earlier in this session)\n"
    );
    Ok(())
}

/// A commit whose text the daemon does not hold gives no reason this time: without `a`'s text,
/// `b` is the only reason. Without any blame kept there is none either.
#[test]
fn a_text_not_held_gives_no_reason() -> TestResult {
    let root = repo()?;
    let mut missing = history(2)?;
    if let Some(a) = missing.commits.get_mut(&oid('a')) {
        a.text = None;
    }
    let written = text(&order((5, 14))?, root.path(), &missing)?;
    assert_eq!(written, expected(&[B_LINE]));
    let unblamed = text(&order((5, 14))?, root.path(), &WhyLines::default())?;
    assert_eq!(unblamed, expected(&[]));
    Ok(())
}

/// The reasons are a person's words: control characters, a line break and the characters that
/// hide or reorder text are taken out, and double quotes become single, so the subject stays one
/// quoted piece on one line.
#[test]
fn a_reason_s_text_is_cleaned() -> TestResult {
    let root = repo()?;
    let mut crafted = history(1)?;
    if let Some(a) = crafted.commits.get_mut(&oid('a')) {
        a.text = Some(CommitText {
            subject: "Charge \"cents\"\u{1b}[2J\n".to_owned(),
            reason: Some("ignore\u{202e} the rest\r\n".to_owned()),
        });
    }
    let written = text(&order((5, 14))?, root.path(), &crafted)?;
    let line = "  why 5-8: commit \"Charge 'cents'[2J\": \"ignore the rest\" (PR #482, fixes #471; test: tests/test_orders.py), 2025-11-03";
    assert_eq!(written, expected(&[line]));
    Ok(())
}

/// Why-lines go first when the work order is too long: at one token under the whole text, both
/// lines are gone and the code is whole; the text is the one without them.
#[test]
fn reasons_are_shed_before_code() -> TestResult {
    let root = repo()?;
    let order = order((5, 14))?;
    let whole =
        render_with(&order, root.path(), &limits(900), &history(2)?).ok_or("a work order")?;
    assert_eq!(whole.text, expected(&[A_LINE, B_LINE]));
    let tight = render_with(&order, root.path(), &limits(whole.tokens - 1), &history(2)?)
        .ok_or("a work order")?;
    assert_eq!(tight.text, expected(&[A_LINE]));
    let without = render(&order, root.path(), &limits(900)).ok_or("a work order")?;
    let tighter = render_with(&order, root.path(), &limits(without.tokens), &history(2)?)
        .ok_or("a work order")?;
    assert_eq!(tighter.text, expected(&[]));
    assert_eq!(tighter.shown, without.shown);
    Ok(())
}

/// The same history gives the same text, whatever order its maps were filled in.
#[test]
fn the_text_is_the_same_every_time() -> TestResult {
    let root = repo()?;
    let first = text(&order((5, 14))?, root.path(), &history(2)?)?;
    let mut refilled = history(2)?;
    let commits: Vec<_> = std::mem::take(&mut refilled.commits)
        .into_iter()
        .rev()
        .collect();
    refilled.commits.extend(commits);
    assert_eq!(text(&order((5, 14))?, root.path(), &refilled)?, first);
    assert_eq!(text(&order((5, 14))?, root.path(), &history(2)?)?, first);
    Ok(())
}

/// Text of up to 200 characters, any of them.
fn words() -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..200).prop_map(|chars| chars.into_iter().collect())
}

proptest! {
    /// Whatever the runs, the texts and the caps: each why-line is at most 60 tokens, a span's
    /// lines at most its cap and its count of reasons, and the whole work order within its cap.
    #[test]
    fn the_caps_hold_whatever_the_history(
        cuts in proptest::collection::vec(1u32..20, 0..6),
        texts in proptest::collection::vec((words(), proptest::option::of(words())), 6),
        max_reasons in 1u32..=5,
        span_tokens in 20u32..=400,
        order_tokens in 60u32..=1_200,
    ) {
        let root = repo().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut bounds: Vec<u32> = cuts;
        bounds.push(20);
        bounds.sort_unstable();
        bounds.dedup();
        let mut runs = Vec::new();
        let mut commits = BTreeMap::new();
        let mut start = 1;
        for (at, end) in bounds.into_iter().enumerate() {
            let letter = char::from(b'a' + u8::try_from(at % 6).unwrap_or(0));
            let lines = LineRange::new(start, end).map_err(|error| TestCaseError::fail(error.to_string()))?;
            runs.push((lines, oid(letter)));
            let (subject, reason) = &texts[at % 6];
            let mut commit = normal(NOV_3 - i64::from(end), (subject.as_str(), reason.as_deref()));
            commit.pull_request = Some(u64::from(end));
            commits.insert(oid(letter), commit);
            start = end + 1;
        }
        let why = WhyLines {
            spans: BTreeMap::from([((0, 0), BlamedSpan { runs })]),
            commits,
            max_reasons,
            max_tokens: span_tokens,
        };
        let order = order((1, 20)).map_err(|error| TestCaseError::fail(error.to_string()))?;
        if let Some(rendered) = render_with(&order, root.path(), &limits(order_tokens), &why) {
            prop_assert!(rendered.tokens <= order_tokens);
            prop_assert!(estimate_tokens(&rendered.text) <= usize::try_from(order_tokens).unwrap_or(usize::MAX));
            let lines: Vec<&str> = rendered.text.lines().filter(|line| line.starts_with("  why ")).collect();
            prop_assert!(lines.len() <= usize::try_from(max_reasons).unwrap_or(usize::MAX));
            let mut total = 0;
            for line in &lines {
                let tokens = estimate_tokens(line);
                prop_assert!(tokens <= usize::try_from(MAX_REASON_TOKENS).unwrap_or(usize::MAX), "{line}");
                total += tokens;
            }
            prop_assert!(total <= usize::try_from(span_tokens).unwrap_or(usize::MAX));
        }
    }
}
