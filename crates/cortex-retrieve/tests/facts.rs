//! The project facts a work order carries: the active facts about its files, ranked by how close
//! they are to the code it shows, written only while true of the files on disk, at most five, and
//! shed after the code and before the items when the work order is over its cap. Every expected
//! value is worked out by hand in the comment above its test.

mod common;

use common::{Repo, TestResult, function};
use cortex_core::{
    ContentHash, FactKind, FactSource, FactStatus, FileFlags, Identifier, LineRange, RepoPath,
    ShortHash,
};
use cortex_retrieve::{
    Basis, FactAt, Item, ItemSpan, Limits, ListedFile, OrderFact, Reason, WorkOrder, order_facts,
    render,
};
use cortex_store::{FactId, FactRecord, NodeId};

/// `a.py`: `f` on lines 1 and 2, `g` on lines 5 and 6.
const A: &str = "def f():\n    return 1\n\n\ndef g():\n    return 2\n";

/// `b.py`: `h` on lines 1 and 2.
const B: &str = "def h():\n    return 3\n";

/// Adds to `repo` a fact of the user's saying `text`, in `status`, anchored to `nodes` with the
/// hashes they hold now, or `kept` in their place when given.
fn fact(
    repo: &mut Repo,
    text: &str,
    (status, kept): (FactStatus, Option<&[u8]>),
    nodes: &[NodeId],
) -> TestResult<FactId> {
    let mut transaction = repo.writer.transaction()?;
    let mut anchors = transaction.queries().anchor_hashes(nodes)?;
    if let Some(kept) = kept {
        for anchor in &mut anchors {
            anchor.hash = ShortHash::of(kept);
        }
    }
    let id = transaction.add_fact(&FactRecord {
        text: text.to_owned(),
        kind: FactKind::Convention,
        source: FactSource::User,
        status,
        created_at_ms: 1,
        task: None,
        anchors,
    })?;
    transaction.commit()?;
    Ok(id)
}

/// Worked by hand. `a.py` (with `f` and `g`) is listed first, its span on `f`; `b.py` (with `h`)
/// second; `c.py` is not listed. Active facts: 1 on `a.py`; 2 on `g`; 3 on `f`; 4 on `h`, given 3
/// times; 5 on `b.py`; 6 on `f`, given twice; 7 on `c.py`; 8 on `a.py` with a hash `a.py` no longer
/// holds. Fact 9 on `a.py` is suspect.
///
/// Closest first: on the shown `f`, 6 (given more) then 3; on other symbols, 2 (`a.py`, listed
/// first) then 4 (`b.py`); on the files, 1 and 8 (`a.py`, the older first) then 5. Fact 7 is about
/// no listed file and fact 9 is not active. Every anchor is current but fact 8's.
#[test]
fn the_facts_about_the_listed_files_rank_by_closeness() -> TestResult {
    let mut repo = Repo::new()?;
    let a = repo.file(
        "a.py",
        A,
        &[function("f", 1, 2), function("g", 5, 6)],
        FileFlags::empty(),
    )?;
    let b = repo.file("b.py", B, &[function("h", 1, 2)], FileFlags::empty())?;
    let c = repo.file("c.py", "x = 1\n", &[], FileFlags::empty())?;
    let (a_file, f, g, b_file, h) = (a[0], a[1], a[2], b[0], b[1]);
    let active = (FactStatus::Active, None);
    let ids = [
        fact(&mut repo, "A is about a.", active, &[a_file])?,
        fact(&mut repo, "G returns two.", active, &[g])?,
        fact(&mut repo, "F returns one.", active, &[f])?,
        fact(&mut repo, "H returns three.", active, &[h])?,
        fact(&mut repo, "B is small.", active, &[b_file])?,
        fact(&mut repo, "F is first.", active, &[f])?,
        fact(&mut repo, "C is elsewhere.", active, &[c[0]])?,
        fact(
            &mut repo,
            "A was once.",
            (FactStatus::Active, Some(b"old")),
            &[a_file],
        )?,
        fact(
            &mut repo,
            "A is suspect.",
            (FactStatus::Suspect, None),
            &[a_file],
        )?,
    ];
    let mut transaction = repo.writer.transaction()?;
    for given in [&[ids[3], ids[5]][..], &[ids[3], ids[5]], &[ids[3]]] {
        transaction.count_fact_uses(given)?;
    }
    transaction.commit()?;
    let files = [
        ListedFile {
            path: RepoPath::new("a.py")?,
            node: a_file,
            shown: vec![f],
        },
        ListedFile {
            path: RepoPath::new("b.py")?,
            node: b_file,
            shown: Vec::new(),
        },
    ];
    let reader = repo.reader()?;
    let facts = order_facts(reader.queries(), &files)?;
    let ranked: Vec<FactId> = facts.iter().map(|fact| fact.id).collect();
    let expected: Vec<FactId> = [5, 2, 1, 3, 0, 7, 4].iter().map(|at| ids[*at]).collect();
    assert_eq!(ranked, expected);
    let first = facts.first().ok_or("a fact")?;
    assert_eq!(
        (first.text.as_str(), first.anchors.as_slice()),
        (
            "F is first.",
            &[FactAt {
                name: "a.py:f".to_owned(),
                path: RepoPath::new("a.py")?,
                content: Some(ContentHash::of(A.as_bytes())),
                is_current: true,
            }][..]
        )
    );
    let current: Vec<bool> = facts
        .iter()
        .map(|fact| fact.anchors.iter().all(|anchor| anchor.is_current))
        .collect();
    assert_eq!(current, [true, true, true, true, true, false, true]);
    Ok(())
}

/// The settings' limits, with one code body shown and `max_tokens` tokens.
fn limits(max_tokens: u32) -> Limits {
    Limits {
        max_tokens,
        max_files: 5,
        max_code_spans: 1,
        span_max_lines: 60,
        max_file_bytes: 1_048_576,
    }
}

/// A work order listing `a.py` with its span on `f`, carrying `facts`.
fn order(facts: Vec<OrderFact>) -> TestResult<WorkOrder> {
    Ok(WorkOrder {
        basis: Basis::Agreement(vec!["the prompt's words", "the commit history"]),
        items: vec![Item {
            path: RepoPath::new("a.py")?,
            content: Some(ContentHash::of(A.as_bytes())),
            spans: vec![ItemSpan {
                lines: LineRange::new(1, 2)?,
                name: Some(Identifier::new("f")?),
                scope: None,
            }],
            reason: Reason::Matches(vec!["f".to_owned()]),
        }],
        facts,
    })
}

/// A fact `id` saying `text`, anchored at `name` in `path`, whose file the index read as `indexed`,
/// current or not.
fn order_fact(
    id: FactId,
    text: &str,
    (name, path, indexed): (&str, &str, &str),
    is_current: bool,
) -> TestResult<OrderFact> {
    Ok(OrderFact {
        id,
        text: text.to_owned(),
        anchors: vec![FactAt {
            name: name.to_owned(),
            path: RepoPath::new(path)?,
            content: Some(ContentHash::of(indexed.as_bytes())),
            is_current,
        }],
    })
}

/// Ids for hand-built facts: `count` facts stored on `a.py`'s node.
fn ids(repo: &mut Repo, node: NodeId, count: usize) -> TestResult<Vec<FactId>> {
    (0..count)
        .map(|at| {
            fact(
                repo,
                &format!("Fact {at}."),
                (FactStatus::Active, None),
                &[node],
            )
        })
        .collect()
}

/// Of five facts, two are true now and written, in order, under the code: one on `a.py:f` as
/// indexed, one on `b.py` as indexed. Withheld: a fact whose anchor's hash is not the one kept, one
/// whose file changed on disk since it was indexed (`b.py` indexed as "x = 1"), one whose text
/// breaks a line. With seven true facts, the first five are written.
#[test]
fn a_work_order_writes_only_the_facts_true_now() -> TestResult {
    let mut repo = Repo::new()?;
    let a = repo.file("a.py", A, &[function("f", 1, 2)], FileFlags::empty())?;
    repo.file("b.py", B, &[], FileFlags::empty())?;
    let ids = ids(&mut repo, a[0], 7)?;
    let facts = vec![
        order_fact(ids[0], "F returns one.", ("a.py:f", "a.py", A), true)?,
        order_fact(ids[1], "F returned two.", ("a.py:f", "a.py", A), false)?,
        order_fact(ids[2], "B was one.", ("b.py", "b.py", "x = 1\n"), true)?,
        order_fact(ids[3], "B is\nsmall.", ("b.py", "b.py", B), true)?,
        order_fact(ids[4], "B is small.", ("b.py", "b.py", B), true)?,
    ];
    let rendered = render(&order(facts)?, repo.root.path(), &limits(900)).ok_or("a work order")?;
    let expected = "\
[cortex] Work order · the prompt's words and the commit history agree
Start here:
  1. a.py:1-2  f  (matches f)
Current code:
  a.py:1-2
  1 | def f():
  2 |     return 1
Project facts:
  - F returns one. (anchored to a.py:f)
  - B is small. (anchored to b.py)
";
    assert_eq!(rendered.text, expected);
    assert_eq!(rendered.facts, [ids[0], ids[4]]);

    let seven: Vec<OrderFact> = ids
        .iter()
        .enumerate()
        .map(|(at, id)| order_fact(*id, &format!("Fact {at}."), ("b.py", "b.py", B), true))
        .collect::<TestResult<_>>()?;
    let rendered = render(&order(seven)?, repo.root.path(), &limits(900)).ok_or("a work order")?;
    assert_eq!(rendered.facts, ids[..5]);
    assert_eq!(rendered.text.matches("\n  - Fact ").count(), 5);
    Ok(())
}

/// Over its cap, a work order sheds its code first, then its facts from the last, and only then its
/// items. One token short of the whole text: the code is taken out and both facts stay. One token
/// short of that: the last fact goes too, and `a.py` stays listed.
#[test]
fn facts_are_shed_after_the_code_and_before_the_items() -> TestResult {
    let mut repo = Repo::new()?;
    let a = repo.file("a.py", A, &[function("f", 1, 2)], FileFlags::empty())?;
    repo.file("b.py", B, &[], FileFlags::empty())?;
    let ids = ids(&mut repo, a[0], 2)?;
    let facts = || -> TestResult<Vec<OrderFact>> {
        Ok(vec![
            order_fact(ids[0], "F returns one.", ("a.py:f", "a.py", A), true)?,
            order_fact(ids[1], "B is small.", ("b.py", "b.py", B), true)?,
        ])
    };
    let root = repo.root.path();
    let whole = render(&order(facts()?)?, root, &limits(900)).ok_or("a work order")?;
    let no_code =
        render(&order(facts()?)?, root, &limits(whole.tokens - 1)).ok_or("a work order")?;
    assert!(!no_code.text.contains("Current code:"), "{}", no_code.text);
    assert_eq!(no_code.facts, ids);
    let one_fact =
        render(&order(facts()?)?, root, &limits(no_code.tokens - 1)).ok_or("a work order")?;
    assert_eq!(one_fact.facts, ids[..1]);
    assert!(one_fact.text.contains("a.py:1-2"), "{}", one_fact.text);
    Ok(())
}
