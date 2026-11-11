//! The origin note after an edit, read from a store holding the blame of `src/money.py` as it was
//! before the edit and the cards of the four commits it names, and from a history holding their
//! texts: each line typed out by hand, which commit is told, the kinds of commit that are told,
//! the cap and once a task, and the edits that get nothing.

mod common;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use common::task::{task, warned};
use common::{Def, Repo, TestResult, function};
use cortex_core::{
    ContentHash, EdgeType, FileFlags, LineRange, RepoPath, ScaleMode, SymbolKind, Ulid,
    WarningKind, WarningsConfig,
};
use cortex_retrieve::{
    CommitText, EditQuery, OriginQuery, WhyHistory, edit_facts, edit_origin, task_so_far,
    warnings_after_edit,
};
use cortex_store::{BlameRecord, BlameRunRecord, CardKind, ChangeCardRecord, NodeId};

/// When the first three commits were committed: 2025-09-14T00:00:00Z, in seconds.
const SEPTEMBER_14_S: i64 = 1_757_808_000;

/// When commit 4 was committed: a day later, 2025-09-15.
const SEPTEMBER_15_S: i64 = SEPTEMBER_14_S + 86_400;

/// The commit numbered `number`, an id of 40 digits.
fn oid(number: u32) -> String {
    format!("{number:040}")
}

/// `src/money.py` as it was before the edits: 60 lines, `x1 = 1` to `x60 = 60`.
fn before() -> String {
    (1..=60).map(|line| format!("x{line} = {line}\n")).collect()
}

/// The hash of [`before`], as the agent reports the file before an edit.
fn before_hash() -> ContentHash {
    ContentHash::of(before().as_bytes())
}

/// A history holding the texts of the commits in `texts`, counting what is asked of it.
#[derive(Default)]
struct Held {
    /// Each commit's text, by id.
    texts: BTreeMap<String, CommitText>,
    /// The texts asked for, in order.
    asked: RefCell<Vec<String>>,
    /// How many blames were asked for.
    blames: Cell<usize>,
}

impl WhyHistory for Held {
    fn commit_text(&self, oid: &str) -> Option<CommitText> {
        self.texts.get(oid).cloned()
    }

    fn ask_for_text(&self, oid: &str) {
        self.asked.borrow_mut().push(oid.to_owned());
    }

    fn ask_for_blame(&self, _path: &RepoPath, _file_hash: ContentHash, _lines: LineRange) {
        self.blames.set(self.blames.get() + 1);
    }
}

/// A history holding the subject of each of the four commits.
fn holding_all() -> Held {
    let texts = [
        (1, "Add the money helpers"),
        (2, "Round half-even for tax"),
        (3, "Format money.py"),
        (4, "Keep cents exact on refunds"),
    ]
    .into_iter()
    .map(|(number, subject)| {
        let text = CommitText {
            subject: subject.to_owned(),
            reason: None,
        };
        (oid(number), text)
    })
    .collect();
    Held {
        texts,
        ..Held::default()
    }
}

/// The store the tests read, and the nodes of `src/money.py` and `tests/test_money.py`.
struct Store {
    /// The repository and its store.
    repo: Repo,
    /// The task the edits are in.
    task: Ulid,
    /// The node of `src/money.py`.
    money: NodeId,
    /// The node of `tests/test_money.py`.
    test: NodeId,
}

/// `src/money.py` stored as [`before`], its 60 lines blamed in that content:
/// - 1 to 39 to commit 1, a normal change from pull request #3 naming no issue and no test;
/// - 40 to 48 to commit 2, a normal change fixing #612 that touched `tests/test_money.py`;
/// - 49 to 52 to commit 4, a normal change of a day later fixing #700;
/// - 53 to 60 to commit 3, a formatting change that names #9.
///
/// An open task of session `s1`.
fn store() -> TestResult<Store> {
    let mut repo = Repo::new()?;
    let money = *repo
        .file("src/money.py", &before(), &[], FileFlags::empty())?
        .first()
        .ok_or("a node")?;
    let test = *repo
        .file(
            "tests/test_money.py",
            "def test_round():\n    pass\n",
            &[Def {
                kind: SymbolKind::Test,
                ..function("test_round", 1, 2)
            }],
            FileFlags::empty(),
        )?
        .first()
        .ok_or("a node")?;
    let mut transaction = repo.writer.transaction()?;
    let run = |start, end, number| BlameRunRecord {
        start,
        end,
        source_start: start,
        oid: oid(number),
    };
    transaction.put_blame(&BlameRecord {
        path: RepoPath::new("src/money.py")?,
        file_hash: before_hash(),
        first_line: 1,
        last_line: 60,
        runs: vec![
            run(1, 39, 1),
            run(40, 48, 2),
            run(49, 52, 4),
            run(53, 60, 3),
        ],
        computed_at_ms: 1,
    })?;
    let card = |number, kind| ChangeCardRecord {
        oid: oid(number),
        first_parent: None,
        committed_at_s: SEPTEMBER_14_S,
        kind,
        pull_request: None,
        issues: Vec::new(),
        has_reason: false,
        files: vec![money],
        symbols: Vec::new(),
        tests: Vec::new(),
        carded_at_ms: 1,
    };
    let cards = [
        ChangeCardRecord {
            pull_request: Some(3),
            has_reason: true,
            ..card(1, CardKind::Normal)
        },
        ChangeCardRecord {
            issues: vec!["#612".to_owned()],
            tests: vec![test],
            ..card(2, CardKind::Normal)
        },
        ChangeCardRecord {
            issues: vec!["#9".to_owned()],
            ..card(3, CardKind::Trivial)
        },
        ChangeCardRecord {
            issues: vec!["#700".to_owned()],
            committed_at_s: SEPTEMBER_15_S,
            ..card(4, CardKind::Normal)
        },
    ];
    for card in &cards {
        transaction.put_change_card(card, &[])?;
    }
    transaction.commit()?;
    let task = task(&mut repo, None)?;
    Ok(Store {
        repo,
        task,
        money,
        test,
    })
}

/// The warnings' text an edit of `src/money.py` gets in `store`'s task, the edit replacing
/// `replaced` (numbered before it) of the file that hashed `before_hash`, with the texts `history`
/// holds and `config`: none when no warning is given.
fn told(
    store: &Store,
    (replaced, before_hash): (&[LineRange], ContentHash),
    history: &Held,
    config: &WarningsConfig,
) -> TestResult<Option<String>> {
    let reader = store.repo.reader()?;
    let edited = RepoPath::new("src/money.py")?;
    let asked = EditQuery {
        root: store.repo.root.path(),
        edited: &edited,
        drifted: &[],
        link_decay: 0.5,
        max_file_bytes: 1_048_576,
    };
    let mut facts = edit_facts(reader.queries(), &asked)?.ok_or("no facts")?;
    let query = OriginQuery {
        history,
        replaced,
        before_hash,
    };
    facts.origin = edit_origin(reader.queries(), &edited, &query)?;
    let so_far = task_so_far(reader.queries(), store.task, store.money)?;
    Ok(warnings_after_edit(&facts, &so_far, config, ScaleMode::Medium).text)
}

/// The note of an edit replacing lines `first` to `last` of the file before it.
fn note_of(store: &Store, (first, last): (u32, u32), history: &Held) -> TestResult<Option<String>> {
    let replaced = [LineRange::new(first, last)?];
    told(
        store,
        (&replaced, before_hash()),
        history,
        &WarningsConfig::default(),
    )
}

/// The note of commit 2, saying how much of the replaced lines it last changed.
fn of_commit_2(share: &str) -> String {
    format!(
        "[cortex] {share} this edit replaced in src/money.py came from commit \"Round half-even \
         for tax\" (fixes #612; test: tests/test_money.py), 2025-09-14.\n"
    )
}

/// The note of commit 4, saying how much of the replaced lines it last changed.
fn of_commit_4(share: &str) -> String {
    format!(
        "[cortex] {share} this edit replaced in src/money.py came from commit \"Keep cents exact \
         on refunds\" (fixes #700), 2025-09-15.\n"
    )
}

/// The eligible commit that last changed the most of the replaced lines is told, worked by hand
/// from the blame:
/// - 40 to 52: commit 2 owns 9 lines, commit 4 (newer) 4: commit 2, for some of them;
/// - 47 to 52: commit 2 owns 2, commit 4 owns 4: commit 4, for some of them;
/// - 47 to 50: 2 each: the newer, commit 4;
/// - 40 to 48: commit 2 owns them all: the lines; 45 alone: the line.
///
/// Nothing more was asked of the history, and no blame.
#[test]
fn the_eligible_commit_with_the_most_lines_is_told() -> TestResult {
    let store = store()?;
    let history = holding_all();
    let cases = [
        ((40, 52), of_commit_2("Some of the lines")),
        ((47, 52), of_commit_4("Some of the lines")),
        ((47, 50), of_commit_4("Some of the lines")),
        ((40, 48), of_commit_2("The lines")),
        ((45, 45), of_commit_2("The line")),
    ];
    for (lines, expected) in cases {
        assert_eq!(
            note_of(&store, lines, &history)?.as_deref(),
            Some(expected.as_str()),
            "{lines:?}"
        );
    }
    assert!(history.asked.borrow().is_empty());
    assert_eq!(history.blames.get(), 0);
    Ok(())
}

/// Only a normal commit that names an issue or touched a test is told. Lines 10 to 12 came from
/// commit 1, which names only a pull request: nothing. Lines 50 to 58 came from commit 4 (50 to
/// 52) and from the formatting commit 3 (53 to 58), which owns more of them but is not a normal
/// change: commit 4 is told.
#[test]
fn only_a_normal_commit_naming_an_issue_or_a_test_is_told() -> TestResult {
    let store = store()?;
    let history = holding_all();
    assert_eq!(note_of(&store, (10, 12), &history)?, None);
    assert_eq!(
        note_of(&store, (50, 58), &history)?.as_deref(),
        Some(of_commit_4("Some of the lines").as_str())
    );
    Ok(())
}

/// Nothing is said, and no blame is asked for: when the history does not hold commit 2's text
/// (which is asked for, once, for a later edit); when the file before the edit hashes as content
/// no blame was kept of (a formatter or another session changed it since the blame); when the edit
/// replaced the lines of two hunks, or of none (it only added lines).
#[test]
fn an_edit_whose_origin_is_not_held_gets_nothing() -> TestResult {
    let store = store()?;
    let config = WarningsConfig::default();
    let lines = [LineRange::new(40, 48)?];

    let empty = Held::default();
    assert_eq!(
        told(&store, (&lines, before_hash()), &empty, &config)?,
        None
    );
    assert_eq!(*empty.asked.borrow(), [oid(2)]);

    let history = holding_all();
    let other = ContentHash::of(b"x1 = 1\n");
    assert_eq!(told(&store, (&lines, other), &history, &config)?, None);
    // Either hunk alone would be told: lines 40 to 48 are commit 2's.
    let two = [LineRange::new(40, 48)?, LineRange::new(50, 52)?];
    assert_eq!(
        told(&store, (&two, before_hash()), &history, &config)?,
        None
    );
    assert_eq!(told(&store, (&[], before_hash()), &history, &config)?, None);
    assert_eq!(history.blames.get() + empty.blames.get(), 0);
    assert!(history.asked.borrow().is_empty());
    Ok(())
}

/// A file whose content before the edit was never blamed gets nothing, and no blame is asked for:
/// the content is gone once edited. `tests/test_money.py` has no blame kept.
#[test]
fn a_file_never_blamed_gets_nothing() -> TestResult {
    let store = store()?;
    let history = holding_all();
    let reader = store.repo.reader()?;
    let replaced = [LineRange::new(1, 1)?];
    let query = OriginQuery {
        history: &history,
        replaced: &replaced,
        before_hash: ContentHash::of(b"def test_round():\n    pass\n"),
    };
    let found = edit_origin(
        reader.queries(),
        &RepoPath::new("tests/test_money.py")?,
        &query,
    )?;
    assert_eq!(found, None);
    assert_eq!(history.blames.get(), 0);
    assert!(history.asked.borrow().is_empty());
    Ok(())
}

/// The note comes after the edit's other warnings and goes first when the cap binds: with the
/// tests of `src/money.py` untouched, one warning an edit gives the tests alone, three give the
/// tests then the note. Once the task was told an origin, it is told no other.
#[test]
fn the_note_keeps_to_the_cap_and_is_told_once_a_task() -> TestResult {
    let mut store = store()?;
    store
        .repo
        .link((store.test, store.money), EdgeType::Structural, (0.8, 1, 0))?;
    let history = holding_all();
    let lines = [LineRange::new(40, 48)?];
    let edit = (&lines[..], before_hash());
    let tests_line = "[cortex] Tests for src/money.py: tests/test_money.py\n";
    let one = WarningsConfig {
        max_per_edit: 1,
        ..WarningsConfig::default()
    };
    assert_eq!(
        told(&store, edit, &history, &one)?.as_deref(),
        Some(tests_line)
    );
    let both = told(&store, edit, &history, &WarningsConfig::default())?.ok_or("nothing")?;
    assert_eq!(both, format!("{tests_line}{}", of_commit_2("The lines")));
    let (task, money) = (store.task, store.money);
    warned(
        &mut store.repo,
        task,
        (WarningKind::Origin, money, money),
        1_000,
    )?;
    assert_eq!(
        told(&store, edit, &history, &WarningsConfig::default())?.as_deref(),
        Some(tests_line)
    );
    Ok(())
}
