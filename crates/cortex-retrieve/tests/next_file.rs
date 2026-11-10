//! The line a read gets naming the file earlier tasks read next, from a store and files on disk
//! built here: named only when the way is strong and plainly the main one, to a file on disk as
//! indexed, not read or changed in the task yet, and not named before in it. Every expected line
//! is typed out by hand.

mod common;

use common::{Repo, TestResult, function};
use cortex_core::{
    Agent, ContentHash, EventKind, EventMeta, FileFlags, HookKind, Identifier, ReasonKind,
    RepoPath, Ulid,
};
use cortex_retrieve::{NextFile, NextQuery, next_file};
use cortex_store::{
    EventRecord, InjectedFile, InjectionRecord, NodeId, SessionRecord, TaskRecord, TransitionRow,
};

/// A repository holding `src/a.py`, `src/b.py`, `src/c.py` and `src/d.py` on disk and in the store,
/// with session `s1` and its open task; the files' nodes and the task.
fn repository() -> TestResult<(Repo, [NodeId; 4], Ulid)> {
    let mut repo = Repo::new()?;
    let mut nodes = Vec::new();
    for name in ["a", "b", "c", "d"] {
        let path = format!("src/{name}.py");
        let file = repo.file(
            &path,
            &format!("def {name}():\n    return 1\n"),
            &[function(name, 1, 2)],
            FileFlags::empty(),
        )?;
        nodes.push(*file.first().ok_or("a file node")?);
    }
    let task = Ulid::from_seed(1, b"next");
    let session = Identifier::new("s1")?;
    let mut transaction = repo.writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: session.clone(),
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 1,
    })?;
    transaction.start_task(&TaskRecord {
        task_id: task,
        session_id: session,
        started_at_ms: 2,
        prompt_hash: None,
        prompt_excerpt: None,
        holdout: Some(false),
    })?;
    transaction.commit()?;
    let nodes: [NodeId; 4] = nodes.try_into().map_err(|_| "four nodes")?;
    Ok((repo, nodes, task))
}

/// Makes the ways from `src` exactly `ways`, each (to, weight, count, last task).
fn ways(repo: &mut Repo, src: NodeId, ways: &[(NodeId, f64, u32, u64)]) -> TestResult {
    let mut transaction = repo.writer.transaction()?;
    let held: Vec<(NodeId, NodeId)> = transaction
        .queries()
        .transitions_from(&[src])?
        .iter()
        .map(|way| (way.src, way.dst))
        .collect();
    transaction.remove_transitions(&held)?;
    let rows: Vec<TransitionRow> = ways
        .iter()
        .map(|&(dst, weight, count, last_seq)| TransitionRow {
            src,
            dst,
            weight,
            count,
            last_seq,
        })
        .collect();
    transaction.set_transitions(&rows)?;
    transaction.commit()?;
    Ok(())
}

/// The line a read of `src/a.py` in `task` gets, at task number `seq`, ways fading to 0.995 a task.
fn asked(repo: &Repo, task: Ulid, seq: u64) -> TestResult<Option<NextFile>> {
    let reader = repo.reader()?;
    let path = RepoPath::new("src/a.py")?;
    let candidate = next_file(
        reader.queries(),
        &NextQuery {
            path: &path,
            task,
            seq,
            link_decay: 0.995,
        },
    )?;
    Ok(candidate.and_then(|candidate| candidate.checked(repo.root.path(), 1_048_576)))
}

/// Case by case, each worked by hand, from `src/a.py` at task 3 unless said:
/// - to b 2.5 (3 times) and c 0.5 (once), at 3: b holds 2.5 of 3.0, at least 2 and more than
///   half, and is named, read there "3 times", the times that way was taken;
/// - to b 1.9 alone: under 2, nothing;
/// - to b 2.5 and c 2.5: b holds exactly half of 5.0, no more, nothing;
/// - to b 2.5, c 2.4 and d 1.0: b is the most but holds 2.5 of 5.9, under half, nothing;
/// - to b 2.5 kept at task 0, asked at task 100: 2.5 x 0.995^100, about 1.51, under 2, nothing;
/// - to b 2.5 again: named; once the task has read `src/b.py`: nothing.
#[test]
fn the_file_read_next_is_named_only_when_the_way_is_strong() -> TestResult {
    let (mut repo, [a, b, c, d], task) = repository()?;
    ways(&mut repo, a, &[(b, 2.5, 3, 3), (c, 0.5, 1, 3)])?;
    let named = asked(&repo, task, 3)?.ok_or("a line")?;
    assert_eq!(named.node, b);
    assert_eq!(
        named.line,
        "[cortex] After reading src/a.py, earlier tasks here went on to read src/b.py (3 times).\n"
    );
    ways(&mut repo, a, &[(b, 1.9, 3, 3)])?;
    assert_eq!(asked(&repo, task, 3)?, None);
    ways(&mut repo, a, &[(b, 2.5, 3, 3), (c, 2.5, 3, 3)])?;
    assert_eq!(asked(&repo, task, 3)?, None);
    ways(
        &mut repo,
        a,
        &[(b, 2.5, 3, 3), (c, 2.4, 3, 3), (d, 1.0, 1, 3)],
    )?;
    assert_eq!(asked(&repo, task, 3)?, None);
    ways(&mut repo, a, &[(b, 2.5, 3, 0)])?;
    assert_eq!(asked(&repo, task, 100)?, None);

    ways(&mut repo, a, &[(b, 2.5, 3, 3)])?;
    assert!(asked(&repo, task, 3)?.is_some());
    let mut transaction = repo.writer.transaction()?;
    transaction.append_event(&EventRecord {
        task_id: Some(task),
        session_id: Some(Identifier::new("s1")?),
        ts_ms: 5,
        kind: EventKind::Read,
        node_id: Some(b),
        path: Some(RepoPath::new("src/b.py")?),
        range: None,
        meta: EventMeta::default(),
    })?;
    transaction.commit()?;
    assert_eq!(asked(&repo, task, 3)?, None);
    Ok(())
}

/// A line of the task naming `src/b.py` already, and a `src/b.py` changed on disk since it was
/// indexed, each leave the read with nothing, the way to b strong all the while; a line naming it
/// that never reached the agent (its text not written in time) does not count.
#[test]
fn a_file_named_already_or_changed_on_disk_is_not_named() -> TestResult {
    let (mut repo, [a, b, _, _], task) = repository()?;
    ways(&mut repo, a, &[(b, 2.5, 3, 3)])?;
    assert!(asked(&repo, task, 3)?.is_some());
    let line = InjectionRecord {
        task_id: Some(task),
        session_id: Identifier::new("s1")?,
        hook: HookKind::PreTool,
        at_ms: 6,
        files: vec![InjectedFile {
            node: b,
            spans: Vec::new(),
            reason: Some(ReasonKind::ReadNext),
            shown: Vec::new(),
            content: None,
        }],
        tokens: 20,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the line")),
        features: None,
    };
    let mut transaction = repo.writer.transaction()?;
    transaction.record_injection(&InjectionRecord {
        text_hash: None,
        ..line.clone()
    })?;
    transaction.commit()?;
    assert!(asked(&repo, task, 3)?.is_some());
    let mut transaction = repo.writer.transaction()?;
    transaction.record_injection(&line)?;
    transaction.commit()?;
    assert_eq!(asked(&repo, task, 3)?, None);

    let (mut repo, [a, b, _, _], task) = repository()?;
    ways(&mut repo, a, &[(b, 2.5, 3, 3)])?;
    std::fs::write(
        repo.root.path().join("src/b.py"),
        "def b():\n    return 2\n",
    )?;
    assert_eq!(asked(&repo, task, 3)?, None);
    Ok(())
}

/// A way as strong as the one to b, to a generated `src/gen.py` or a vendored `vendor/lib.py`,
/// names nothing: neither is code an agent works on. Edited in the task, b is not named either.
#[test]
fn a_file_not_worked_on_or_changed_already_is_not_named() -> TestResult {
    let (mut repo, [a, b, _, _], task) = repository()?;
    for (path, flags) in [
        ("src/gen.py", FileFlags::GENERATED),
        ("vendor/lib.py", FileFlags::VENDORED),
    ] {
        let to = *repo
            .file(
                path,
                "def f():\n    return 1\n",
                &[function("f", 1, 2)],
                flags,
            )?
            .first()
            .ok_or("a file node")?;
        ways(&mut repo, a, &[(to, 2.5, 3, 3)])?;
        assert_eq!(asked(&repo, task, 3)?, None, "{path}");
    }
    ways(&mut repo, a, &[(b, 2.5, 3, 3)])?;
    assert!(asked(&repo, task, 3)?.is_some());
    let mut transaction = repo.writer.transaction()?;
    transaction.append_event(&EventRecord {
        task_id: Some(task),
        session_id: Some(Identifier::new("s1")?),
        ts_ms: 5,
        kind: EventKind::Edit,
        node_id: Some(b),
        path: Some(RepoPath::new("src/b.py")?),
        range: None,
        meta: EventMeta::default(),
    })?;
    transaction.commit()?;
    assert_eq!(asked(&repo, task, 3)?, None);
    Ok(())
}
