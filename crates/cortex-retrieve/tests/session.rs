//! What a session has been doing, read from the events the recorder stored for it.

mod common;

use common::{FEATURES, Repo, TestResult, event, failed, function, of_subagent, record, tool};
use cortex_core::{
    ContentHash, FileFlags, HookKind, Identifier, LineRange, RepoPath, ShortHash, ToolKind,
};
use cortex_retrieve::{GraphSettings, ReadSpan, SessionMatch, read_graph, session};
use cortex_store::{InjectedFile, InjectionRecord, NodeId};

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

/// A read of the whole file that the agent's tool cut after line 9 is recorded as the whole
/// file's read, and holds lines 1 to 9 in the session's context, not the whole file.
#[test]
fn a_read_cut_short_holds_the_lines_it_returned() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "line\n".repeat(15);
    let a = repo.file(
        "a.py",
        &text,
        &[function("f", 1, 4), function("g", 6, 14)],
        FileFlags::empty(),
    )?;
    let mut cut = tool(("s1", 2_100), ToolKind::Read, ("a.py", &[]))?;
    if let Some(target) = cut.tool_target.as_mut() {
        target.cut_after = Some(9);
    }
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            cut,
        ],
    )?;
    let (found, _) = shown(&repo, &[a[1], a[2]])?;
    assert_eq!(found.in_context, [read_of_a((1, 9), &text)?]);
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

/// Spans as their first and last lines.
type Lines<'a> = &'a [(u32, u32)];

/// Records in `repo` a work order given in task `task` of session `s1` at `at_ms`, showing the code
/// of `shown` and naming the other lines of `listed` of the file of `node`, which held `text`; kept
/// back by the gate when `is_gated_out`.
fn work_order(
    repo: &mut Repo,
    (task, at_ms): (cortex_core::Ulid, i64),
    (node, text): (NodeId, &str),
    (listed, shown, is_gated_out): (Lines<'_>, Lines<'_>, bool),
) -> TestResult {
    let mut transaction = repo.writer.transaction()?;
    transaction.record_injection(&InjectionRecord {
        task_id: Some(task),
        session_id: Identifier::new("s1")?,
        hook: HookKind::Prompt,
        at_ms,
        files: vec![InjectedFile {
            node,
            spans: listed.to_vec(),
            reason: None,
            shown: shown.to_vec(),
            content: Some(ShortHash::of(text.as_bytes())),
        }],
        tokens: 100,
        confidence: None,
        is_gated_out,
        is_holdout: false,
        text_hash: (!is_gated_out).then(|| ContentHash::of(b"a work order")),
        features: Some(FEATURES),
    })?;
    transaction.commit()?;
    Ok(())
}

/// The code a work order of the session showed is held until the session's next compaction, with
/// what its file held then: a work order showing `f` (lines 1 to 4) and naming `g` (6 to 9), and
/// one the gate kept back that would have shown `g`, leave `f` alone held. After a compaction
/// nothing is, until a later work order shows `g`. None of it is a read: the context the agent
/// read is not touched.
#[test]
fn a_work_order_s_code_is_held_until_a_compaction() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "line\n".repeat(10);
    let a = repo.file(
        "a.py",
        &text,
        &[function("f", 1, 4), function("g", 6, 9)],
        FileFlags::empty(),
    )?;
    let file = *a.first().ok_or("a file node")?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
        ],
    )?;
    let task = repo
        .reader()?
        .queries()
        .open_task(&Identifier::new("s1")?)?
        .ok_or("the task")?
        .record
        .task_id;
    work_order(
        &mut repo,
        (task, 2_050),
        (file, &text),
        (&[(1, 4), (6, 9)], &[(1, 4)], false),
    )?;
    work_order(
        &mut repo,
        (task, 2_060),
        (file, &text),
        (&[(6, 9)], &[(6, 9)], true),
    )?;
    let (found, _) = shown(&repo, &[])?;
    assert_eq!(found.shown, [read_of_a((1, 4), &text)?]);
    assert!(found.in_context.is_empty());

    record(&mut repo, &[event("s1", 2_200, HookKind::Compact)?])?;
    let (found, _) = shown(&repo, &[])?;
    assert!(found.shown.is_empty());
    work_order(
        &mut repo,
        (task, 2_300),
        (file, &text),
        (&[(6, 9)], &[(6, 9)], false),
    )?;
    let (found, _) = shown(&repo, &[])?;
    assert_eq!(found.shown, [read_of_a((6, 9), &text)?]);
    Ok(())
}
