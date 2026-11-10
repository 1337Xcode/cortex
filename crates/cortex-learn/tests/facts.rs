//! The life of a fact on worked histories: said again on the same anchors in alike enough words it
//! is the same fact, a candidate turning active when the user says it or an agent says it in a
//! second task; a changed or gone anchor makes it suspect, 14 days suspect retire it, and saying it
//! against the code as it is now makes it active again. The user confirms, retires and corrects
//! facts. Every expected value is worked out by hand
//! in the comment above its test.

mod common;

use common::{Store, TestResult};
use cortex_core::{FactKind, FactSource, FactStatus, RepoPath, Ulid};
use cortex_learn::{
    Asserted, Assertion, FactError, Reviewed, assert_fact, confirm_fact, edit_fact, retire_fact,
    review_facts,
};
use cortex_store::{FactId, NodeId};

/// Fourteen days in milliseconds: 14 x 24 x 60 x 60 x 1,000.
const FOURTEEN_DAYS_MS: i64 = 1_209_600_000;

/// States `text` from `source` on `nodes` at `at_ms` in `store`, in the task made from `task` (none
/// when `None`), and commits it.
fn say(
    store: &mut Store,
    (text, source, task): (&str, FactSource, Option<u64>),
    nodes: &[NodeId],
    at_ms: i64,
) -> Result<Asserted, FactError> {
    let mut transaction = store.writer.transaction()?;
    let asserted = assert_fact(
        &mut transaction,
        &Assertion {
            text,
            kind: FactKind::Convention,
            source,
            task: task.map(|seed| Ulid::from_seed(seed, b"task")),
            nodes,
        },
        at_ms,
    )?;
    transaction.commit()?;
    Ok(asserted)
}

/// Reviews the facts of `store` at `at_ms`, and commits.
fn review(store: &mut Store, at_ms: i64) -> TestResult<Reviewed> {
    let mut transaction = store.writer.transaction()?;
    let reviewed = review_facts(&mut transaction, at_ms)?;
    transaction.commit()?;
    Ok(reviewed)
}

/// Fact `id`'s status and when it was set.
fn status(store: &Store, id: FactId) -> TestResult<(FactStatus, i64)> {
    let row = store.writer.queries().fact(id)?.ok_or("no such fact")?;
    Ok((row.record.status, row.status_at_ms))
}

/// The fact an assertion added.
fn added(asserted: Asserted) -> TestResult<FactId> {
    match asserted {
        Asserted::Added(id) => Ok(id),
        other => Err(format!("expected a new fact, got {other:?}").into()),
    }
}

/// An agent says "Money is kept in cents." on `money.py` in task 1 at 10: a candidate. It says
/// "money is kept in CENTS" in task 1 again at 20: the same words once case and marks are set
/// aside (likeness 1), the same task: merged, still a candidate from 10. It says "Money is always
/// kept in cents." in task 2 at 30: 5 words shared of 6 held (0.83, at least 0.8), a second task:
/// active from 30. One fact in all.
#[test]
fn a_candidate_said_in_a_second_task_turns_active() -> TestResult {
    let mut store = Store::new()?;
    let money = store.index_text(&[("src/money.py", "cents")])?;
    let agent = |text, task| (text, FactSource::Agent, Some(task));
    let fact = added(say(
        &mut store,
        agent("Money is kept in cents.", 1),
        &money,
        10,
    )?)?;
    assert_eq!(status(&store, fact)?, (FactStatus::Candidate, 10));
    let again = say(&mut store, agent("money is kept in CENTS", 1), &money, 20)?;
    assert_eq!(again, Asserted::Merged(fact));
    assert_eq!(status(&store, fact)?, (FactStatus::Candidate, 10));
    let second = say(
        &mut store,
        agent("Money is always kept in cents.", 2),
        &money,
        30,
    )?;
    assert_eq!(second, Asserted::Activated(fact));
    assert_eq!(status(&store, fact)?, (FactStatus::Active, 30));
    let queries = store.writer.queries();
    assert_eq!(queries.current_facts(FactStatus::Active)?.len(), 1);
    assert_eq!(queries.current_facts(FactStatus::Candidate)?.len(), 0);
    Ok(())
}

/// An import says "Tax rounds half up." on `tax.py` at 10: a candidate, asserted in no task. An
/// agent says it in task 1 at 20 (merged), the import again at 25 (merged), the agent in task 1 at
/// 30 and in task 2 at 40 (merged both times: an imported fact waits for a person), and the user at
/// 45: active. Another candidate, an agent's from task 3 on `money.py`, turns active the moment the
/// user says it, at 50.
#[test]
fn an_import_waits_for_the_user_and_the_user_confirms_at_once() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index_text(&[("src/tax.py", "half up"), ("src/money.py", "cents")])?;
    let (tax, money) = (&nodes[..1], &nodes[1..]);
    let rule = "Tax rounds half up.";
    let fact = added(say(&mut store, (rule, FactSource::Import, None), tax, 10)?)?;
    let steps = [
        ((FactSource::Agent, Some(1)), 20, Asserted::Merged(fact)),
        ((FactSource::Import, None), 25, Asserted::Merged(fact)),
        ((FactSource::Agent, Some(1)), 30, Asserted::Merged(fact)),
        ((FactSource::Agent, Some(2)), 40, Asserted::Merged(fact)),
        ((FactSource::User, None), 45, Asserted::Activated(fact)),
    ];
    for ((source, task), at_ms, expected) in steps {
        assert_eq!(say(&mut store, (rule, source, task), tax, at_ms)?, expected);
    }
    assert_eq!(status(&store, fact)?, (FactStatus::Active, 45));
    let cents = "Money is kept in cents.";
    let other = added(say(
        &mut store,
        (cents, FactSource::Agent, Some(3)),
        money,
        45,
    )?)?;
    let confirmed = say(&mut store, (cents, FactSource::User, None), money, 50)?;
    assert_eq!(confirmed, Asserted::Activated(other));
    assert_eq!(status(&store, other)?, (FactStatus::Active, 50));
    Ok(())
}

/// A fact retired here and imported again stays retired: the import is that fact, merged; so is an
/// agent saying it in task 1 and in task 2, which would confirm a candidate. Said again by the user,
/// the same words make a new fact, as any rule said after its retirement does.
#[test]
fn an_import_of_a_fact_retired_here_leaves_it_retired() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index_text(&[("src/tax.py", "half up")])?;
    let rule = "Tax rounds half up.";
    let fact = added(say(
        &mut store,
        (rule, FactSource::Import, None),
        &nodes,
        10,
    )?)?;
    let mut transaction = store.writer.transaction()?;
    transaction.set_fact_status(fact, FactStatus::Retired, 20)?;
    transaction.commit()?;
    for (source, task, at_ms) in [
        (FactSource::Import, None, 30),
        (FactSource::Agent, Some(1), 31),
        (FactSource::Agent, Some(2), 32),
    ] {
        assert_eq!(
            say(&mut store, (rule, source, task), &nodes, at_ms)?,
            Asserted::Merged(fact)
        );
    }
    assert_eq!(status(&store, fact)?, (FactStatus::Retired, 20));
    let again = say(&mut store, (rule, FactSource::User, None), &nodes, 40)?;
    assert!(
        matches!(again, Asserted::Added(new) if new != fact),
        "{again:?}"
    );
    Ok(())
}

/// The user's facts on `tax.py`. "Tax rounds half up" (a). An agent's "Tax rounds half up always"
/// in task 1: 4 words shared of 5 held, 0.8 exactly: a said again. "Tax rounds half": 3 of 4 with a, 0.75: a new fact
/// (b). "Tax rounds half up" on `tax.py` and `money.py`: other anchors, a new fact (c), and the
/// same said on the two files listed the other way round is c again. With a retired, "Tax rounds
/// half up" is a new fact (d): b holds 3 of its 4 words.
#[test]
fn facts_on_other_anchors_or_in_other_words_stay_apart() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index_text(&[("src/tax.py", "half up"), ("src/money.py", "cents")])?;
    let (tax, money) = (nodes[0], nodes[1]);
    let user = |text| (text, FactSource::User, None);
    let first = added(say(&mut store, user("Tax rounds half up"), &[tax], 1)?)?;
    let alike = say(
        &mut store,
        ("Tax rounds half up always", FactSource::Agent, Some(1)),
        &[tax],
        2,
    )?;
    assert_eq!(alike, Asserted::Merged(first));
    added(say(&mut store, user("Tax rounds half"), &[tax], 3)?)?;
    let both = added(say(
        &mut store,
        user("Tax rounds half up"),
        &[tax, money],
        4,
    )?)?;
    let reversed = say(&mut store, user("Tax rounds half up"), &[money, tax], 5)?;
    assert_eq!(reversed, Asserted::Merged(both));
    let mut transaction = store.writer.transaction()?;
    transaction.set_fact_status(first, FactStatus::Retired, 6)?;
    transaction.commit()?;
    let after_retiring = say(&mut store, user("Tax rounds half up"), &[tax], 7)?;
    assert!(matches!(after_retiring, Asserted::Added(id) if id != first));
    Ok(())
}

/// `money.py` holds "v1", `tax.py` "t1". The user's fact f on `money.py` and an agent's candidate
/// g on `tax.py`, both at 10. A review at 20 changes nothing. `money.py` becomes "v2": a review at
/// 100 makes f suspect from 100. Reviews at 100 plus 14 days less 1 ms retire nothing; at 100 plus
/// 14 days, f retires. `tax.py` goes: a review at 200 makes g suspect.
#[test]
fn a_changed_anchor_makes_a_fact_suspect_and_14_days_retire_it() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index_text(&[("src/money.py", "v1"), ("src/tax.py", "t1")])?;
    let (money, tax) = (&nodes[..1], &nodes[1..]);
    let cents = ("Money is kept in cents.", FactSource::User, None);
    let fact = added(say(&mut store, cents, money, 10)?)?;
    let half_up = ("Tax rounds half up.", FactSource::Agent, Some(1));
    let candidate = added(say(&mut store, half_up, tax, 10)?)?;
    assert_eq!(review(&mut store, 20)?, Reviewed::default());
    let changed = store.index_text(&[("src/money.py", "v2")])?;
    assert_eq!(changed, money, "the file keeps its node");
    let suspected = review(&mut store, 100)?;
    assert_eq!(suspected.suspected, [fact]);
    assert_eq!(status(&store, fact)?, (FactStatus::Suspect, 100));
    let day_short = review(&mut store, 100 + FOURTEEN_DAYS_MS - 1)?;
    assert_eq!(day_short, Reviewed::default());
    let due = review(&mut store, 100 + FOURTEEN_DAYS_MS)?;
    assert_eq!((due.suspected, due.retired), (Vec::new(), vec![fact]));
    assert_eq!(
        status(&store, fact)?,
        (FactStatus::Retired, 100 + FOURTEEN_DAYS_MS)
    );
    assert_eq!(status(&store, candidate)?, (FactStatus::Candidate, 10));
    let mut transaction = store.writer.transaction()?;
    transaction.remove_file(&RepoPath::new("src/tax.py")?)?;
    transaction.commit()?;
    assert_eq!(review(&mut store, 200)?.suspected, [candidate]);
    Ok(())
}

/// The user's fact on `money.py` ("v1") at 10; `money.py` becomes "v2" and a review at 20 makes it
/// suspect. An import saying it at 25 changes nothing; an agent saying it in task 1 at 30 confirms
/// it against the code now: active from 30, and a review at 40 finds nothing changed.
#[test]
fn a_suspect_fact_said_against_the_code_now_is_active_again() -> TestResult {
    let mut store = Store::new()?;
    let money = store.index_text(&[("src/money.py", "v1")])?;
    let text = "Money is kept in cents.";
    let fact = added(say(&mut store, (text, FactSource::User, None), &money, 10)?)?;
    store.index_text(&[("src/money.py", "v2")])?;
    assert_eq!(review(&mut store, 20)?.suspected, [fact]);
    let imported = say(&mut store, (text, FactSource::Import, None), &money, 25)?;
    assert_eq!(imported, Asserted::Merged(fact));
    assert_eq!(status(&store, fact)?, (FactStatus::Suspect, 20));
    let confirmed = say(&mut store, (text, FactSource::Agent, Some(1)), &money, 30)?;
    assert_eq!(confirmed, Asserted::Activated(fact));
    assert_eq!(status(&store, fact)?, (FactStatus::Active, 30));
    assert_eq!(review(&mut store, 40)?, Reviewed::default());
    Ok(())
}

/// An agent's candidate on `money.py` ("v1") from task 1 at 10. The agent changes `money.py` to
/// "v2" and a review at 20 makes the candidate suspect. Said again in task 1 at 30, it stays
/// suspect: an agent cannot confirm its own rule by changing the code it is about. Said in task 2
/// at 40, it turns active, as a candidate would.
#[test]
fn a_suspect_candidate_turns_active_only_as_a_candidate_would() -> TestResult {
    let mut store = Store::new()?;
    let money = store.index_text(&[("src/money.py", "v1")])?;
    let rule = |task| ("Money is kept in cents.", FactSource::Agent, Some(task));
    let fact = added(say(&mut store, rule(1), &money, 10)?)?;
    store.index_text(&[("src/money.py", "v2")])?;
    assert_eq!(review(&mut store, 20)?.suspected, [fact]);
    assert_eq!(
        say(&mut store, rule(1), &money, 30)?,
        Asserted::Merged(fact)
    );
    assert_eq!(status(&store, fact)?, (FactStatus::Suspect, 20));
    assert_eq!(
        say(&mut store, rule(2), &money, 40)?,
        Asserted::Activated(fact)
    );
    assert_eq!(status(&store, fact)?, (FactStatus::Active, 40));
    Ok(())
}

/// Facts about the whole repository are anchored to its root alone, at most ten not retired.
/// Ten rules noted: each added. An eleventh: refused. The first said again: merged, not refused.
/// The first retired: the eleventh is added. The root named with `money.py`: refused.
#[test]
fn facts_about_the_whole_repository_are_capped_at_ten() -> TestResult {
    let mut store = Store::new()?;
    let money = store.index_text(&[("src/money.py", "cents")])?;
    let root = store.folder_node("")?;
    let user = |text: &'static str| (text, FactSource::User, None);
    let rules = [
        "Every module has tests.",
        "Errors carry the operation.",
        "Times are UTC milliseconds.",
        "Paths use forward slashes.",
        "No network calls in tests.",
        "Configs are written atomically.",
        "Logs never hold code.",
        "Hashes are BLAKE3.",
        "Each crate has a README.",
        "Commits are atomic.",
    ];
    let mut ids = Vec::new();
    for (at, rule) in rules.iter().enumerate() {
        ids.push(added(say(
            &mut store,
            user(rule),
            &[root],
            10 + i64::try_from(at)?,
        )?)?);
    }
    let eleventh = user("Docs describe what was run.");
    assert!(matches!(
        say(&mut store, eleventh, &[root], 30),
        Err(FactError::TooManyGlobal)
    ));
    assert_eq!(
        say(&mut store, user(rules[0]), &[root], 31)?,
        Asserted::Merged(ids[0])
    );
    word(&mut store, |transaction| {
        retire_fact(transaction, ids[0], 32)
    })?;
    added(say(&mut store, eleventh, &[root], 33)?)?;
    assert!(matches!(
        say(
            &mut store,
            user("Money is kept in cents."),
            &[root, money[0]],
            34
        ),
        Err(FactError::GlobalWithOthers)
    ));
    Ok(())
}

/// A file whose contents are not read and a folder have no hash to keep: a fact on either is
/// refused, naming the node, and nothing is stored.
#[test]
fn a_fact_on_what_has_no_hash_is_refused() -> TestResult {
    let mut store = Store::new()?;
    let unread = store.index(&["assets/logo.bin"])?;
    let folder = store.folder_node("src")?;
    let read = store.index_text(&[("src/money.py", "cents")])?;
    for node in [unread[0], folder] {
        let refused = say(
            &mut store,
            ("Money is kept in cents.", FactSource::User, None),
            &[read[0], node],
            1,
        );
        assert!(
            matches!(refused, Err(FactError::Unanchorable { node: refused }) if refused == node)
        );
    }
    let queries = store.writer.queries();
    assert_eq!(queries.current_facts(FactStatus::Active)?.len(), 0);
    Ok(())
}

/// The user's word. An agent's candidate on `money.py` ("v1") from 10, confirmed at 20: active
/// from 20. `money.py` becomes "v2" and a review at 30 makes it suspect; confirmed at 40 it is
/// active from 40 with its anchor stamped anew, so a review at 50 finds nothing. The user's pitfall
/// on `tax.py` from 10, corrected at 60: a new active pitfall from 60 on `tax.py` with the new
/// text, which supersedes it; the old one is no longer current and cannot be confirmed, retired or
/// corrected again. `tax.py` goes: the new fact cannot be confirmed or corrected (its anchor has no
/// hash now), and is retired at 70.
#[test]
fn the_user_confirms_retires_and_corrects_facts() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index_text(&[("src/money.py", "v1"), ("src/tax.py", "t1")])?;
    let (money, tax) = (&nodes[..1], &nodes[1..]);
    let cents = ("Money is kept in cents.", FactSource::Agent, Some(1));
    let candidate = added(say(&mut store, cents, money, 10)?)?;
    word(&mut store, |transaction| {
        confirm_fact(transaction, candidate, 20)
    })?;
    assert_eq!(status(&store, candidate)?, (FactStatus::Active, 20));
    store.index_text(&[("src/money.py", "v2")])?;
    assert_eq!(review(&mut store, 30)?.suspected, [candidate]);
    word(&mut store, |transaction| {
        confirm_fact(transaction, candidate, 40)
    })?;
    assert_eq!(status(&store, candidate)?, (FactStatus::Active, 40));
    assert_eq!(review(&mut store, 50)?, Reviewed::default());

    let mut transaction = store.writer.transaction()?;
    let pitfall = assert_fact(
        &mut transaction,
        &Assertion {
            text: "Tax rounds half up.",
            kind: FactKind::Pitfall,
            source: FactSource::User,
            task: None,
            nodes: tax,
        },
        10,
    )?;
    transaction.commit()?;
    let old = added(pitfall)?;
    let new = word(&mut store, |transaction| {
        edit_fact(transaction, old, "Tax rounds half to even.", 60)
    })?;
    let row = store.writer.queries().fact(new)?.ok_or("no new fact")?;
    assert_eq!(
        (row.record.text.as_str(), row.record.kind, row.record.source),
        (
            "Tax rounds half to even.",
            FactKind::Pitfall,
            FactSource::User
        )
    );
    assert_eq!(status(&store, new)?, (FactStatus::Active, 60));
    let anchored: Vec<NodeId> = row
        .record
        .anchors
        .iter()
        .map(|anchor| anchor.node)
        .collect();
    assert_eq!(anchored, tax);
    let old_row = store.writer.queries().fact(old)?.ok_or("no old fact")?;
    assert_eq!(old_row.superseded_by, Some(new));
    let superseded = |error: FactError| matches!(error, FactError::Superseded { fact, by } if fact == old && by == new);
    let mut transaction = store.writer.transaction()?;
    assert!(confirm_fact(&mut transaction, old, 61).is_err_and(superseded));
    assert!(retire_fact(&mut transaction, old, 61).is_err_and(superseded));
    assert!(edit_fact(&mut transaction, old, "Tax is exact.", 61).is_err_and(superseded));
    transaction.commit()?;

    let mut transaction = store.writer.transaction()?;
    transaction.remove_file(&RepoPath::new("src/tax.py")?)?;
    transaction.commit()?;
    let gone =
        |error: FactError| matches!(error, FactError::Unanchorable { node } if node == tax[0]);
    let mut transaction = store.writer.transaction()?;
    assert!(confirm_fact(&mut transaction, new, 65).is_err_and(gone));
    assert!(edit_fact(&mut transaction, new, "Tax is exact.", 65).is_err_and(gone));
    retire_fact(&mut transaction, new, 70)?;
    transaction.commit()?;
    assert_eq!(status(&store, new)?, (FactStatus::Retired, 70));
    Ok(())
}

/// Carries out `change` in a transaction of `store`'s, and commits it.
fn word<T>(
    store: &mut Store,
    change: impl FnOnce(&mut cortex_store::WriteTx<'_>) -> Result<T, FactError>,
) -> TestResult<T> {
    let mut transaction = store.writer.transaction()?;
    let done = change(&mut transaction)?;
    transaction.commit()?;
    Ok(done)
}

/// The text and the fact correcting fact `id`, if any.
fn text_of(store: &Store, id: FactId) -> TestResult<(String, Option<FactId>)> {
    let row = store.writer.queries().fact(id)?.ok_or("no such fact")?;
    Ok((row.record.text, row.superseded_by))
}

/// A note that negates or renumbers a rule. Worked by hand on `writer.py` and `client.py`:
/// - An agent's "Always call flush before close in the writer module" in task 1 (a). Another
///   agent's "Never call flush before close in the writer module" in task 2: 8 words shared of 10
///   held, 0.8, but "always" against "never": another rule (b), and a stays a candidate (said in a
///   second task, it would have turned active).
/// - An agent's "Always retry a request after a timeout in the client" in task 3 (c). The user's
///   "Never retry ...": 8 of 10, 0.8; the user's words correct c: a new active fact (u) supersedes
///   it.
/// - The user's "The client gives up on a request after 30 seconds of silence" (r1); then "... after
///   60 seconds of silence": 11 of 13, 0.85; the user's words correct r1 (r2 supersedes it). An
///   agent's "... after 90 seconds of silence" in task 4: 11 of 13 with r2, but 90 against 60:
///   another rule, a candidate (s), and r2 stays as the user said it.
#[test]
fn a_note_that_negates_or_renumbers_a_rule_is_another_rule_or_corrects_it() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index_text(&[("src/writer.py", "flush"), ("src/client.py", "retry")])?;
    let (writer, client) = (&nodes[..1], &nodes[1..]);
    let agent = |text, task| (text, FactSource::Agent, Some(task));
    let user = |text| (text, FactSource::User, None);
    let always = "Always call flush before close in the writer module";
    let never = "Never call flush before close in the writer module";
    let a = added(say(&mut store, agent(always, 1), writer, 10)?)?;
    let b = added(say(&mut store, agent(never, 2), writer, 20)?)?;
    assert_ne!(a, b);
    assert_eq!(status(&store, a)?, (FactStatus::Candidate, 10));
    assert_eq!(status(&store, b)?, (FactStatus::Candidate, 20));

    let retry = "Always retry a request after a timeout in the client";
    let no_retry = "Never retry a request after a timeout in the client";
    let c = added(say(&mut store, agent(retry, 3), client, 30)?)?;
    let u = added(say(&mut store, user(no_retry), client, 40)?)?;
    assert_eq!(text_of(&store, c)?, (retry.to_owned(), Some(u)));
    assert_eq!(text_of(&store, u)?, (no_retry.to_owned(), None));
    assert_eq!(status(&store, u)?, (FactStatus::Active, 40));

    let thirty = "The client gives up on a request after 30 seconds of silence";
    let sixty = "The client gives up on a request after 60 seconds of silence";
    let ninety = "The client gives up on a request after 90 seconds of silence";
    let r1 = added(say(&mut store, user(thirty), client, 50)?)?;
    let r2 = added(say(&mut store, user(sixty), client, 60)?)?;
    assert_eq!(text_of(&store, r1)?, (thirty.to_owned(), Some(r2)));
    assert_eq!(status(&store, r2)?, (FactStatus::Active, 60));
    let s = added(say(&mut store, agent(ninety, 4), client, 70)?)?;
    assert_eq!(status(&store, s)?, (FactStatus::Candidate, 70));
    assert_eq!(text_of(&store, r2)?, (sixty.to_owned(), None));
    Ok(())
}

/// Candidates leave the user's slots for rules about the whole repository free. Agents note ten
/// such rules in task 1: candidates, each added; an eleventh of an agent's is refused (ten not
/// retired). The user's own rule is added: no fact that is not a candidate holds a slot yet.
#[test]
fn candidates_leave_the_user_s_global_slots_free() -> TestResult {
    let mut store = Store::new()?;
    store.index_text(&[("src/money.py", "cents")])?;
    let root = store.folder_node("")?;
    for at in 0..10 {
        let rule = format!("Every module {at} has tests.");
        added(say(
            &mut store,
            (rule.as_str(), FactSource::Agent, Some(1)),
            &[root],
            10 + at,
        )?)?;
    }
    assert!(matches!(
        say(
            &mut store,
            ("Errors carry the operation.", FactSource::Agent, Some(1)),
            &[root],
            30
        ),
        Err(FactError::TooManyGlobal)
    ));
    let mine = added(say(
        &mut store,
        ("Logs never hold code.", FactSource::User, None),
        &[root],
        31,
    )?)?;
    assert_eq!(status(&store, mine)?, (FactStatus::Active, 31));
    Ok(())
}

/// A fact names at most 16 files and symbols: noted on 17 files it is refused, naming 17; on 16 it
/// is added.
#[test]
fn a_fact_about_more_than_16_places_is_refused() -> TestResult {
    let mut store = Store::new()?;
    let names: Vec<String> = (0..17).map(|at| format!("src/f{at:02}.py")).collect();
    let files: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "x")).collect();
    let nodes = store.index_text(&files)?;
    let user = ("Each module has a test.", FactSource::User, None);
    assert!(matches!(
        say(&mut store, user, &nodes, 10),
        Err(FactError::TooManyAnchors { count: 17 })
    ));
    added(say(&mut store, user, &nodes[..16], 11)?)?;
    Ok(())
}
