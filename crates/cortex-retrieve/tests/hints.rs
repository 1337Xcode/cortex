//! The hint a read gets, from a store and files on disk built here: a range the session read from
//! the same file is said to be read already; in a long file, the parts the task's work order gave
//! are named; and nothing when the file changed since it was indexed, is short, or nothing applies.
//! Every expected line is typed out by hand.

mod common;

use common::task::given;
use common::{Def, Repo, TestResult, function};
use cortex_core::{
    Agent, ContentHash, EventKind, EventMeta, FileFlags, Identifier, LineRange, RepoPath,
    SymbolKind, ToolKind, Ulid, estimate_tokens,
};
use cortex_retrieve::{HintQuery, MAX_HINT_TOKENS, read_hint};
use cortex_store::{EventRecord, NodeId, SessionRecord, TaskRecord};

/// The session of the reads.
const SESSION: &str = "s1";

/// A file of `lines` lines, each its own number, so that any change shows.
fn text_of(lines: u32) -> String {
    (1..=lines)
        .map(|line| format!("x{line} = {line}\n"))
        .collect()
}

/// `src/long.py`, 700 lines on disk and in the store, defining `create_order` on lines 120 to 150
/// and the class `OrderService` on lines 300 to 320; the store's session `s1` with one open task;
/// the file's node, its text and the task.
fn long_file(repo: &mut Repo, path: &str) -> TestResult<(NodeId, String, Ulid)> {
    let text = text_of(700);
    let class = Def {
        kind: SymbolKind::Class,
        ..function("OrderService", 300, 320)
    };
    let nodes = repo.file(
        path,
        &text,
        &[function("create_order", 120, 150), class],
        FileFlags::empty(),
    )?;
    let node = *nodes.first().ok_or("a file node")?;
    let task = Ulid::from_seed(1, b"task");
    let session = Identifier::new(SESSION)?;
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
    Ok((node, text, task))
}

/// Records that the session read lines `range` (none for the whole file) of the file of `node` at
/// `path`, which then held `content`, the read succeeding as `is_success` says.
fn read(
    repo: &mut Repo,
    (path, node): (&str, NodeId),
    (range, content, is_success): (Option<(u32, u32)>, &str, bool),
) -> TestResult {
    let mut transaction = repo.writer.transaction()?;
    transaction.append_event(&EventRecord {
        task_id: None,
        session_id: Some(Identifier::new(SESSION)?),
        ts_ms: 4,
        kind: EventKind::Read,
        node_id: Some(node),
        path: Some(RepoPath::new(path)?),
        range: range
            .map(|(start, end)| LineRange::new(start, end))
            .transpose()?,
        meta: EventMeta {
            tool: Some(ToolKind::Read),
            is_success: Some(is_success),
            content: Some(ContentHash::of(content.as_bytes()).short()),
            ..EventMeta::default()
        },
    })?;
    transaction.commit()?;
    Ok(())
}

/// Records that the session's context was compacted.
fn compacted(repo: &mut Repo) -> TestResult {
    let mut transaction = repo.writer.transaction()?;
    transaction.append_event(&EventRecord {
        task_id: None,
        session_id: Some(Identifier::new(SESSION)?),
        ts_ms: 4,
        kind: EventKind::Compact,
        node_id: None,
        path: None,
        range: None,
        meta: EventMeta::default(),
    })?;
    transaction.commit()?;
    Ok(())
}

/// The hint a read of `asked` lines of `path` in the session gets, in task `task`.
fn hint(
    repo: &Repo,
    path: &str,
    asked: Option<(u32, u32)>,
    task: Option<Ulid>,
) -> TestResult<Option<String>> {
    let path = RepoPath::new(path)?;
    let session = Identifier::new(SESSION)?;
    let reader = repo.reader()?;
    Ok(read_hint(
        reader.queries(),
        &HintQuery {
            root: repo.root.path(),
            path: &path,
            asked: asked
                .map(|(start, end)| LineRange::new(start, end))
                .transpose()?,
            session: &session,
            task,
            max_file_bytes: 1_048_576,
        },
    )?)
}

/// A long file whose task's work order showed two of its spans: a read of it is told those parts,
/// by their lines and the symbols they span, the strongest first. A work order the gate kept back
/// names nothing.
#[test]
fn a_long_file_is_told_the_parts_the_task_was_given() -> TestResult {
    let mut repo = Repo::new()?;
    let (node, _, task) = long_file(&mut repo, "src/long.py")?;
    given(&mut repo, (task, node), &[(400, 410)], true)?;
    assert_eq!(hint(&repo, "src/long.py", None, Some(task))?, None);
    given(
        &mut repo,
        (task, node),
        &[(300, 320), (120, 150), (300, 320)],
        false,
    )?;
    assert_eq!(
        hint(&repo, "src/long.py", None, Some(task))?.as_deref(),
        Some(
            "[cortex] For this task the relevant parts of src/long.py are lines 300-320 \
             (OrderService) and 120-150 (create_order).\n"
        )
    );
    // Outside the task, or with no task, the parts are not named.
    assert_eq!(hint(&repo, "src/long.py", None, None)?, None);
    Ok(())
}

/// A work order given from the file as it was is not told once the file changed: the task's work
/// order named `create_order` at lines 120 to 150; fifty lines are then written above it and the
/// file is indexed again, so its lines moved, and a read of it is told no part.
#[test]
fn parts_given_from_an_older_file_are_not_told() -> TestResult {
    let mut repo = Repo::new()?;
    let (node, text, task) = long_file(&mut repo, "src/long.py")?;
    given(&mut repo, (task, node), &[(120, 150)], false)?;
    assert!(hint(&repo, "src/long.py", None, Some(task))?.is_some());
    let moved = format!("{}{text}", "# added\n".repeat(50));
    repo.file(
        "src/long.py",
        &moved,
        &[function("create_order", 170, 200)],
        FileFlags::empty(),
    )?;
    assert_eq!(hint(&repo, "src/long.py", None, Some(task))?, None);
    Ok(())
}

/// A range the session read before from the same file, unchanged, is said to be read: lines 100 to
/// 200, read whole, hold 120 to 150; a read of the whole file holds any lines, and is told by the
/// lines asked. Not told: a read that failed, a read of the file as it was before a change, a read
/// whose range does not hold all the lines asked (90 to 150), and a read the session's context
/// lost when it was compacted; a read after the compaction is told again.
#[test]
fn a_range_read_before_is_said_to_be_read() -> TestResult {
    let mut repo = Repo::new()?;
    let (node, text, task) = long_file(&mut repo, "src/long.py")?;
    let file = ("src/long.py", node);
    read(&mut repo, file, (Some((100, 200)), &text, false))?;
    read(&mut repo, file, (Some((100, 200)), "an older text\n", true))?;
    assert_eq!(
        hint(&repo, "src/long.py", Some((120, 150)), Some(task))?,
        None
    );
    read(&mut repo, file, (Some((100, 200)), &text, true))?;
    assert_eq!(
        hint(&repo, "src/long.py", Some((120, 150)), Some(task))?.as_deref(),
        Some(
            "[cortex] You read lines 100-200 of this file earlier in this session; unchanged since.\n"
        )
    );
    assert_eq!(
        hint(&repo, "src/long.py", Some((90, 150)), Some(task))?,
        None
    );
    read(&mut repo, file, (None, &text, true))?;
    assert_eq!(
        hint(&repo, "src/long.py", Some((90, 150)), Some(task))?.as_deref(),
        Some(
            "[cortex] You read lines 90-150 of this file earlier in this session; unchanged since.\n"
        )
    );
    compacted(&mut repo)?;
    assert_eq!(
        hint(&repo, "src/long.py", Some((90, 150)), Some(task))?,
        None
    );
    read(&mut repo, file, (Some((80, 160)), &text, true))?;
    assert_eq!(
        hint(&repo, "src/long.py", Some((90, 150)), Some(task))?.as_deref(),
        Some(
            "[cortex] You read lines 80-160 of this file earlier in this session; unchanged since.\n"
        )
    );
    Ok(())
}

/// No hint for a file of 600 lines, whatever its task was given; nor for a long file changed on
/// disk since it was indexed, whose lines may no longer be the ones named.
#[test]
fn a_short_or_changed_file_gets_no_hint() -> TestResult {
    let mut repo = Repo::new()?;
    let (node, text, task) = long_file(&mut repo, "src/long.py")?;
    given(&mut repo, (task, node), &[(120, 150)], false)?;
    let short = *repo
        .file(
            "src/short.py",
            &text_of(600),
            &[function("f", 10, 20)],
            FileFlags::empty(),
        )?
        .first()
        .ok_or("a file node")?;
    given(&mut repo, (task, short), &[(10, 20)], false)?;
    assert_eq!(hint(&repo, "src/short.py", None, Some(task))?, None);
    read(&mut repo, ("src/long.py", node), (None, &text, true))?;
    std::fs::write(
        repo.root.path().join("src").join("long.py"),
        format!("{text}x701 = 701\n"),
    )?;
    assert_eq!(hint(&repo, "src/long.py", Some((1, 5)), Some(task))?, None);
    assert_eq!(hint(&repo, "src/long.py", None, Some(task))?, None);
    Ok(())
}

/// A hint is held to its cap of 120 tokens, 480 bytes at four bytes a token. By hand: a path of
/// 458 bytes with one part is 44 + 458 + 4 + 28 + 2 = 536 bytes, too many: no hint. A path of 400
/// bytes with three unnamed parts is 45 + 400 + 5 + 32 + 2 = 484 bytes, 121 tokens, too many; with
/// two, 45 + 400 + 5 + 23 + 2 = 475 bytes, 119 tokens: the first two are named.
#[test]
fn a_hint_is_held_to_its_cap() -> TestResult {
    let mut repo = Repo::new()?;
    // No part of a path may be longer than 255 characters on Windows: each is split in two.
    let longest = format!("src/{}/{}.py", "p".repeat(200), "p".repeat(250));
    let (node, _, task) = long_file(&mut repo, &longest)?;
    given(&mut repo, (task, node), &[(120, 150)], false)?;
    assert_eq!(hint(&repo, &longest, None, Some(task))?, None);

    let mut other = Repo::new()?;
    let long = format!("src/{}/{}.py", "q".repeat(200), "q".repeat(192));
    let (node, _, task) = long_file(&mut other, &long)?;
    given(
        &mut other,
        (task, node),
        &[(1, 100), (101, 200), (201, 299)],
        false,
    )?;
    let text = hint(&other, &long, None, Some(task))?.ok_or("a hint")?;
    assert_eq!(
        text,
        format!(
            "[cortex] For this task the relevant parts of {long} are lines 1-100 and 101-200.\n"
        )
    );
    assert_eq!(
        (text.len(), estimate_tokens(&text)),
        (475, MAX_HINT_TOKENS - 1)
    );
    Ok(())
}
