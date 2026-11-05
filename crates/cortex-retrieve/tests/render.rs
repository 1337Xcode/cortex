//! The work order as written, from files on disk: the text typed out by hand, the caps held, and
//! nothing that changed since it was indexed shown.

mod common;

use common::{Repo, TestResult};
use cortex_core::{ContentHash, FileFlags, Identifier, LineRange, RepoPath, estimate_tokens};
use cortex_retrieve::{
    Basis, Item, ItemSpan, Limits, MAX_WORK_ORDER_CHARS, Reason, WorkOrder, render,
};
use proptest::prelude::*;

/// The settings' limits, with `max_code_spans` code bodies shown.
fn limits(max_code_spans: usize) -> Limits {
    Limits {
        max_tokens: 900,
        max_files: 5,
        max_code_spans,
        span_max_lines: 60,
        max_file_bytes: 1_048_576,
    }
}

/// An item for `path` holding `text`, with spans of `lines`, the first named `name`, listed for
/// `reason`.
fn item(
    (path, text): (&str, &str),
    lines: &[(u32, u32)],
    name: Option<&str>,
    reason: Reason,
) -> TestResult<Item> {
    let mut spans = Vec::new();
    for (at, (start, end)) in lines.iter().enumerate() {
        spans.push(ItemSpan {
            lines: LineRange::new(*start, *end)?,
            name: if at == 0 {
                name.map(Identifier::new).transpose()?
            } else {
                None
            },
        });
    }
    Ok(Item {
        path: RepoPath::new(path)?,
        content: Some(ContentHash::of(text.as_bytes())),
        spans,
        reason,
    })
}

/// `a.py`'s text: two functions.
const A: &str = "def f():\n    return 1\n\n\ndef g():\n    return 2\n";

/// `b.py`'s text.
const B: &str = "x = 1\n";

/// A work order lists only files that are there. Of three items, `gone.py` was indexed and then
/// deleted, `link.py` is a folder where the file was, and `a.py` is as indexed: only `a.py` is
/// written, its code shown. With the deleted file alone there is nothing to list and no work order.
#[test]
fn a_file_that_is_gone_is_not_listed() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", A, &[], FileFlags::empty())?;
    repo.file("gone.py", B, &[], FileFlags::empty())?;
    repo.file("link.py", B, &[], FileFlags::empty())?;
    std::fs::remove_file(repo.root.path().join("gone.py"))?;
    std::fs::remove_file(repo.root.path().join("link.py"))?;
    std::fs::create_dir(repo.root.path().join("link.py"))?;
    let gone = item(("gone.py", B), &[(1, 1)], None, Reason::LikeEarlierWork)?;
    let order = WorkOrder {
        basis: Basis::Agreement(vec!["the prompt's words", "the commit history"]),
        items: vec![
            gone.clone(),
            item(("link.py", B), &[(1, 1)], None, Reason::LikeEarlierWork)?,
            item(
                ("a.py", A),
                &[(1, 2)],
                Some("f"),
                Reason::Matches(vec!["f".to_owned()]),
            )?,
        ],
    };
    let rendered = render(&order, repo.root.path(), &limits(1)).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · the prompt's words and the commit history agree
Start here:
  1. a.py:1-2  f  (matches f)
Current code:
  a.py:1-2
  1 | def f():
  2 |     return 1
";
    assert_eq!(rendered.text, expected);
    let alone = WorkOrder {
        basis: Basis::Agreement(vec!["the prompt's words", "the commit history"]),
        items: vec![gone],
    };
    assert!(render(&alone, repo.root.path(), &limits(1)).is_none());
    Ok(())
}

/// A file that cannot be opened is not listed either, whether the index read it or holds no
/// content for it. Of five items, `locked.py` (indexed) and `bare.py` (nothing indexed) cannot be
/// opened, `deleted.py` (nothing indexed) is not there and `folder.py` (nothing indexed) is a
/// folder; only `a.py` is written, its code shown. Without `a.py` there is no work order.
#[test]
fn a_file_that_cannot_be_read_is_not_listed() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", A, &[], FileFlags::empty())?;
    repo.file("locked.py", B, &[], FileFlags::empty())?;
    std::fs::write(repo.root.path().join("bare.py"), B)?;
    std::fs::create_dir(repo.root.path().join("folder.py"))?;
    let _locked = unreadable(&repo.root.path().join("locked.py"))?;
    let _bare = unreadable(&repo.root.path().join("bare.py"))?;
    let unindexed = |path: &str| -> TestResult<Item> {
        Ok(Item {
            content: None,
            ..item((path, ""), &[], None, Reason::LikeEarlierWork)?
        })
    };
    let not_there = vec![
        item(("locked.py", B), &[(1, 1)], None, Reason::LikeEarlierWork)?,
        unindexed("bare.py")?,
        unindexed("deleted.py")?,
        unindexed("folder.py")?,
    ];
    let mut items = not_there.clone();
    items.push(item(
        ("a.py", A),
        &[(1, 2)],
        Some("f"),
        Reason::Matches(vec!["f".to_owned()]),
    )?);
    let basis = Basis::Agreement(vec!["the prompt's words", "the commit history"]);
    let order = WorkOrder {
        basis: basis.clone(),
        items,
    };
    let rendered = render(&order, repo.root.path(), &limits(1)).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · the prompt's words and the commit history agree
Start here:
  1. a.py:1-2  f  (matches f)
Current code:
  a.py:1-2
  1 | def f():
  2 |     return 1
";
    assert_eq!(rendered.text, expected);
    let alone = WorkOrder {
        basis,
        items: not_there,
    };
    assert!(render(&alone, repo.root.path(), &limits(1)).is_none());
    Ok(())
}

/// Makes the file at `path` impossible to open for reading while what this returns is kept: a
/// handle that shares nothing with other openers.
#[cfg(windows)]
fn unreadable(path: &std::path::Path) -> TestResult<Option<std::fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)?;
    assert!(
        std::fs::File::open(path).is_err(),
        "the held file can still be opened"
    );
    Ok(Some(held))
}

/// Takes every permission off the file at `path`, so that it cannot be opened for reading; nothing
/// needs keeping. A user whom file permissions do not bind, such as root, fails the test here
/// rather than passing it without checking anything.
#[cfg(unix)]
fn unreadable(path: &std::path::Path) -> TestResult<Option<std::fs::File>> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000))?;
    assert!(
        std::fs::File::open(path).is_err(),
        "the file can still be opened"
    );
    Ok(None)
}

/// A work order typed out by hand: the header names what agreed; `a.py`, whose code is shown, is
/// where to start, with its symbol and reason; `b.py` with its lines and `c.py`, which the index
/// holds no contents for, by path alone, are also likely; the code shown is `a.py`'s lines 1 and
/// 2, numbered. Every item is written, `a.py` and `b.py` with their span and `c.py` with none.
#[test]
fn a_work_order_is_written_as_typed_here() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", A, &[], FileFlags::empty())?;
    repo.file("b.py", B, &[], FileFlags::empty())?;
    std::fs::write(repo.root.path().join("c.py"), "")?;
    let order = WorkOrder {
        basis: Basis::Agreement(vec!["the prompt's words", "the commit history"]),
        items: vec![
            item(
                ("a.py", A),
                &[(1, 2)],
                Some("f"),
                Reason::Matches(vec!["f".to_owned()]),
            )?,
            item(
                ("b.py", B),
                &[(1, 1)],
                None,
                Reason::ChangesWith {
                    other: "a.py".to_owned(),
                    together: 3,
                    of: 4,
                },
            )?,
            Item {
                content: None,
                ..item(("c.py", ""), &[], None, Reason::LikeEarlierWork)?
            },
        ],
    };
    let rendered = render(&order, repo.root.path(), &limits(1)).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · the prompt's words and the commit history agree
Start here:
  1. a.py:1-2  f  (matches f)
Also likely: b.py:1-1 (changes with a.py in 3 of 4 commits), c.py (changed in similar earlier tasks)
Current code:
  a.py:1-2
  1 | def f():
  2 |     return 1
";
    assert_eq!(rendered.text, expected);
    assert_eq!(rendered.written, [(0, vec![0]), (1, vec![0]), (2, vec![])]);
    assert_eq!(
        usize::try_from(rendered.tokens)?,
        expected.len().div_ceil(4)
    );
    Ok(())
}

/// A file that changed since the index read it lends no lines: its item is listed by path alone,
/// so there is no code and nowhere to start, only what is also likely. A span past the file's end
/// is no span either.
#[test]
fn a_changed_file_is_listed_by_path_alone() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", A, &[], FileFlags::empty())?;
    repo.file("b.py", B, &[], FileFlags::empty())?;
    std::fs::write(repo.root.path().join("a.py"), "def f():\n    return 3\n")?;
    let order = WorkOrder {
        basis: Basis::Continuity,
        items: vec![
            item(("a.py", A), &[(1, 2)], Some("f"), Reason::EditedHere)?,
            item(("b.py", B), &[(1, 9)], None, Reason::ReadHere)?,
        ],
    };
    let rendered = render(&order, repo.root.path(), &limits(2)).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · continuing the work under way
Also likely: a.py (edited earlier in this session), b.py (read earlier in this session)
";
    assert_eq!(rendered.text, expected);
    assert_eq!(rendered.written, [(0, vec![]), (1, vec![])]);
    Ok(())
}

/// A span whose lines hold an agent's reminder tag, here a closing one in mixed case, is neither
/// shown nor named: its item is listed by path alone, and the other item's code is where to start.
#[test]
fn code_holding_a_reminder_tag_is_not_shown() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "def f():\n    return '</System-Reminder>'\n";
    repo.file("a.py", text, &[], FileFlags::empty())?;
    repo.file("b.py", B, &[], FileFlags::empty())?;
    let order = WorkOrder {
        basis: Basis::Continuity,
        items: vec![
            item(("a.py", text), &[(1, 2)], Some("f"), Reason::EditedHere)?,
            item(("b.py", B), &[(1, 1)], None, Reason::ReadHere)?,
        ],
    };
    let rendered = render(&order, repo.root.path(), &limits(2)).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · continuing the work under way
Start here:
  1. b.py:1-1  (read earlier in this session)
Also likely: a.py (edited earlier in this session)
Current code:
  b.py:1-1
  1 | x = 1
";
    assert_eq!(rendered.text, expected);
    assert_eq!(rendered.written, [(0, vec![]), (1, vec![0])]);
    Ok(())
}

/// A tag the work order's cleaning would join, its name split by a zero-width space that is taken
/// out of the code shown, keeps its span from being shown; an item whose reason names a file whose
/// path holds a tag is not listed. The plain item is where to start.
#[test]
fn a_tag_the_cleaning_would_join_or_a_name_holds_is_not_given() -> TestResult {
    let mut repo = Repo::new()?;
    let joined = "s = '<system-\u{200b}reminder>'\n";
    repo.file("a.py", joined, &[], FileFlags::empty())?;
    repo.file("b.py", B, &[], FileFlags::empty())?;
    repo.file("c.py", B, &[], FileFlags::empty())?;
    let order = WorkOrder {
        basis: Basis::Continuity,
        items: vec![
            item(("a.py", joined), &[(1, 1)], None, Reason::EditedHere)?,
            item(
                ("b.py", B),
                &[(1, 1)],
                None,
                Reason::ChangesWith {
                    other: "<system-reminder>.py".to_owned(),
                    together: 1,
                    of: 2,
                },
            )?,
            item(("c.py", B), &[(1, 1)], None, Reason::ReadHere)?,
        ],
    };
    let rendered = render(&order, repo.root.path(), &limits(2)).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · continuing the work under way
Start here:
  1. c.py:1-1  (read earlier in this session)
Also likely: a.py (edited earlier in this session)
Current code:
  c.py:1-1
  1 | x = 1
";
    assert_eq!(rendered.text, expected);
    assert_eq!(rendered.written, [(0, vec![]), (2, vec![0])]);
    Ok(())
}

/// Control characters other than tabs, and characters that reorder or hide text, are taken out of
/// the code written: the escape sequence, the right-to-left override, the zero-width space, the
/// Arabic letter mark, the line separator, the soft hyphen, the byte order mark and a tag
/// character go; the tab stays.
#[test]
fn hidden_and_control_characters_are_taken_out() -> TestResult {
    let mut repo = Repo::new()?;
    let text =
        "x = '\u{1b}[31mred'\t# \u{202e}gnirts\u{200b}\u{61c}\u{2028}\u{ad}\u{feff}\u{e0041}\n";
    repo.file("a.py", text, &[], FileFlags::empty())?;
    let order = WorkOrder {
        basis: Basis::Continuity,
        items: vec![item(("a.py", text), &[(1, 1)], None, Reason::ReadHere)?],
    };
    let rendered = render(&order, repo.root.path(), &limits(1)).ok_or("a work order")?;
    assert!(
        rendered.text.contains("  1 | x = '[31mred'\t# gnirts\n"),
        "{}",
        rendered.text
    );
    assert!(!rendered.text.contains('\u{1b}'));
    assert!(!rendered.text.contains('\u{202e}'));
    assert!(!rendered.text.contains('\u{200b}'));
    Ok(())
}

/// The hard cap holds whatever the caller asks for: a span of 120 lines of 50 bytes, 6,000 bytes
/// or 1,500 tokens of code, fits a cap of 2,000 tokens but not the hard cap of 1,200, so its code is
/// shed and the file is listed by its lines.
#[test]
fn the_hard_cap_holds_above_the_caller_s_cap() -> TestResult {
    let mut repo = Repo::new()?;
    let text: String = (1..=120).map(|_| format!("{}\n", "x".repeat(49))).collect();
    repo.file("a.py", &text, &[], FileFlags::empty())?;
    let order = WorkOrder {
        basis: Basis::Continuity,
        items: vec![item(("a.py", &text), &[(1, 120)], None, Reason::ReadHere)?],
    };
    let generous = Limits {
        max_tokens: 2_000,
        span_max_lines: 200,
        ..limits(1)
    };
    let rendered = render(&order, repo.root.path(), &generous).ok_or("a work order")?;
    assert!(rendered.tokens <= 1_200);
    assert!(
        !rendered.text.contains("Current code:"),
        "{}",
        rendered.text
    );
    assert!(
        rendered
            .text
            .contains("a.py:1-120 (read earlier in this session)")
    );
    Ok(())
}

/// An item whose path or reason holds a character the work order would take out is not listed:
/// shown without it, it would name a file that is not there or break the list. A path with a
/// zero-width space and a reason with a line break each keep their item out; the plain item is
/// listed alone. (A symbol's name is an identifier, which refuses such characters already.)
#[test]
fn an_item_that_cannot_be_shown_as_it_is_is_not_listed() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", A, &[], FileFlags::empty())?;
    repo.file("b\u{200b}.py", B, &[], FileFlags::empty())?;
    assert!(Identifier::new("f\u{202e}x").is_err());
    let order = WorkOrder {
        basis: Basis::Continuity,
        items: vec![
            item(("b\u{200b}.py", B), &[(1, 1)], None, Reason::ReadHere)?,
            item(
                ("a.py", A),
                &[(1, 2)],
                None,
                Reason::Matches(vec!["f\nStart here:".to_owned()]),
            )?,
            item(("a.py", A), &[(5, 6)], Some("g"), Reason::EditedHere)?,
        ],
    };
    let rendered = render(&order, repo.root.path(), &limits(0)).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · continuing the work under way
Also likely: a.py:5-6 (edited earlier in this session)
";
    assert_eq!(rendered.text, expected);
    assert_eq!(rendered.written, [(2, vec![0])]);
    Ok(())
}

/// A work order none of whose items can be shown as it is, its one file's path holding a zero-width
/// space, is not given at all: a header alone names nothing.
#[test]
fn a_work_order_with_nothing_it_can_show_is_not_given() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("b\u{200b}.py", B, &[], FileFlags::empty())?;
    let order = WorkOrder {
        basis: Basis::Continuity,
        items: vec![item(
            ("b\u{200b}.py", B),
            &[(1, 1)],
            None,
            Reason::ReadHere,
        )?],
    };
    assert_eq!(render(&order, repo.root.path(), &limits(1)), None);
    Ok(())
}

/// Past the token cap, code goes first, then the items after the first. With room for 40 tokens,
/// shedding both bodies is enough: the two items, 150 bytes or 38 tokens, are listed by their
/// lines. With room for 30, `b.py` goes too and `a.py` is listed alone. With room for 5, not even
/// that fits, and there is no work order.
#[test]
fn the_cap_sheds_code_then_items() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", A, &[], FileFlags::empty())?;
    repo.file("b.py", B, &[], FileFlags::empty())?;
    let order = WorkOrder {
        basis: Basis::Continuity,
        items: vec![
            item(("a.py", A), &[(1, 2)], Some("f"), Reason::EditedHere)?,
            item(("b.py", B), &[(1, 1)], None, Reason::ReadHere)?,
        ],
    };
    let roomier = Limits {
        max_tokens: 40,
        ..limits(2)
    };
    let rendered = render(&order, repo.root.path(), &roomier).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · continuing the work under way
Also likely: a.py:1-2 (edited earlier in this session), b.py:1-1 (read earlier in this session)
";
    assert_eq!(rendered.text, expected);
    assert_eq!(rendered.tokens, 38);
    let tight = Limits {
        max_tokens: 30,
        ..limits(2)
    };
    let one_file = Limits {
        max_files: 1,
        ..limits(2)
    };
    let rendered = render(&order, repo.root.path(), &one_file).ok_or("a work order")?;
    assert_eq!(rendered.written, [(0, vec![0])]);
    let short_spans = Limits {
        span_max_lines: 1,
        ..limits(2)
    };
    let rendered = render(&order, repo.root.path(), &short_spans).ok_or("a work order")?;
    assert_eq!(rendered.written, [(0, vec![]), (1, vec![0])]);
    let rendered = render(&order, repo.root.path(), &tight).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · continuing the work under way
Also likely: a.py:1-2 (edited earlier in this session)
";
    assert_eq!(rendered.text, expected);
    assert!(rendered.tokens <= 30);
    let none = Limits {
        max_tokens: 5,
        ..limits(2)
    };
    assert_eq!(render(&order, repo.root.path(), &none), None);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Whatever the items, their lines and the limits, a work order holds no more tokens than its
    /// cap (never more than 1,200) and no more than 8,000 characters, and the same order written
    /// twice is the same text.
    #[test]
    fn a_work_order_keeps_its_caps_and_its_bytes(
        count in 1_usize..8,
        lines in 1_u32..400,
        max_tokens in 1_u32..2_000,
        max_code_spans in 0_usize..4,
    ) {
        let repo = Repo::new();
        prop_assume!(repo.is_ok());
        let Ok(mut repo) = repo else { return Ok(()) };
        let text: String = (1..=lines).map(|line| format!("line {line} {}\n", "é→".repeat(20))).collect();
        let mut items = Vec::new();
        for at in 0..count {
            let path = format!("f{at}.py");
            prop_assert!(repo.file(&path, &text, &[], FileFlags::empty()).is_ok());
            let span = item((&path, &text), &[(1, lines)], Some("f"), Reason::ReadHere);
            prop_assert!(span.is_ok());
            if let Ok(span) = span {
                items.push(span);
            }
        }
        let order = WorkOrder { basis: Basis::Continuity, items };
        let limits = Limits {
            max_tokens,
            max_files: 5,
            max_code_spans,
            span_max_lines: 400,
            max_file_bytes: 1_048_576,
        };
        let first = render(&order, repo.root.path(), &limits);
        prop_assert_eq!(&first, &render(&order, repo.root.path(), &limits));
        if let Some(rendered) = first {
            let tokens = u32::try_from(estimate_tokens(&rendered.text)).unwrap_or(u32::MAX);
            prop_assert_eq!(tokens, rendered.tokens);
            prop_assert!(tokens <= max_tokens.min(1_200));
            prop_assert!(rendered.text.chars().count() <= MAX_WORK_ORDER_CHARS);
        }
    }
}
