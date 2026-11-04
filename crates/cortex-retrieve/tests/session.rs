//! What a session has been doing, read from the events the recorder stored for it.

mod common;

use common::{Repo, TestResult, function};
use cortex_core::{
    Agent, Event, FileFlags, HookKind, Identifier, LineRange, RepoPath, ShortHash, ToolKind,
    ToolResult, ToolTarget, Ulid,
};
use cortex_learn::{Commands, Recorder};
use cortex_retrieve::{GraphSettings, ReadSpan, SessionMatch, read_graph, session};
use cortex_store::NodeId;

/// An event of `hook` in `session` at `ts_ms`.
fn event(session: &str, ts_ms: i64, hook: HookKind) -> TestResult<Event> {
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, session.as_bytes()),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new(session)?,
        parent_session_id: None,
        cwd: std::path::PathBuf::from("/repo"),
        repo_root: std::path::PathBuf::from("/repo"),
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: (hook == HookKind::Prompt).then(|| "fix it".to_owned()),
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// A tool's result in `session` at `ts_ms`: `tool` on `path`, the lines `ranges`, done.
fn tool(
    (session, ts_ms): (&str, i64),
    tool: ToolKind,
    (path, ranges): (&str, &[(u32, u32)]),
) -> TestResult<Event> {
    Ok(Event {
        tool: Some(tool),
        tool_target: Some(ToolTarget {
            path: Some(RepoPath::new(path)?),
            ranges: ranges
                .iter()
                .map(|(start, end)| LineRange::new(*start, *end))
                .collect::<Result<_, _>>()?,
            ..ToolTarget::default()
        }),
        tool_result: Some(ToolResult {
            exit_code: None,
            is_success: true,
            is_interrupted: false,
            output_bytes: None,
        }),
        ..event(session, ts_ms, HookKind::PostTool)?
    })
}

/// `event`, failed.
fn failed(event: Event) -> Event {
    Event {
        tool_result: Some(ToolResult {
            exit_code: None,
            is_success: false,
            is_interrupted: false,
            output_bytes: None,
        }),
        ..event
    }
}

/// A subagent's event: its session is `parent/agent`, and its parent is `parent`.
fn of_subagent(parent: &str, event: Event) -> TestResult<Event> {
    Ok(Event {
        session_id: Identifier::new(&format!("{parent}/agent"))?,
        parent_session_id: Some(Identifier::new(parent)?),
        ..event
    })
}

/// Records `events` into `repo`'s store, each in a transaction of its own.
fn record(repo: &mut Repo, events: &[Event]) -> TestResult {
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    for event in events {
        let mut transaction = repo.writer.transaction()?;
        recorder.record(&mut transaction, event)?;
        transaction.commit()?;
    }
    Ok(())
}

/// What session `s1` shows of `repo`, with the graph's numbers of `nodes`.
fn shown(repo: &Repo, nodes: &[NodeId]) -> TestResult<(SessionMatch, Vec<u32>)> {
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let found = session(reader.queries(), &graph, &Identifier::new("s1")?)?;
    let numbers = nodes
        .iter()
        .map(|node| graph.index_of(*node).ok_or("a node of the store"))
        .collect::<Result<Vec<u32>, _>>()?;
    Ok((found, numbers))
}

/// A read of `a.py` at `range`, as the context holds it: with the short hash of `text`, which the
/// index holds for it.
fn read_of_a(range: (u32, u32), text: &str) -> TestResult<ReadSpan> {
    Ok(ReadSpan {
        path: RepoPath::new("a.py")?,
        range: Some(LineRange::new(range.0, range.1)?),
        content: Some(ShortHash::of(text.as_bytes())),
    })
}

/// A session read lines 2 to 3 of `a.py` (in `f`, lines 1 to 4), edited lines 7 to 8 (in `g`,
/// lines 6 to 9), had a subagent edit `b.py` whole, read line 1 (`f` again) and line 7 (`g`),
/// failed to read `c.py` and failed to edit `d.py`; another session edited `c.py`. Worked by hand:
/// `f` is read twice and carries 0.4 once; `g` is edited and read, 1.0 + 0.4 = 1.4; `b.py` is
/// edited, 1.0; `c.py` and `d.py` nothing. The files edited are `a.py` and `b.py`. The context
/// holds the three reads of `a.py`, newest first, with the short hash of what the index held for
/// it; not the failed read.
#[test]
fn edits_and_reads_seed_what_they_touched() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "line\n".repeat(10);
    let defs = [function("f", 1, 4), function("g", 6, 9)];
    let a = repo.file("a.py", &text, &defs, FileFlags::empty())?;
    let b = repo.file("b.py", "x = 1\n", &[], FileFlags::empty())?;
    let c = repo.file("c.py", "y = 2\n", &[], FileFlags::empty())?;
    let d = repo.file("d.py", "z = 3\n", &[], FileFlags::empty())?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(("s1", 2_100), ToolKind::Read, ("a.py", &[(2, 3)]))?,
            tool(("s1", 2_200), ToolKind::Edit, ("a.py", &[(7, 8)]))?,
            of_subagent("s1", tool(("s1", 2_300), ToolKind::Edit, ("b.py", &[]))?)?,
            tool(("s1", 2_400), ToolKind::Read, ("a.py", &[(1, 1)]))?,
            tool(("s1", 2_500), ToolKind::Read, ("a.py", &[(7, 7)]))?,
            failed(tool(("s1", 2_600), ToolKind::Read, ("c.py", &[]))?),
            failed(tool(("s1", 2_610), ToolKind::Edit, ("d.py", &[]))?),
            event("s2", 2_650, HookKind::SessionStart)?,
            event("s2", 2_700, HookKind::Prompt)?,
            tool(("s2", 2_800), ToolKind::Edit, ("c.py", &[]))?,
        ],
    )?;
    let (found, numbers) = shown(&repo, &[a[0], a[1], a[2], b[0], c[0], d[0]])?;
    let [file_a, f, g, file_b, file_c, file_d] = numbers.as_slice() else {
        return Err("six nodes".into());
    };
    assert_eq!(found.seeds.get(f), Some(&0.4));
    assert_eq!(found.seeds.get(g), Some(&1.4));
    assert_eq!(found.seeds.get(file_b), Some(&1.0));
    assert_eq!(found.seeds.get(file_c), None);
    assert_eq!(found.seeds.get(file_d), None);
    assert_eq!(found.seeds.len(), 3);
    let mut edited = vec![*file_a, *file_b];
    edited.sort_unstable();
    assert_eq!(
        found.edited_files.iter().copied().collect::<Vec<_>>(),
        edited
    );
    assert_eq!(
        found.in_context,
        [
            read_of_a((7, 7), &text)?,
            read_of_a((1, 1), &text)?,
            read_of_a((2, 3), &text)?,
        ]
    );
    Ok(())
}

/// A stretch of lines counts for each symbol it meets, the symbol's first and last lines included,
/// or for its file when it meets none: lines 4 to 5 meet `f` (1 to 4) at its last line, line 5
/// meets nothing, lines 5 to 6 meet `g` (6 to 9) at its first line. So `f`, the file and `g` are
/// each seeded at 0.4.
#[test]
fn a_stretch_counts_for_the_symbols_it_meets() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "line\n".repeat(10);
    let defs = [function("f", 1, 4), function("g", 6, 9)];
    let a = repo.file("a.py", &text, &defs, FileFlags::empty())?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(("s1", 2_100), ToolKind::Read, ("a.py", &[(4, 5)]))?,
            tool(("s1", 2_200), ToolKind::Read, ("a.py", &[(5, 5)]))?,
            tool(("s1", 2_300), ToolKind::Read, ("a.py", &[(5, 6)]))?,
        ],
    )?;
    let (found, numbers) = shown(&repo, &a)?;
    let expected: std::collections::BTreeMap<u32, f64> =
        numbers.iter().map(|node| (*node, 0.4)).collect();
    assert_eq!(found.seeds, expected);
    Ok(())
}

/// What was read before the last compaction is no longer in the session's context, though it is
/// still among its latest reads and seeds what it touched (`g`); what was read after it (`f`) is
/// in the context too. A subagent's read is in its own context, not the session's, and seeds
/// nothing: `h`, which only the subagent read, is no seed.
#[test]
fn a_compaction_clears_what_is_in_context() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "line\n".repeat(15);
    let defs = [
        function("f", 1, 4),
        function("g", 6, 9),
        function("h", 11, 14),
    ];
    let a = repo.file("a.py", &text, &defs, FileFlags::empty())?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(("s1", 2_100), ToolKind::Read, ("a.py", &[(7, 8)]))?,
            event("s1", 2_200, HookKind::Compact)?,
            of_subagent(
                "s1",
                tool(("s1", 2_300), ToolKind::Read, ("a.py", &[(12, 13)]))?,
            )?,
            tool(("s1", 2_400), ToolKind::Read, ("a.py", &[(3, 4)]))?,
        ],
    )?;
    let (found, numbers) = shown(&repo, &[a[1], a[2], a[3]])?;
    let [f, g, h] = numbers.as_slice() else {
        return Err("three nodes".into());
    };
    assert_eq!(found.in_context, [read_of_a((3, 4), &text)?]);
    assert_eq!(found.seeds.get(f), Some(&0.4));
    assert_eq!(found.seeds.get(g), Some(&0.4));
    assert_eq!(found.seeds.get(h), None);
    assert_eq!(found.seeds.len(), 2);
    Ok(())
}

/// Eleven whole files are read one after another, a compaction coming before the sixth read, then
/// a read of a twelfth fails. The latest ten reads that did not fail seed their files (the second
/// to the eleventh); the first is not a seed, nor the twelfth, and neither the compaction nor the
/// failed read takes one of the ten. The context holds the six reads since the compaction, newest
/// first, as whole files.
#[test]
fn only_the_latest_ten_reads_are_seeds() -> TestResult {
    let mut repo = Repo::new()?;
    let mut files = Vec::new();
    let mut events = vec![
        event("s1", 1_000, HookKind::SessionStart)?,
        event("s1", 2_000, HookKind::Prompt)?,
    ];
    for at in 0..12_i64 {
        let path = format!("r{at:02}.py");
        files.push(repo.file(&path, "x = 1\n", &[], FileFlags::empty())?[0]);
        if at == 5 {
            events.push(event("s1", 2_095 + 10 * at, HookKind::Compact)?);
        }
        let read = tool(("s1", 2_100 + 10 * at), ToolKind::Read, (&path, &[]))?;
        events.push(if at == 11 { failed(read) } else { read });
    }
    record(&mut repo, &events)?;
    let (found, numbers) = shown(&repo, &files)?;
    let [first, latest @ .., twelfth] = numbers.as_slice() else {
        return Err("twelve nodes".into());
    };
    assert_eq!(found.seeds.get(first), None);
    assert_eq!(found.seeds.get(twelfth), None);
    for node in latest {
        assert_eq!(found.seeds.get(node), Some(&0.4), "{node}");
    }
    assert_eq!(found.seeds.len(), 10);
    let held: Vec<(String, Option<LineRange>)> = found
        .in_context
        .iter()
        .map(|span| (span.path.as_str().to_owned(), span.range))
        .collect();
    let expected: Vec<(String, Option<LineRange>)> = (5..11)
        .rev()
        .map(|at| (format!("r{at:02}.py"), None))
        .collect();
    assert_eq!(held, expected);
    Ok(())
}

/// A session with no events shows nothing.
#[test]
fn a_new_session_shows_nothing() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file("a.py", "x = 1\n", &[], FileFlags::empty())?;
    let (found, _) = shown(&repo, &[])?;
    assert_eq!(found, SessionMatch::default());
    Ok(())
}
