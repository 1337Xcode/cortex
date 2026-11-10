//! What the daemon learns as tasks end, from captured Claude Code payloads sent through the real
//! binary to a real daemon: a task that searched, read, edited and then passed its tests is judged
//! a success, and its interest, its link, its record and its cluster are written; when the next
//! task of the session puts the change back, that task's end judges the first reverted. Then the
//! acceptance run: a repository with a scripted history and three tasks that end in a success, a
//! failure and a revert, checked for the co-change edges of the history, the outcomes, records and
//! links of the tasks, and the receipts of the session.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};

use common::calc::{
    MAIN, OPS, OPS_FIXED, S1, TESTS, captured, edit_back, prompt, read_of_tests, repository,
    tests_run,
};
use common::hook_call::send;
use common::{Daemon, TestResult, git, payloads, receipts, wait_for};
use cortex_core::{
    CommandKind, ContentHash, EdgeType, EventKind, Identifier, InterestTier, RepoId, RepoPath,
    TaskOutcome, Ulid, UserFolder, canonical_root,
};
use cortex_meter::utc_text;
use cortex_store::{
    ChangedFile, ChangedSymbol, NodeId, Pitfalls, Queries, RanCheck, ReadFile, Reader, TaskRow,
};
use serde_json::{Value, json};

/// The Claude Code version whose captured transcript gives the acceptance run's responses.
const METERED: &str = "2.1.316";

/// Reads the store of the repository at `root` with `read`.
fn read<T>(root: &Path, read: impl FnOnce(Queries<'_>) -> TestResult<T>) -> TestResult<T> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    read(reader.queries())
}

/// The tasks of the captured session, in the order they started.
fn tasks(root: &Path) -> TestResult<Vec<Ulid>> {
    let session = Identifier::new(S1)?;
    read(root, |queries| {
        let mut tasks: Vec<Ulid> = Vec::new();
        for row in queries.events_of_session(&session)? {
            if let Some(task) = row.record.task_id
                && !tasks.contains(&task)
            {
                tasks.push(task);
            }
        }
        Ok(tasks)
    })
}

/// Whether the `number`th task of the session has its record, which only learning writes.
fn is_learned(root: &Path, number: usize) -> TestResult<bool> {
    let Some(task) = tasks(root)?.get(number).copied() else {
        return Ok(false);
    };
    read(root, |queries| Ok(queries.episode_of_task(task)?.is_some()))
}

/// Whether the store holds the session's edit of `calc/ops.py`.
fn has_edit(root: &Path) -> TestResult<bool> {
    let session = Identifier::new(S1)?;
    read(root, |queries| {
        Ok(queries
            .events_of_session(&session)?
            .iter()
            .any(|row| row.record.kind == EventKind::Edit))
    })
}

/// Whether the store holds any cluster of tasks.
fn has_clusters(root: &Path) -> TestResult<bool> {
    read(root, |queries| Ok(!queries.archetypes()?.is_empty()))
}

/// What became of `task` and the words that say why.
fn outcome(root: &Path, task: Ulid) -> TestResult<(Option<TaskOutcome>, Option<String>)> {
    read(root, |queries| {
        let row = queries.task(task)?.ok_or("no task row")?;
        Ok((row.outcome, row.outcome_evidence))
    })
}

/// Whether the store holds the file at `path` with the contents `text`: the daemon read that
/// change.
fn holds(root: &Path, path: &str, text: &str) -> TestResult<bool> {
    read(root, |queries| {
        let row = queries.file(&RepoPath::new(path)?)?;
        Ok(row.is_some_and(|row| row.file.content_hash == Some(ContentHash::of(text.as_bytes()))))
    })
}

/// The nodes the rows here are about: `calc/ops.py`, `tests/test_ops.py`, `main.py`, the `calc`
/// folder and `divide`, the symbol on lines 12 and 13 of `calc/ops.py`.
struct Nodes {
    /// `calc/ops.py`.
    ops: NodeId,
    /// `tests/test_ops.py`.
    tests: NodeId,
    /// `main.py`.
    main: NodeId,
    /// The `calc` folder.
    calc: NodeId,
    /// `divide`.
    divide: NodeId,
}

impl Nodes {
    /// The nodes as the store of the repository at `root` holds them.
    fn of(root: &Path) -> TestResult<Self> {
        read(root, |queries| {
            let file = |path: &str| -> TestResult<NodeId> {
                Ok(queries
                    .file_node_at(&RepoPath::new(path)?)?
                    .ok_or("no file node")?)
            };
            let ops_row = queries
                .file(&RepoPath::new("calc/ops.py")?)?
                .ok_or("no file row")?;
            let divide = queries
                .symbol_spans_in_file(ops_row.id)?
                .into_iter()
                .find(|span| (span.start_line, span.end_line) == (12, 13))
                .ok_or("no divide")?
                .node;
            Ok(Self {
                ops: file("calc/ops.py")?,
                tests: file("tests/test_ops.py")?,
                main: file("main.py")?,
                calc: queries
                    .folder_node_at(&RepoPath::new("calc")?)?
                    .ok_or("no folder node")?,
                divide,
            })
        })
    }
}

/// A session through the binary: the task searches for Python files and for `def divide`, reads
/// `calc/ops.py` and `tests/test_ops.py` whole, fixes line 13 of `calc/ops.py`, and runs its tests,
/// which pass; then its answer ends. What its end teaches, worked out by hand:
///
/// - It is a success: the tests, by `unittest`, exited 0 after its last edit.
/// - Interest: `calc/ops.py` was found by both searches and opened after (0.2 each), read whole
///   (0.3), edited in `divide` (half the symbol's 1.0) and vouched for by the passing tests (0.5):
///   1.7. `tests/test_ops.py` was found and opened (0.2) and read whole (0.3): 0.5. `divide` takes
///   the edit's 1.0 and the `calc` folder a quarter of it, 0.25. `main.py` was found and never
///   opened: 0.05, too little to be more than dark.
/// - One link, between the two files: the edited one counts 1.0 and the one only read 0.2 (the
///   edited file does not name the tests; they name it), so the learning rate of 0.1 times the
///   two, times the full gain of a success: 0.02, from one observation.
/// - Its record: the two files read once each before the change, in that order; one line of
///   `calc/ops.py` changed, inside `divide` but not its `def` line; the one check; no pitfall.
/// - One cluster, of this task: its file, and the words of the file's path and of `divide`, each
///   given by the one task, in the order met.
///
/// The next task puts line 13 back as it was, and once the daemon has read the file back, its
/// answer ends: nothing vouches for it, so nothing is known of it, and the first task's work is
/// gone, every file it edited holding again what it held before, so the first is now reverted.
/// Its link loses what its success gave it, which decayed as the link did over the one task since:
/// 0.02 x 0.995 less the same leaves nothing, its second observation, as of the second task. It
/// edited one file, so no pair of edited files is weakened besides.
#[test]
#[ignore = "timing under load: its premise (an idle index when the first edit is recorded) needs an idle machine; `cargo xtask verify --full` runs it"]
fn a_session_s_tasks_are_learned_from_as_they_end() -> TestResult {
    let (home, repo) = repository()?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    let home = home.path();
    for name in [
        "session-start-startup.json",
        "user-prompt-submit.json",
        "post-tool-use-glob.json",
        "post-tool-use-grep.json",
        "post-tool-use-read.json",
    ] {
        send(&captured(name, &repo)?, home)?;
    }
    send(&read_of_tests(&repo)?, home)?;
    // The edit's hook goes first and the file changes after, so that its event is stored before
    // the index can read the change, as when the watcher's 200 ms of quiet outlasts the hook: what
    // the file held before the edit is then the store's row. The daemon reads the change while
    // the agent goes on, as it would while the tests run, so what the next task finds the file
    // holding before it changes it is what the store holds.
    send(&captured("post-tool-use-edit.json", &repo)?, home)?;
    wait_for(|| has_edit(&repo))?;
    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    wait_for(|| holds(&repo, "calc/ops.py", OPS_FIXED))?;
    send(
        &tests_run(&repo, "post-tool-use-bash-test-passing.json")?,
        home,
    )?;
    send(&captured("stop.json", &repo)?, home)?;
    // The clusters are written once the tasks waiting are learned from, after the record.
    wait_for(|| Ok(is_learned(&repo, 0)? && has_clusters(&repo)?))?;
    let first = *tasks(&repo)?.first().ok_or("no task")?;
    assert_eq!(
        outcome(&repo, first)?,
        (
            Some(TaskOutcome::Success),
            Some("tests by unittest exited 0 after the last edit".to_owned())
        )
    );
    let nodes = Nodes::of(&repo)?;
    assert_first_task_marks(&repo, first, &nodes)?;

    send(&prompt(&repo, "Put divide back the way it was.")?, home)?;
    std::fs::write(repo.join("calc/ops.py"), OPS)?;
    send(&edit_back(&repo)?, home)?;
    wait_for(|| holds(&repo, "calc/ops.py", OPS))?;
    send(&captured("stop.json", &repo)?, home)?;
    wait_for(|| is_learned(&repo, 1))?;
    let second = *tasks(&repo)?.get(1).ok_or("no second task")?;
    assert_eq!(outcome(&repo, second)?, (Some(TaskOutcome::Unknown), None));
    let reverted = (
        Some(TaskOutcome::Reverted),
        Some(
            "every edited file is back as it was before the task, by the end of the next task"
                .to_owned(),
        ),
    );
    let judged = outcome(&repo, first)?;
    if judged != reverted {
        let why = what_happened(&repo, home)?;
        return Err(format!("the first task is {judged:?}, not {reverted:?}\n{why}").into());
    }
    let record = read(&repo, |queries| Ok(queries.episode_of_task(first)?))?;
    assert_eq!(
        record.map(|record| record.outcome),
        Some(TaskOutcome::Reverted)
    );
    let links = read(&repo, |queries| {
        Ok(queries.edges_among(&[nodes.ops, nodes.tests], EdgeType::CoAccess)?)
    })?;
    let [link] = links.as_slice() else {
        return Err(format!("{} links", links.len()).into());
    };
    let second_seq = read(&repo, |queries| {
        Ok(queries.task(second)?.ok_or("no task row")?.seq)
    })?;
    assert!(link.edge.weight.abs() < 1e-12, "{}", link.edge.weight);
    assert_eq!(
        (link.edge.count, link.edge.last_seq),
        (2, i64::try_from(second_seq)?)
    );
    Ok(())
}

/// What the store and the daemon's log say of the session, for a failure to show: each event of
/// the session (its kind, file, task, time and source) and the log's warnings and errors, none of
/// which hold code or prompt text.
fn what_happened(root: &Path, home: &Path) -> TestResult<String> {
    let tasks = tasks(root)?;
    let mut said = read(root, |queries| {
        let mut lines = Vec::new();
        for row in queries.events_of_session(&Identifier::new(S1)?)? {
            let record = row.record;
            let task = record
                .task_id
                .and_then(|task| tasks.iter().position(|known| *known == task));
            lines.push(format!(
                "event {:?} {:?} task {task:?} at {} source {:?}",
                record.kind, record.path, record.ts_ms, record.meta.source
            ));
        }
        Ok(lines)
    })?;
    let id = RepoId::of(&canonical_root(root)?);
    let log = UserFolder::in_home(home)
        .logs_folder()
        .join(format!("daemon-{id}.log"));
    if let Ok(text) = std::fs::read_to_string(log) {
        said.extend(
            text.lines()
                .filter(|line| line.contains("WARN") || line.contains("ERROR"))
                .map(str::to_owned),
        );
    }
    Ok(said.join("\n"))
}

/// Checks the interest, the link, the record and the cluster the first task's end left, as worked
/// out on [`a_session_s_tasks_are_learned_from_as_they_end`].
fn assert_first_task_marks(root: &Path, task: Ulid, nodes: &Nodes) -> TestResult {
    let seq = read(root, |queries| {
        Ok(queries.task(task)?.ok_or("no task row")?.seq)
    })?;
    let interest = read(root, |queries| {
        Ok(queries.interest_of(&[nodes.ops, nodes.tests, nodes.main, nodes.calc, nodes.divide])?)
    })?;
    let expected = [
        (nodes.ops, 1.7, InterestTier::Interesting),
        (nodes.tests, 0.5, InterestTier::Interesting),
        (nodes.main, 0.05, InterestTier::Dark),
        (nodes.calc, 0.25, InterestTier::Interesting),
        (nodes.divide, 1.0, InterestTier::Interesting),
    ];
    for (node, score, tier) in expected {
        let row = interest
            .iter()
            .find(|row| row.node == node)
            .ok_or("no interest")?;
        assert!((row.score - score).abs() < 1e-9, "{node:?}: {}", row.score);
        assert_eq!((row.last_seq, row.tier), (seq, tier), "{node:?}");
    }
    let links = read(root, |queries| {
        Ok(queries.edges_among(&[nodes.ops, nodes.tests], EdgeType::CoAccess)?)
    })?;
    let [link] = links.as_slice() else {
        return Err(format!("{} links", links.len()).into());
    };
    let pair = (nodes.ops.min(nodes.tests), nodes.ops.max(nodes.tests));
    assert_eq!((link.src, link.edge.dst), pair);
    assert!(
        (link.edge.weight - 0.02).abs() < 1e-9,
        "{}",
        link.edge.weight
    );
    assert_eq!(
        (link.edge.count, link.edge.last_seq),
        (1, i64::try_from(seq)?)
    );
    let record = read(root, |queries| Ok(queries.episode_of_task(task)?))?;
    let unittest = Identifier::new("unittest")?;
    assert_eq!(
        record.map(|record| (
            record.localize,
            record.change,
            record.verify,
            record.pitfalls
        )),
        Some((
            vec![
                ReadFile {
                    node: nodes.ops,
                    reads: 1
                },
                ReadFile {
                    node: nodes.tests,
                    reads: 1
                },
            ],
            vec![ChangedFile {
                node: nodes.ops,
                lines: 1,
                symbols: vec![ChangedSymbol {
                    node: nodes.divide,
                    header: false
                }],
            }],
            vec![RanCheck {
                runner: unittest,
                kind: CommandKind::Test,
                code: Some(0)
            }],
            Pitfalls::default(),
        ))
    );
    let clusters = read(root, |queries| Ok(queries.archetypes()?))?;
    let words: Vec<Vec<String>> = clusters
        .iter()
        .map(|cluster| {
            cluster
                .seed_terms
                .iter()
                .map(|term| term.as_str().to_owned())
                .collect()
        })
        .collect();
    assert_eq!(words, [["calc", "ops", "py", "divide"]]);
    let shapes: Vec<(Vec<NodeId>, u32)> = clusters
        .into_iter()
        .map(|cluster| (cluster.files, cluster.episode_count))
        .collect();
    assert_eq!(shapes, [(vec![nodes.ops], 1)]);
    Ok(())
}

/// The meter's transcript of the session: the four responses of the captured Grep session, written
/// out a task at a time, each response's first line stamped at a time inside its task.
struct Transcript {
    /// Where the session's transcript is, in Claude Code's projects folder.
    path: PathBuf,
    /// The captured lines up to the second response, up to the third, and the rest.
    parts: [String; 3],
}

impl Transcript {
    /// The captured transcript, to be written to the session's place under `home`.
    fn new(home: &Path) -> TestResult<Self> {
        let captured = std::fs::read_to_string(
            payloads::fixtures(METERED)
                .join("transcripts")
                .join("grep-session.jsonl"),
        )?;
        let start = |id: &str| {
            captured
                .find(&format!("\"id\":\"{id}\""))
                .and_then(|at| captured.get(..at)?.rfind('\n'))
                .map(|newline| newline + 1)
                .ok_or(format!("no response {id}"))
        };
        let (second, third) = (start("msg_fixture0002")?, start("msg_fixture0003")?);
        let part = |range: std::ops::Range<usize>| -> TestResult<String> {
            Ok(captured
                .get(range)
                .ok_or("a part of the transcript")?
                .to_owned())
        };
        Ok(Self {
            path: payloads::transcripts_folder(home).join(format!("{S1}.jsonl")),
            parts: [
                part(0..second)?,
                part(second..third)?,
                part(third..captured.len())?,
            ],
        })
    }

    /// Appends part `index`, each `(from, to)` of `times` replacing the captured time `from` with
    /// the UTC time `to` ms.
    fn append(&self, index: usize, times: &[(&str, i64)]) -> TestResult {
        let mut text = self.parts.get(index).ok_or("no such part")?.clone();
        for (from, at_ms) in times {
            assert_eq!(text.matches(from).count(), 1, "{from}");
            text = text.replace(from, &utc_text(*at_ms).ok_or("a time after 1970")?);
        }
        std::fs::create_dir_all(self.path.parent().ok_or("a transcript has a folder")?)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(text.as_bytes())?;
        Ok(())
    }
}

/// Scripts the repository's history, newest last: the first commit adds its five files; the second
/// changes `calc/ops.py` and its tests, the third puts both back; the fourth changes the tests
/// alone and the fifth puts them back. The files hold at the end what [`repository`] wrote.
fn script_history(root: &Path, home: &Path) -> TestResult {
    git::git(root, home, &["init", "-q"])?;
    let all = [
        "calc/__init__.py",
        "calc/ops.py",
        "tests/__init__.py",
        "tests/test_ops.py",
        "main.py",
    ];
    git::commit(root, home, "add the calculator", &all)?;
    let pair = ["calc/ops.py", "tests/test_ops.py"];
    for path in pair {
        let mut text = std::fs::read_to_string(root.join(path))?;
        text.push_str("# a note\n");
        std::fs::write(root.join(path), text)?;
    }
    git::commit(root, home, "note the calculator", &pair)?;
    std::fs::write(root.join("calc/ops.py"), OPS)?;
    std::fs::write(root.join("tests/test_ops.py"), TESTS)?;
    git::commit(root, home, "drop the notes", &pair)?;
    std::fs::write(root.join("tests/test_ops.py"), format!("{TESTS}# a note\n"))?;
    git::commit(root, home, "note the tests", &["tests/test_ops.py"])?;
    std::fs::write(root.join("tests/test_ops.py"), TESTS)?;
    git::commit(root, home, "drop the note", &["tests/test_ops.py"])?;
    Ok(())
}

/// The co-change edges from the file at `path`: each partner, its weight and count.
fn cochanges_of(root: &Path, path: &str) -> TestResult<Vec<(NodeId, f64, u32)>> {
    read(root, |queries| {
        let node = queries
            .file_node_at(&RepoPath::new(path)?)?
            .ok_or("no file node")?;
        Ok(queries
            .edges_from(node)?
            .into_iter()
            .filter(|row| row.edge_type == EdgeType::CoChange)
            .map(|row| (row.edge.dst, row.edge.weight, row.edge.count))
            .collect())
    })
}

/// The session's tasks as the store holds them, in the order they started.
fn task_rows(root: &Path) -> TestResult<Vec<TaskRow>> {
    read(root, |queries| {
        Ok(queries.tasks_of_session(&Identifier::new(S1)?)?)
    })
}

/// When the `number`th task of the session started, once it has.
fn started(root: &Path, number: usize) -> TestResult<i64> {
    wait_for(|| Ok(task_rows(root)?.len() > number))?;
    let rows = task_rows(root)?;
    Ok(rows.get(number).ok_or("no such task")?.record.started_at_ms)
}

/// `payload` naming `transcript` as its session's transcript.
fn in_transcript(mut payload: Value, transcript: &Path) -> Value {
    payload["transcript_path"] = Value::from(transcript.to_string_lossy().into_owned());
    payload
}

/// The acceptance run: one repository with a history and three tasks of one session through the
/// real binary, and what the daemon learned and metered, worked out by hand.
///
/// The history (see [`script_history`]) makes `calc/ops.py` change in commits 1 to 3 and
/// `tests/test_ops.py` in all five; every other file only in the first. A commit's weight is
/// 2^(-age / 500), its age counted from the newest at 0: 1, 0.998615, 0.997231, 0.995850 and
/// 0.994470 for ages 0 to 4. The two files changed together in commits 1 to 3, ages 4, 3 and 2:
///
/// - From `calc/ops.py`, whose every commit changed its tests: confidence 1, count 3.
/// - From `tests/test_ops.py`: (0.997231 + 0.995850 + 0.994470) / (1 + 0.998615 + 0.997231 +
///   0.995850 + 0.994470) = 2.987551 / 4.986166 = 0.599168, count 3.
/// - Every other pair changed together once, under the two co-changes an edge needs.
///
/// The tasks:
///
/// 1. Searches, reads `calc/ops.py` and its tests, fixes line 13 and runs the tests, which pass:
///    a success, and one link between the two files, 0.1 x 1.0 (edited) x 0.2 (read only) x 1.0
///    (a success's gain) = 0.02. Its tool calls: two searches, two reads, the edit and the tests,
///    six.
/// 2. Reads `calc/ops.py`, writes `NOTES.md` and runs a test that fails: a failure, whose gain
///    of 0 gives the two files no link, where a success would give them 0.02 and a task nobody
///    vouches for 0.006. Three calls.
/// 3. Changes line 3 of `main.py` and puts it back: the one file it edited is back as it was
///    before it, so it was reverted, which weakens only links among files edited together, and it
///    edited one. The second task's note is still there, so the second is not reverted. Two
///    calls. (Its file is not `calc/ops.py`: an edit within two seconds of a reported change of the
///    same file has an unknown content before it, and these tasks follow each other in less.)
///
/// The transcript gives each task responses of the captured Grep session (counts read off the
/// fixture by hand, priced as Sonnet 5.5 at $2 input, $4 one-hour cache writes, $0.10 cache reads
/// and $10 output per million tokens, so 2,000,000, 4,000,000, 100,000 and 10,000,000
/// picodollars a token):
///
/// 1. The first: 3,702 input, 29,797 written, 100 output: 33,499 input in all;
///    7,404,000,000 + 119,188,000,000 + 1,000,000,000 = 127,592,000,000.
/// 2. The second: 2 input, 3,819 written, 29,797 read, 95 output: 33,618 in all;
///    4,000,000 + 15,276,000,000 + 2,979,700,000 + 950,000,000 = 19,209,700,000.
/// 3. The third and fourth: 2, 155, 33,616, 360 and 2, 763, 33,771, 3: 68,309 in all, 4
///    uncached, 67,387 read, 918 written, 363 output; 7,585,600,000 + 6,463,100,000 =
///    14,048,700,000.
///
/// The session: 4 turns, 135,426 input, 3,708 uncached, 97,184 read, 34,534 written, 558 output,
/// 160,850,400,000 picodollars, eleven tool calls.
#[test]
fn a_repository_s_history_and_tasks_are_learned_and_metered() -> TestResult {
    let (home_folder, repo) = repository()?;
    let home = home_folder.path();
    script_history(&repo, home)?;
    let transcript = Transcript::new(home)?;
    let daemon = Daemon::start(&repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(!cochanges_of(&repo, "calc/ops.py")?.is_empty()))?;
    let send_in = |payload: Value| send(&in_transcript(payload, &transcript.path), home);

    send_in(captured("session-start-startup.json", &repo)?)?;
    send_in(captured("user-prompt-submit.json", &repo)?)?;
    let first_ms = started(&repo, 0)?;
    transcript.append(0, &[("2026-11-02T23:04:17.241Z", first_ms + 1)])?;
    for name in [
        "post-tool-use-glob.json",
        "post-tool-use-grep.json",
        "post-tool-use-read.json",
    ] {
        send_in(captured(name, &repo)?)?;
    }
    send_in(read_of_tests(&repo)?)?;
    send_in(captured("post-tool-use-edit.json", &repo)?)?;
    wait_for(|| has_edit(&repo))?;
    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    wait_for(|| holds(&repo, "calc/ops.py", OPS_FIXED))?;
    send_in(tests_run(&repo, "post-tool-use-bash-test-passing.json")?)?;
    send_in(captured("stop.json", &repo)?)?;
    wait_for(|| is_learned(&repo, 0))?;

    send_in(prompt(&repo, "Write down how divide divides.")?)?;
    let second_ms = started(&repo, 1)?;
    transcript.append(1, &[("2026-11-02T23:04:18.748Z", second_ms + 1)])?;
    send_in(captured("post-tool-use-read.json", &repo)?)?;
    send_in(captured("post-tool-use-write.json", &repo)?)?;
    wait_for(|| Ok(edits(&repo)? == 2))?;
    let note = "divide uses true division\n";
    std::fs::write(repo.join("NOTES.md"), note)?;
    // The daemon reads the new file while the tests run, as it would for an agent, so that the
    // task's end knows the note's node.
    wait_for(|| holds(&repo, "NOTES.md", note))?;
    send_in(tests_run(&repo, "post-tool-use-failure-bash-test.json")?)?;
    send_in(captured("stop.json", &repo)?)?;
    wait_for(|| is_learned(&repo, 1))?;

    send_in(prompt(&repo, "Try another sum in main.py, then go back.")?)?;
    let third_ms = started(&repo, 2)?;
    transcript.append(
        2,
        &[
            ("2026-11-02T23:04:20.454Z", third_ms + 1),
            ("2026-11-02T23:04:22.200Z", third_ms + 2),
        ],
    )?;
    // Each edit's hook goes first and the file changes once its event is stored, as in the first
    // task, so that what the file held before the edit is the store's row. The first task's edit
    // and the second's note are the session's first two edits.
    send_in(edit_main(&repo, MAIN, MAIN_CHANGED)?)?;
    wait_for(|| Ok(edits(&repo)? == 3))?;
    std::fs::write(repo.join("main.py"), MAIN_CHANGED)?;
    wait_for(|| holds(&repo, "main.py", MAIN_CHANGED))?;
    send_in(edit_main(&repo, MAIN_CHANGED, MAIN)?)?;
    wait_for(|| Ok(edits(&repo)? == 4))?;
    std::fs::write(repo.join("main.py"), MAIN)?;
    wait_for(|| holds(&repo, "main.py", MAIN))?;
    send_in(captured("stop.json", &repo)?)?;
    wait_for(|| is_learned(&repo, 2))?;
    wait_for(|| Ok(usage_rows(&repo)? == 4))?;
    drop(daemon);

    assert_history_edges(&repo)?;
    assert_tasks(&repo)?;
    assert_receipts(&repo, home)
}

/// `main.py` once the third task changed the sum it prints, on line 3.
const MAIN_CHANGED: &str = "from calc.ops import add\n\nprint(add(2, 2))\n";

/// The captured edit of line 13 of `calc/ops.py` made an edit of line 3 of `main.py`, from the
/// file holding `from` to its holding `to`.
fn edit_main(root: &Path, from: &str, to: &str) -> TestResult<Value> {
    let mut edit = captured("post-tool-use-edit.json", root)?;
    let path = Value::from(root.join("main.py").to_string_lossy().into_owned());
    let line = |text: &str| -> TestResult<String> {
        Ok(text.lines().nth(2).ok_or("no line 3")?.to_owned())
    };
    let (old, new) = (line(from)?, line(to)?);
    edit["tool_input"]["file_path"] = path.clone();
    edit["tool_input"]["old_string"] = Value::from(old.as_str());
    edit["tool_input"]["new_string"] = Value::from(new.as_str());
    let response = &mut edit["tool_response"];
    response["filePath"] = path;
    response["oldString"] = Value::from(old.as_str());
    response["newString"] = Value::from(new.as_str());
    response["originalFile"] = Value::from(from);
    response["structuredPatch"] = json!([{
        "oldStart": 1,
        "oldLines": 3,
        "newStart": 1,
        "newLines": 3,
        "lines": [" from calc.ops import add", " ", format!("-{old}"), format!("+{new}")],
    }]);
    Ok(edit)
}

/// How many edits of the session the store holds, a hook's report of a write or an edit each.
fn edits(root: &Path) -> TestResult<usize> {
    let session = Identifier::new(S1)?;
    read(root, |queries| {
        Ok(queries
            .events_of_session(&session)?
            .iter()
            .filter(|row| row.record.kind == EventKind::Edit && row.record.meta.source.is_none())
            .count())
    })
}

/// How many responses of the session the store holds.
fn usage_rows(root: &Path) -> TestResult<usize> {
    read(root, |queries| {
        Ok(queries.usage_of_session(&Identifier::new(S1)?)?.len())
    })
}

/// Checks the co-change edges and commit counts the history left, as worked out on
/// [`a_repository_s_history_and_tasks_are_learned_and_metered`].
fn assert_history_edges(root: &Path) -> TestResult {
    let node = |path: &str| -> TestResult<NodeId> {
        read(root, |queries| {
            Ok(queries
                .file_node_at(&RepoPath::new(path)?)?
                .ok_or("no file node")?)
        })
    };
    let (ops, tests) = (node("calc/ops.py")?, node("tests/test_ops.py")?);
    let close = |found: Vec<(NodeId, f64, u32)>, to: NodeId, weight: f64| match found.as_slice() {
        [(dst, found_weight, count)] => {
            *dst == to && (found_weight - weight).abs() < 1e-9 && *count == 3
        }
        _ => false,
    };
    let from_ops = cochanges_of(root, "calc/ops.py")?;
    assert!(close(from_ops.clone(), tests, 1.0), "{from_ops:?}");
    let from_tests = cochanges_of(root, "tests/test_ops.py")?;
    assert!(
        close(from_tests.clone(), ops, 0.599_168_032_001_645_4),
        "{from_tests:?}"
    );
    for path in ["main.py", "calc/__init__.py", "tests/__init__.py"] {
        assert_eq!(cochanges_of(root, path)?, [], "{path}");
    }
    let counts = read(root, |queries| {
        let mut counts = Vec::new();
        for path in ["calc/ops.py", "tests/test_ops.py", "main.py"] {
            let file = queries.file(&RepoPath::new(path)?)?.ok_or("no file row")?;
            let rows = queries.file_commits_of(&[file.id])?;
            counts.push(rows.first().map(|row| row.count));
        }
        Ok(counts)
    })?;
    assert_eq!(counts, [Some(3), Some(5), Some(1)]);
    Ok(())
}

/// Checks what became of each task, its record, and the links the three left, as worked out on
/// [`a_repository_s_history_and_tasks_are_learned_and_metered`].
fn assert_tasks(root: &Path) -> TestResult {
    let tasks = task_rows(root)?;
    let judged: Vec<(Option<TaskOutcome>, Option<&str>)> = tasks
        .iter()
        .map(|task| (task.outcome, task.outcome_evidence.as_deref()))
        .collect();
    assert_eq!(
        judged,
        [
            (
                Some(TaskOutcome::Success),
                Some("tests by unittest exited 0 after the last edit")
            ),
            (
                Some(TaskOutcome::Failure),
                Some("tests by unittest exited 1 after the last edit")
            ),
            (
                Some(TaskOutcome::Reverted),
                Some("every edited file is back as it was before the task")
            ),
        ]
    );
    let records = read(root, |queries| {
        tasks
            .iter()
            .map(|task| {
                let record = queries.episode_of_task(task.record.task_id)?;
                Ok(record.map(|record| record.outcome))
            })
            .collect::<TestResult<Vec<_>>>()
    })?;
    assert_eq!(
        records,
        [
            Some(TaskOutcome::Success),
            Some(TaskOutcome::Failure),
            Some(TaskOutcome::Reverted)
        ]
    );
    let first_seq = tasks.first().ok_or("no task")?.seq;
    let links = read(root, |queries| {
        let mut nodes = Vec::new();
        for path in ["calc/ops.py", "tests/test_ops.py", "main.py", "NOTES.md"] {
            nodes.extend(queries.file_node_at(&RepoPath::new(path)?)?);
        }
        Ok(queries.edges_among(&nodes, EdgeType::CoAccess)?)
    })?;
    let [link] = links.as_slice() else {
        return Err(format!("{} links", links.len()).into());
    };
    assert!(
        (link.edge.weight - 0.02).abs() < 1e-9,
        "{}",
        link.edge.weight
    );
    assert_eq!(
        (link.edge.count, link.edge.last_seq),
        (1, i64::try_from(first_seq)?)
    );
    Ok(())
}

/// Checks what `cortex receipts` shows of the session, as worked out on
/// [`a_repository_s_history_and_tasks_are_learned_and_metered`]. Every cache write there is for an
/// hour, at 4,000,000 picodollars a token: 29,797, 3,819 and 155 + 763 = 918 written cost
/// 119,188,000,000, 15,276,000,000 and 3,672,000,000, together 138,136,000,000.
fn assert_receipts(root: &Path, home: &Path) -> TestResult {
    let printed = receipts::run(root, home, &["--json"])?;
    assert!(printed.status.success(), "{printed:?}");
    let report: Value = serde_json::from_slice(&printed.stdout)?;
    let session = report.pointer("/recorded/sessions/0").ok_or("no session")?;
    let field = |value: &Value, name: &str| value.get(name).cloned().unwrap_or(Value::Null);
    let tasks = session
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or("no tasks")?;
    let shown: Vec<[Value; 4]> = tasks
        .iter()
        .map(|task| ["seq", "outcome", "tool_calls", "usage"].map(|name| field(task, name)))
        .collect();
    assert_eq!(
        shown,
        [
            [
                json!(1),
                json!("success"),
                json!(6),
                receipts::tally(
                    1,
                    [33_499, 3_702, 0, 29_797, 100],
                    [127_592_000_000, 119_188_000_000]
                )
            ],
            [
                json!(2),
                json!("failure"),
                json!(3),
                receipts::tally(
                    1,
                    [33_618, 2, 29_797, 3_819, 95],
                    [19_209_700_000, 15_276_000_000]
                )
            ],
            [
                json!(3),
                json!("reverted"),
                json!(2),
                receipts::tally(
                    2,
                    [68_309, 4, 67_387, 918, 363],
                    [14_048_700_000, 3_672_000_000]
                )
            ],
        ]
    );
    assert_eq!(
        ["tool_calls", "outside_tool_calls", "total"].map(|name| field(session, name)),
        [
            json!(11),
            json!(0),
            receipts::tally(
                4,
                [135_426, 3_708, 97_184, 34_534, 558],
                [160_850_400_000, 138_136_000_000]
            )
        ]
    );
    assert_eq!(
        field(session, "outside_tasks").get("turns"),
        Some(&json!(0))
    );
    Ok(())
}
