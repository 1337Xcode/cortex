//! Injections recorded and read back: what a prompt's retrieval gave or weighed and what a primer
//! gave, keeping node ids, line ranges, the kinds of reasons and the gate's numbers, and nothing a
//! prompt or a file says.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{
    Agent, ContentHash, GateFeatures, HookKind, Identifier, ReasonKind, ShortHash, Ulid,
};
use cortex_store::{InjectedFile, InjectionRecord, NodeId, SessionRecord, TaskRecord, Writer};

/// Stores files at `paths` and returns their nodes, in that order.
fn nodes(writer: &mut Writer, paths: &[&str]) -> TestResult<Vec<NodeId>> {
    let mut transaction = writer.transaction()?;
    let mut nodes = Vec::new();
    for path in paths {
        let change = transaction.replace_file(&index(path, Vec::new(), Vec::new())?)?;
        nodes.push(change.file_node);
    }
    transaction.commit()?;
    Ok(nodes)
}

/// Records session `session` and, in it, the tasks `tasks`, so that injections may belong to them.
fn session_with(writer: &mut Writer, session: &Identifier, tasks: &[Ulid]) -> TestResult {
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: session.clone(),
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 500,
    })?;
    for (at, task) in tasks.iter().enumerate() {
        transaction.start_task(&TaskRecord {
            task_id: *task,
            session_id: session.clone(),
            started_at_ms: 1_500 + i64::try_from(at)?,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: Some(false),
        })?;
    }
    transaction.commit()?;
    Ok(())
}

/// The gate's numbers of one retrieval, as typed here.
fn features() -> GateFeatures {
    GateFeatures {
        top_lexical: 1.0,
        top_mass: 0.625,
        agreeing_kinds: 2,
        supporting_kinds: 3,
        similar_task: 0.0,
        recent_precision: None,
        is_continuing: false,
    }
}

/// A prompt's work order of two files, and a primer, recorded and read back as they were
/// recorded, by task and by session; the task's injection unlabelled until its task ends.
#[test]
fn injections_are_read_back_as_recorded() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let [service, model] = nodes(&mut writer, &["src/service.py", "src/model.py"])?[..] else {
        return Err("two nodes".into());
    };
    let session = Identifier::new("s1")?;
    let task = Ulid::parse("01JC0000000000000000000000")?;
    session_with(&mut writer, &session, &[task])?;
    let order = InjectionRecord {
        task_id: Some(task),
        session_id: session.clone(),
        hook: HookKind::Prompt,
        at_ms: 2_000,
        files: vec![
            InjectedFile {
                node: service,
                spans: vec![(1, 4)],
                reason: Some(ReasonKind::Named),
                shown: vec![(1, 4)],
                content: Some(ShortHash::of(b"the service")),
            },
            InjectedFile {
                node: model,
                spans: vec![(1, 3), (10, 12)],
                reason: Some(ReasonKind::ChangesWith),
                shown: Vec::new(),
                content: None,
            },
        ],
        tokens: 182,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the work order")),
        features: Some(features()),
    };
    let primer = InjectionRecord {
        task_id: None,
        session_id: session.clone(),
        hook: HookKind::SessionStart,
        at_ms: 1_000,
        files: Vec::new(),
        tokens: 40,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the primer")),
        features: None,
    };
    let mut transaction = writer.transaction()?;
    let primer_id = transaction.record_injection(&primer)?;
    let order_id = transaction.record_injection(&order)?;
    transaction.commit()?;
    let queries = writer.queries();
    let of_task = queries.injections_of_task(task)?;
    let [row] = &of_task[..] else {
        return Err(format!("{of_task:?}").into());
    };
    assert_eq!(row.id, order_id);
    assert_eq!(row.record, order);
    assert_eq!((row.used_count, row.label), (None, None));
    let of_session: Vec<_> = queries
        .injections_of_session(&session)?
        .into_iter()
        .map(|row| (row.id, row.record))
        .collect();
    assert_eq!(of_session, [(primer_id, primer), (order_id, order)]);
    Ok(())
}

/// A retrieval the gate kept back is recorded with the files it weighed, with no span, no reason
/// and no text; a held-out task's, which the gate let through, with what would have been given.
/// The two are told apart.
#[test]
fn what_was_kept_back_is_recorded_too() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let [tax, service] = nodes(&mut writer, &["src/tax.py", "src/service.py"])?[..] else {
        return Err("two nodes".into());
    };
    let (gated, held) = (
        Ulid::parse("01JC0000000000000000000001")?,
        Ulid::parse("01JC0000000000000000000004")?,
    );
    let session = Identifier::new("s2")?;
    session_with(&mut writer, &session, &[gated, held])?;
    let kept_back = InjectionRecord {
        task_id: Some(gated),
        session_id: session.clone(),
        hook: HookKind::Prompt,
        at_ms: 3_000,
        files: vec![InjectedFile {
            node: tax,
            spans: Vec::new(),
            reason: None,
            shown: Vec::new(),
            content: None,
        }],
        tokens: 0,
        confidence: None,
        is_gated_out: true,
        is_holdout: false,
        text_hash: None,
        features: Some(features()),
    };
    let held_out = InjectionRecord {
        task_id: Some(held),
        session_id: session,
        hook: HookKind::Prompt,
        at_ms: 3_500,
        files: vec![InjectedFile {
            node: service,
            spans: vec![(1, 4)],
            reason: Some(ReasonKind::Named),
            shown: Vec::new(),
            content: None,
        }],
        tokens: 120,
        confidence: None,
        is_gated_out: false,
        is_holdout: true,
        text_hash: Some(ContentHash::of(b"what would have been given")),
        features: Some(features()),
    };
    let mut transaction = writer.transaction()?;
    transaction.record_injection(&kept_back)?;
    transaction.record_injection(&held_out)?;
    transaction.commit()?;
    let queries = writer.queries();
    let records = |task| -> TestResult<Vec<InjectionRecord>> {
        Ok(queries
            .injections_of_task(task)?
            .into_iter()
            .map(|row| row.record)
            .collect())
    };
    assert_eq!(records(gated)?, [kept_back]);
    assert_eq!(records(held)?, [held_out]);
    Ok(())
}

/// The items are stored as typed here, node ids, line ranges and the kind of each reason, and
/// nothing else: no word of the prompt and no name of a file.
#[test]
fn an_injection_holds_no_words() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let [service] = nodes(&mut writer, &["src/service.py"])?[..] else {
        return Err("one node".into());
    };
    let task = Ulid::parse("01JC0000000000000000000002")?;
    let session = Identifier::new("s3")?;
    session_with(&mut writer, &session, &[task])?;
    let mut transaction = writer.transaction()?;
    transaction.record_injection(&InjectionRecord {
        task_id: Some(task),
        session_id: session,
        hook: HookKind::Prompt,
        at_ms: 4_000,
        files: vec![InjectedFile {
            node: service,
            spans: vec![(1, 4)],
            reason: Some(ReasonKind::Matches),
            shown: Vec::new(),
            content: None,
        }],
        tokens: 90,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"matches order, orders")),
        features: Some(features()),
    })?;
    transaction.commit()?;
    drop(writer);
    let connection = rusqlite::Connection::open(scratch.folder.join("cortex.db"))?;
    let (items, features): (String, String) =
        connection.query_row("SELECT items, features FROM injections", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    assert_eq!(
        items,
        format!(
            r#"[{{"node":{},"spans":[[1,4]],"reason":"matches"}}]"#,
            service.get()
        )
    );
    assert_eq!(
        features,
        r#"{"top_lexical":1.0,"top_mass":0.625,"agreeing_kinds":2,"supporting_kinds":3,"similar_task":0.0,"recent_precision":null,"is_continuing":false}"#
    );
    Ok(())
}

/// An injection belongs to a stored session and, for a prompt's, a stored task: one recorded
/// before its task is refused, and nothing of it is kept.
#[test]
fn an_injection_needs_its_task() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let session = Identifier::new("s4")?;
    session_with(&mut writer, &session, &[])?;
    let task = Ulid::parse("01JC0000000000000000000003")?;
    let mut transaction = writer.transaction()?;
    let refused = transaction.record_injection(&InjectionRecord {
        task_id: Some(task),
        session_id: session.clone(),
        hook: HookKind::Prompt,
        at_ms: 5_000,
        files: Vec::new(),
        tokens: 0,
        confidence: None,
        is_gated_out: true,
        is_holdout: false,
        text_hash: None,
        features: Some(features()),
    });
    assert!(refused.is_err());
    transaction.commit()?;
    assert!(writer.queries().injections_of_session(&session)?.is_empty());
    Ok(())
}

/// What a task's end found is kept with its injection: labelled with two files used and one of
/// the first three among them, then labelled again with none, which replaces both. A row the
/// store does not hold is said to be missing, and nothing is written.
#[test]
fn a_label_is_kept_with_its_injection() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let [service] = nodes(&mut writer, &["src/service.py"])?[..] else {
        return Err("one node".into());
    };
    let session = Identifier::new("s1")?;
    let task = Ulid::parse("01JC0000000000000000000000")?;
    session_with(&mut writer, &session, &[task])?;
    let order = InjectionRecord {
        task_id: Some(task),
        session_id: session,
        hook: HookKind::Prompt,
        at_ms: 2_000,
        files: vec![InjectedFile {
            node: service,
            spans: vec![(1, 4)],
            reason: Some(ReasonKind::Named),
            shown: Vec::new(),
            content: None,
        }],
        tokens: 60,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the work order")),
        features: Some(features()),
    };
    let mut transaction = writer.transaction()?;
    let id = transaction.record_injection(&order)?;
    assert!(transaction.label_injection(id, 2, true)?);
    transaction.commit()?;
    let labelled = |writer: &Writer| -> TestResult<(Option<u32>, Option<bool>)> {
        let rows = writer.queries().injections_of_task(task)?;
        let [row] = &rows[..] else {
            return Err(format!("{rows:?}").into());
        };
        assert_eq!(row.record, order);
        Ok((row.used_count, row.label))
    };
    assert_eq!(labelled(&writer)?, (Some(2), Some(true)));
    let mut transaction = writer.transaction()?;
    assert!(transaction.label_injection(id, 0, false)?);
    transaction.commit()?;
    assert_eq!(labelled(&writer)?, (Some(0), Some(false)));
    // An injection recorded in a transaction that is let go is not held.
    let mut transaction = writer.transaction()?;
    let missing = transaction.record_injection(&order)?;
    drop(transaction);
    let mut transaction = writer.transaction()?;
    assert!(!transaction.label_injection(missing, 1, true)?);
    transaction.commit()?;
    assert_eq!(labelled(&writer)?, (Some(0), Some(false)));
    Ok(())
}

/// An injection was delivered only when its text reached the agent: a work order with its text's
/// hash was; one the gate kept back, one held out (its text's hash kept all the same) and one whose
/// text was not written in time (no hash) were not.
#[test]
fn only_a_text_the_agent_got_was_delivered() -> TestResult {
    let session = Identifier::new("s1")?;
    let shown = InjectionRecord {
        task_id: None,
        session_id: session,
        hook: HookKind::Prompt,
        at_ms: 1,
        files: Vec::new(),
        tokens: 10,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the work order")),
        features: None,
    };
    assert!(shown.was_delivered());
    let kept_back = InjectionRecord {
        is_gated_out: true,
        text_hash: None,
        ..shown.clone()
    };
    let held_out = InjectionRecord {
        is_holdout: true,
        ..shown.clone()
    };
    let late = InjectionRecord {
        text_hash: None,
        ..shown.clone()
    };
    for record in [kept_back, held_out, late] {
        assert!(!record.was_delivered(), "{record:?}");
    }
    Ok(())
}

/// Only an injection that reached the agent and is labelled counts for the self-check, the newest
/// first, as `InjectionRecord::was_delivered` tells one: of a delivered work order labelled 1, one
/// the gate kept back, one held out and one whose text was not written in time, each labelled 1,
/// a delivered one not labelled yet and a delivered one labelled 0, recorded in that order, the
/// labels are 0 then 1 and the count is 2; of those 2, 1 was used.
#[test]
fn only_labelled_deliveries_count() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let session = Identifier::new("s1")?;
    let tasks: Vec<Ulid> = (1..=6)
        .map(|at| Ulid::from_seed(at, b"self-check"))
        .collect();
    session_with(&mut writer, &session, &tasks)?;
    let delivered = |task: Ulid| InjectionRecord {
        task_id: Some(task),
        session_id: session.clone(),
        hook: HookKind::Prompt,
        at_ms: 2_000,
        files: Vec::new(),
        tokens: 20,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the work order")),
        features: None,
    };
    let rows = [
        (delivered(tasks[0]), Some(true)),
        (
            InjectionRecord {
                is_gated_out: true,
                text_hash: None,
                ..delivered(tasks[1])
            },
            Some(true),
        ),
        (
            InjectionRecord {
                is_holdout: true,
                ..delivered(tasks[2])
            },
            Some(true),
        ),
        (
            InjectionRecord {
                text_hash: None,
                ..delivered(tasks[3])
            },
            Some(true),
        ),
        (delivered(tasks[4]), None),
        (delivered(tasks[5]), Some(false)),
    ];
    let mut transaction = writer.transaction()?;
    let mut counted = 0;
    for (record, label) in &rows {
        let id = transaction.record_injection(record)?;
        if let Some(label) = label {
            transaction.label_injection(id, 0, *label)?;
            counted += usize::from(record.was_delivered());
        }
    }
    transaction.commit()?;
    let queries = writer.queries();
    assert_eq!(queries.delivered_labels(10)?, [false, true]);
    assert_eq!(queries.delivered_labels(1)?, [false]);
    assert_eq!(queries.delivered_labelled()?, 2);
    assert_eq!(queries.delivered_precision()?, (2, 1));
    assert_eq!(counted, 2, "the store's count agrees with was_delivered");
    Ok(())
}

/// A session's work orders since a time are its prompts' retrievals that reached the agent, as
/// `was_delivered` says, the latest first: of a primer at 5, work orders given at 10 and 50, one
/// the gate kept back at 20, one held out at 30, an edit's warnings at 40, the notices a prompt was
/// given at 45 (a text at the prompt without the gate's features), and another session's work order
/// at 60, session s1's since 0 are those at 50 and 10, since 10 the one at 50, and the latest one
/// alone at 50.
#[test]
fn a_session_s_work_orders_are_those_given() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let (s1, s2) = (Identifier::new("s1")?, Identifier::new("s2")?);
    let (task, other) = (
        Ulid::parse("01JC0000000000000000000000")?,
        Ulid::parse("01JC0000000000000000000001")?,
    );
    session_with(&mut writer, &s1, &[task])?;
    session_with(&mut writer, &s2, &[other])?;
    let given = |session: &Identifier, task_id, hook, at_ms| InjectionRecord {
        task_id,
        session_id: session.clone(),
        hook,
        at_ms,
        files: Vec::new(),
        tokens: 10,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"a text")),
        // A prompt's retrieval records the gate's features; any other text records none.
        features: (hook == HookKind::Prompt).then(features),
    };
    let rows = [
        given(&s1, None, HookKind::SessionStart, 5),
        given(&s1, Some(task), HookKind::Prompt, 10),
        // Kept back, though with a hash: each of the three conditions excludes a row on its own.
        InjectionRecord {
            is_gated_out: true,
            ..given(&s1, Some(task), HookKind::Prompt, 20)
        },
        InjectionRecord {
            is_holdout: true,
            ..given(&s1, Some(task), HookKind::Prompt, 30)
        },
        given(&s1, Some(task), HookKind::PostTool, 40),
        // The notices a prompt was given: a text at the prompt, without the gate's features.
        InjectionRecord {
            features: None,
            ..given(&s1, Some(task), HookKind::Prompt, 45)
        },
        given(&s1, Some(task), HookKind::Prompt, 50),
        given(&s2, Some(other), HookKind::Prompt, 60),
    ];
    let mut transaction = writer.transaction()?;
    for row in &rows {
        transaction.record_injection(row)?;
    }
    transaction.commit()?;
    let queries = writer.queries();
    let times = |after_ms, limit| -> TestResult<Vec<i64>> {
        Ok(queries
            .work_orders_since(&s1, after_ms, limit)?
            .iter()
            .map(|row| row.record.at_ms)
            .collect())
    };
    assert_eq!(times(0, 10)?, [50, 10]);
    assert_eq!(times(10, 10)?, [50]);
    assert_eq!(times(0, 1)?, [50]);
    Ok(())
}

/// A task was given a file for a reason when a text that reached its agent named the file for that
/// reason. Task t: a.py named as read next, delivered; b.py named as read next by a text not
/// written in time; c.py named as read next by a text the gate kept back, and named by a delivered
/// work order. Task u: b.py named as read next, delivered. So t was given a.py as read next, not
/// b.py nor c.py, and c.py as named; u was given b.py as read next and nothing else.
#[test]
fn a_task_was_given_what_reached_it() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let [a, b, c] = nodes(&mut writer, &["a.py", "b.py", "c.py"])?[..] else {
        return Err("three nodes".into());
    };
    let session = Identifier::new("s1")?;
    let (t, u) = (
        Ulid::parse("01JC0000000000000000000000")?,
        Ulid::parse("01JC0000000000000000000001")?,
    );
    session_with(&mut writer, &session, &[t, u])?;
    let given = |task_id, node, reason| InjectionRecord {
        task_id: Some(task_id),
        session_id: session.clone(),
        hook: HookKind::PreTool,
        at_ms: 2_000,
        files: vec![InjectedFile {
            node,
            spans: Vec::new(),
            reason: Some(reason),
            shown: Vec::new(),
            content: None,
        }],
        tokens: 20,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"a line")),
        features: None,
    };
    let rows = [
        given(t, a, ReasonKind::ReadNext),
        InjectionRecord {
            text_hash: None,
            ..given(t, b, ReasonKind::ReadNext)
        },
        InjectionRecord {
            is_gated_out: true,
            ..given(t, c, ReasonKind::ReadNext)
        },
        InjectionRecord {
            hook: HookKind::Prompt,
            ..given(t, c, ReasonKind::Named)
        },
        given(u, b, ReasonKind::ReadNext),
    ];
    let mut transaction = writer.transaction()?;
    for row in &rows {
        transaction.record_injection(row)?;
    }
    transaction.commit()?;
    let queries = writer.queries();
    let named = |task_id, reason| -> TestResult<Vec<bool>> {
        [a, b, c]
            .into_iter()
            .map(|node| Ok(queries.has_named(task_id, node, reason)?))
            .collect()
    };
    assert_eq!(named(t, ReasonKind::ReadNext)?, [true, false, false]);
    assert_eq!(named(t, ReasonKind::Named)?, [false, false, true]);
    assert_eq!(named(u, ReasonKind::ReadNext)?, [false, true, false]);
    Ok(())
}

/// A file of an injection recorded before it kept the code it showed reads back with none shown.
#[test]
fn an_older_record_shows_nothing() -> TestResult {
    let file: InjectedFile = serde_json::from_str(r#"{"node":3,"spans":[[1,2]],"reason":null}"#)?;
    assert_eq!((file.shown, file.content), (Vec::new(), None));
    Ok(())
}
