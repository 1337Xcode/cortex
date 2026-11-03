//! What the daemon learns as tasks end, from captured Claude Code payloads sent through the real
//! binary to a real daemon: a task that searched, read, edited and then passed its tests is judged
//! a success, and its interest, its link, its record and its cluster are written; when the next
//! task of the session puts the change back, that task's end judges the first reverted.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::hook_call::hook;
use common::{Daemon, TestResult, payloads, wait_for};
use cortex_core::{
    CommandKind, ContentHash, EdgeType, EventKind, Identifier, InterestTier, RepoPath, TaskOutcome,
    Ulid,
};
use cortex_store::{
    ChangedFile, ChangedSymbol, NodeId, Pitfalls, Queries, RanCheck, ReadFile, Reader,
};
use serde_json::Value;

/// The Claude Code version whose captured payloads these tests send.
const CAPTURED: &str = "2.1.312";

/// The session of the captured payloads.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// `calc/ops.py` as the captured session found it: `divide`, on lines 12 and 13, divides by floor.
const OPS: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a + b\n\n\n\
                   def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a // b\n";

/// `calc/ops.py` once the captured edit made `divide` divide truly, on line 13.
const OPS_FIXED: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a + b\n\n\n\
                         def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a / b\n";

/// `tests/test_ops.py`: 14 lines testing the three functions.
const TESTS: &str = "import unittest\n\nfrom calc.ops import add, divide, mul\n\n\n\
                     class OpsTest(unittest.TestCase):\n    def test_add(self):\n        \
                     self.assertEqual(add(2, 3), 5)\n\n    def test_mul(self):\n        \
                     self.assertEqual(mul(2, 3), 6)\n\n    def test_divide(self):\n        \
                     self.assertEqual(divide(7, 2), 3.5)\n";

/// A home folder holding the repository the payloads were captured in: `calc/ops.py` and the
/// tests the session ran, `main.py` and the two packages' empty `__init__.py`, which the captured
/// search for Python files lists.
fn repository() -> TestResult<(tempfile::TempDir, PathBuf)> {
    let home = tempfile::tempdir()?;
    let repo = home.path().join("repo");
    std::fs::create_dir_all(repo.join("calc"))?;
    std::fs::create_dir_all(repo.join("tests"))?;
    std::fs::write(repo.join("calc/__init__.py"), "")?;
    std::fs::write(repo.join("calc/ops.py"), OPS)?;
    std::fs::write(repo.join("tests/__init__.py"), "")?;
    std::fs::write(repo.join("tests/test_ops.py"), TESTS)?;
    std::fs::write(
        repo.join("main.py"),
        "from calc.ops import add\n\nprint(add(1, 2))\n",
    )?;
    Ok((home, repo))
}

/// The captured payload `name`, its paths moved under `root`.
fn captured(name: &str, root: &Path) -> TestResult<Value> {
    Ok(serde_json::from_slice(&payloads::payload(
        CAPTURED, name, root,
    )?)?)
}

/// The captured read of `calc/ops.py` made a read of the whole of `tests/test_ops.py`.
fn read_of_tests(root: &Path) -> TestResult<Value> {
    let mut read = captured("post-tool-use-read.json", root)?;
    let path = root.join("tests").join("test_ops.py");
    let path = Value::String(path.to_string_lossy().into_owned());
    read["tool_input"]["file_path"] = path.clone();
    let file = &mut read["tool_response"]["file"];
    file["filePath"] = path;
    file["content"] = Value::String(TESTS.to_owned());
    file["numLines"] = Value::from(15);
    file["totalLines"] = Value::from(15);
    Ok(read)
}

/// The captured edit of line 13 of `calc/ops.py` made the edit that puts it back.
fn edit_back(root: &Path) -> TestResult<Value> {
    let mut edit = captured("post-tool-use-edit.json", root)?;
    let (fixed, floor) = ("    return a / b", "    return a // b");
    edit["tool_input"]["old_string"] = Value::from(fixed);
    edit["tool_input"]["new_string"] = Value::from(floor);
    let response = &mut edit["tool_response"];
    response["oldString"] = Value::from(fixed);
    response["newString"] = Value::from(floor);
    response["originalFile"] = Value::from(OPS_FIXED);
    response["structuredPatch"][0]["lines"] = Value::from(vec![
        " ".to_owned(),
        " ".to_owned(),
        " def divide(a, b):".to_owned(),
        format!("-{fixed}"),
        format!("+{floor}"),
    ]);
    Ok(edit)
}

/// The captured run of passing tests, which Claude Code sent in a session of its own, sent as the
/// session's: its session, prompt and transcript are the captured prompt's.
fn tests_passing(root: &Path) -> TestResult<Value> {
    let mut run = captured("post-tool-use-bash-test-passing.json", root)?;
    let prompt = captured("user-prompt-submit.json", root)?;
    for field in ["session_id", "transcript_path", "prompt_id"] {
        run[field] = prompt[field].clone();
    }
    Ok(run)
}

/// The captured prompt with words of its own.
fn prompt(root: &Path, words: &str) -> TestResult<Value> {
    let mut prompt = captured("user-prompt-submit.json", root)?;
    prompt["prompt"] = Value::from(words);
    Ok(prompt)
}

/// Runs `cortex hook` for `payload` with `home` as the user's home folder, as Claude Code would.
/// The call must end as an agent requires of a hook: nothing has been learned that injects.
fn send(payload: &Value, home: &Path) -> TestResult {
    let event = payload["hook_event_name"]
        .as_str()
        .ok_or("no event name")?
        .to_owned();
    let input = serde_json::to_vec(payload)?;
    let call = hook(
        &["--agent", "claude", "--event", &event],
        input,
        Duration::ZERO,
        home,
    )?;
    assert!(call.is_silent_success(), "{event}: {:?}", call.output);
    Ok(())
}

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

/// Whether the store holds `calc/ops.py` with the contents `text`: the daemon read that change.
fn holds_ops(root: &Path, text: &str) -> TestResult<bool> {
    read(root, |queries| {
        let row = queries.file(&RepoPath::new("calc/ops.py")?)?;
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
#[test]
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
    wait_for(|| holds_ops(&repo, OPS_FIXED))?;
    send(&tests_passing(&repo)?, home)?;
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
    wait_for(|| holds_ops(&repo, OPS))?;
    send(&captured("stop.json", &repo)?, home)?;
    wait_for(|| is_learned(&repo, 1))?;
    let second = *tasks(&repo)?.get(1).ok_or("no second task")?;
    assert_eq!(outcome(&repo, second)?, (Some(TaskOutcome::Unknown), None));
    assert_eq!(
        outcome(&repo, first)?,
        (
            Some(TaskOutcome::Reverted),
            Some(
                "every edited file is back as it was before the task, by the end of the next task"
                    .to_owned()
            )
        )
    );
    let record = read(&repo, |queries| Ok(queries.episode_of_task(first)?))?;
    assert_eq!(
        record.map(|record| record.outcome),
        Some(TaskOutcome::Reverted)
    );
    Ok(())
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
