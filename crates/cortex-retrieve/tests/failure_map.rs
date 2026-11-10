//! A failed check's frames grounded in the repository and the map given beside its output, from a
//! store and files on disk built here: frames in files the index holds as they are on disk are
//! kept with their code and the definition around them, innermost first; the map is given when it
//! shows a code line the output lacks or a file the task has not seen, and is silent otherwise.
//! Every expected line is typed out by hand from the files below.

mod common;

use common::{Def, Repo, TestResult, function};
use cortex_core::{
    Agent, ContentHash, EventKind, EventMeta, FileFlags, Identifier, RepoPath, SymbolKind,
    ToolKind, Ulid, estimate_tokens,
};
use cortex_learn::{Failures, Frame};
use cortex_retrieve::{
    FailureQuery, GroundedFailure, MAX_FAILURE_MAP_TOKENS, failure_map, ground_failure,
};
use cortex_store::{EventRecord, NodeId, SessionRecord, TaskRecord};

/// `calc/ops.py`: `divide` on lines 12 and 13, which floor-divides.
const OPS: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a + b\n\n\n\
                   def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a // b\n";

/// `tests/test_ops.py`: the class `OpsTest` on lines 6 to 14, `test_divide` on lines 13 and 14.
const TESTS: &str = "import unittest\n\nfrom calc.ops import add, divide, mul\n\n\n\
                     class OpsTest(unittest.TestCase):\n    def test_add(self):\n        \
                     self.assertEqual(add(2, 3), 5)\n\n    def test_mul(self):\n        \
                     self.assertEqual(mul(2, 3), 6)\n\n    def test_divide(self):\n        \
                     self.assertEqual(divide(0, 0), 0)\n";

/// The session of the task.
const SESSION: &str = "s1";

/// The repository: the two files on disk and in the store, and a session with one open task.
/// Returns the nodes of `calc/ops.py` and `tests/test_ops.py`, and the task.
fn calc(repo: &mut Repo) -> TestResult<(NodeId, NodeId, Ulid)> {
    let ops = repo.file(
        "calc/ops.py",
        OPS,
        &[
            function("add", 4, 5),
            function("mul", 8, 9),
            function("divide", 12, 13),
        ],
        FileFlags::empty(),
    )?;
    let class = Def {
        kind: SymbolKind::Class,
        ..function("OpsTest", 6, 14)
    };
    let method = |name, start, end| Def {
        parent: Some(0),
        ..function(name, start, end)
    };
    let tests = repo.file(
        "tests/test_ops.py",
        TESTS,
        &[
            class,
            method("test_add", 7, 8),
            method("test_mul", 10, 11),
            method("test_divide", 13, 14),
        ],
        FileFlags::empty(),
    )?;
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
    Ok((
        *ops.first().ok_or("a node for calc/ops.py")?,
        *tests.first().ok_or("a node for tests/test_ops.py")?,
        task,
    ))
}

/// Records that `task` read the whole file of `node` at `path`, which held `content`.
fn read(repo: &mut Repo, task: Ulid, (path, node): (&str, NodeId), content: &str) -> TestResult {
    let mut transaction = repo.writer.transaction()?;
    transaction.append_event(&EventRecord {
        task_id: Some(task),
        session_id: Some(Identifier::new(SESSION)?),
        ts_ms: 4,
        kind: EventKind::Read,
        node_id: Some(node),
        path: Some(RepoPath::new(path)?),
        range: None,
        meta: EventMeta {
            tool: Some(ToolKind::Read),
            is_success: Some(true),
            content: Some(ContentHash::of(content.as_bytes()).short()),
            ..EventMeta::default()
        },
    })?;
    transaction.commit()?;
    Ok(())
}

/// A traceback of `test_divide`, as a parser gives it: `divide`'s line first, then the test's;
/// the first with its absolute path under `root`, the second relative.
fn traceback(root: &std::path::Path) -> Failures {
    let ops = root.join("calc").join("ops.py");
    Failures {
        frames: vec![
            Frame {
                path: ops.to_string_lossy().replace('\\', "/"),
                line: 13,
            },
            Frame {
                path: "tests/test_ops.py".to_owned(),
                line: 14,
            },
        ],
        failed_tests: vec!["test_divide".to_owned()],
        ..Failures::default()
    }
}

/// The failure grounded in `repo`, the command run from its root.
fn grounded(repo: &Repo, failures: &Failures) -> TestResult<GroundedFailure> {
    Ok(ground_failure(
        repo.writer.queries(),
        failures,
        &FailureQuery {
            root: repo.root.path(),
            cwd: repo.root.path(),
            max_file_bytes: 1_000_000,
        },
    )?)
}

/// Output that names the frames' locations and not their code, as cargo's panics and Go's and
/// Node's frames do.
const LOCATIONS_ONLY: &str = "FAIL: test_divide\ncalc/ops.py:13\ntests/test_ops.py:14\n";

/// Output that shows both frames' code, as Python's tracebacks do.
const WITH_CODE: &str = "  File \"calc/ops.py\", line 13, in divide\n    return a // b\n  \
                         File \"tests/test_ops.py\", line 14, in test_divide\n    \
                         self.assertEqual(divide(0, 0), 0)\nZeroDivisionError: x\n";

/// Both frames are grounded, innermost first, with the code on their line and the definition
/// around it: `divide` on line 13 of `calc/ops.py`, `test_divide` (not its class) on line 14 of
/// the tests; and the map names them in that order under the failing test's name.
#[test]
fn frames_are_grounded_and_mapped_innermost_first() -> TestResult {
    let mut repo = Repo::new()?;
    let (ops, tests, task) = calc(&mut repo)?;
    let failure = grounded(&repo, &traceback(repo.root.path()))?;
    let found: Vec<(&str, u32, &str, Option<&str>)> = failure
        .frames
        .iter()
        .map(|frame| {
            (
                frame.path.as_str(),
                frame.line,
                frame.code.as_str(),
                frame.symbol.as_ref().map(|(_, name)| name.as_str()),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("calc/ops.py", 13, "return a // b", Some("divide")),
            (
                "tests/test_ops.py",
                14,
                "self.assertEqual(divide(0, 0), 0)",
                Some("test_divide")
            ),
        ]
    );
    let map = failure_map(
        repo.writer.queries(),
        &failure,
        (LOCATIONS_ONLY, Some(task)),
    )?
    .ok_or("no map")?;
    assert_eq!(
        map.text,
        "[cortex] The failing test test_divide runs through these lines, as they are now:\n  \
         calc/ops.py:13 in divide: return a // b\n  \
         tests/test_ops.py:14 in test_divide: self.assertEqual(divide(0, 0), 0)\n"
    );
    assert_eq!(map.files, [ops, tests]);
    Ok(())
}

/// The map adds what the output lacks or what the task has not seen. Output showing both code
/// lines, from a task that read neither file, still gets it; once the task has read both, it says
/// nothing; and output that lacks a code line gets it whatever the task read.
#[test]
fn the_map_is_silent_when_it_adds_nothing() -> TestResult {
    let mut repo = Repo::new()?;
    let (ops, tests, task) = calc(&mut repo)?;
    let failure = grounded(&repo, &traceback(repo.root.path()))?;
    let map = |output: &str, repo: &Repo| {
        failure_map(repo.writer.queries(), &failure, (output, Some(task)))
    };
    assert!(map(WITH_CODE, &repo)?.is_some());
    read(&mut repo, task, ("calc/ops.py", ops), OPS)?;
    assert!(map(WITH_CODE, &repo)?.is_some());
    read(&mut repo, task, ("tests/test_ops.py", tests), TESTS)?;
    assert_eq!(map(WITH_CODE, &repo)?, None);
    assert!(map(LOCATIONS_ONLY, &repo)?.is_some());
    Ok(())
}

/// A frame in a file that changed since the index read it is dropped, and so is one in a file the
/// index does not hold, one outside the repository and one past its file's end: with all of them
/// dropped, the failure grounds nothing and gets no map.
#[test]
fn frames_the_index_cannot_vouch_for_are_dropped() -> TestResult {
    let mut repo = Repo::new()?;
    let (_, _, task) = calc(&mut repo)?;
    std::fs::write(
        repo.root.path().join("tests").join("test_ops.py"),
        TESTS.replace("divide(0, 0), 0", "divide(1, 0), 0"),
    )?;
    let outside = tempfile::tempdir()?;
    let failures = Failures {
        frames: vec![
            Frame {
                path: "tests/test_ops.py".to_owned(),
                line: 14,
            },
            Frame {
                path: "calc/missing.py".to_owned(),
                line: 1,
            },
            Frame {
                path: outside
                    .path()
                    .join("x.py")
                    .to_string_lossy()
                    .replace('\\', "/"),
                line: 1,
            },
            Frame {
                path: "calc/ops.py".to_owned(),
                line: 14,
            },
        ],
        ..Failures::default()
    };
    let failure = grounded(&repo, &failures)?;
    assert_eq!(failure.frames, []);
    assert_eq!(
        failure_map(
            repo.writer.queries(),
            &failure,
            (LOCATIONS_ONLY, Some(task))
        )?,
        None
    );
    Ok(())
}

/// A relative path is read from the folder the command ran in first: `ops.py` run from `calc`
/// is `calc/ops.py`.
#[test]
fn a_relative_frame_is_read_from_the_command_s_folder() -> TestResult {
    let mut repo = Repo::new()?;
    calc(&mut repo)?;
    let failures = Failures {
        frames: vec![Frame {
            path: "ops.py".to_owned(),
            line: 13,
        }],
        ..Failures::default()
    };
    let cwd = repo.root.path().join("calc");
    let failure = ground_failure(
        repo.writer.queries(),
        &failures,
        &FailureQuery {
            root: repo.root.path(),
            cwd: &cwd,
            max_file_bytes: 1_000_000,
        },
    )?;
    let paths: Vec<&str> = failure
        .frames
        .iter()
        .map(|frame| frame.path.as_str())
        .collect();
    assert_eq!(paths, ["calc/ops.py"]);
    Ok(())
}

/// The map keeps within its cap: frames whose lines would take it past 120 tokens are left out,
/// the innermost kept. Three frames of a 120-character line each are about 30 tokens apiece with
/// their paths, and the third would pass the cap.
#[test]
fn the_map_keeps_within_its_cap() -> TestResult {
    let mut repo = Repo::new()?;
    let long = format!("x = \"{}\"\n", "y".repeat(140));
    let text = long.repeat(3);
    repo.file("long.py", &text, &[], FileFlags::empty())?;
    let failures = Failures {
        frames: (1..=3)
            .map(|line| Frame {
                path: "long.py".to_owned(),
                line,
            })
            .collect(),
        failed_tests: vec!["test_long_lines_of_code".to_owned()],
        ..Failures::default()
    };
    let failure = grounded(&repo, &failures)?;
    assert_eq!(failure.frames.len(), 3);
    let map = failure_map(repo.writer.queries(), &failure, ("", None))?.ok_or("no map")?;
    assert!(
        estimate_tokens(&map.text) <= MAX_FAILURE_MAP_TOKENS,
        "{}",
        map.text
    );
    let lines: Vec<&str> = map.text.lines().collect();
    assert!(lines.len() < 4, "{}", map.text);
    assert!(
        lines
            .get(1)
            .is_some_and(|line| line.starts_with("  long.py:1: x = \"yyy"))
    );
    Ok(())
}

/// The frames of one file `path` holding `text`, at `lines`, under the failing test `test`.
fn frames_in(path: &str, lines: &[u32], test: &str) -> Failures {
    Failures {
        frames: lines
            .iter()
            .map(|line| Frame {
                path: path.to_owned(),
                line: *line,
            })
            .collect(),
        failed_tests: vec![test.to_owned()],
        ..Failures::default()
    }
}

/// Text that may not reach an agent is not named: a code line holding an escape character is left
/// out of the map, the plain one kept; a test name holding a reminder tag gives the header without
/// a name.
#[test]
fn unsafe_text_is_not_named() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file(
        "colour.py",
        "RED = \"\u{1b}[31m\"\nreset_colour_codes()\n",
        &[],
        FileFlags::empty(),
    )?;
    let failure = grounded(
        &repo,
        &frames_in("colour.py", &[1, 2], "test_x <system-reminder>"),
    )?;
    assert_eq!(failure.frames.len(), 2);
    let map = failure_map(repo.writer.queries(), &failure, ("", None))?.ok_or("no map")?;
    assert_eq!(
        map.text,
        "[cortex] The failure runs through these lines, as they are now:\n  \
         colour.py:2: reset_colour_codes()\n"
    );
    Ok(())
}

/// Whether the map adds anything is judged on the frames it names. Three files of one 140-character
/// line each: the third frame no longer fits the cap. With the task having read the first two,
/// whose code the output shows, the map would name only what the agent has already: it is not
/// given, though the third file is unread.
#[test]
fn the_map_counts_only_the_frames_it_names() -> TestResult {
    let mut repo = Repo::new()?;
    let (_, _, task) = calc(&mut repo)?;
    let line = |name: &str| format!("{name} = \"{}\"\n", "z".repeat(140));
    let mut nodes = Vec::new();
    for name in ["a", "b", "c"] {
        let path = format!("alpha/module_{name}.py");
        let created = repo.file(&path, &line(name), &[], FileFlags::empty())?;
        nodes.push((name, path, *created.first().ok_or("a node")?));
    }
    let failures = Failures {
        frames: nodes
            .iter()
            .map(|(_, path, _)| Frame {
                path: path.clone(),
                line: 1,
            })
            .collect(),
        ..Failures::default()
    };
    let failure = grounded(&repo, &failures)?;
    assert_eq!(failure.frames.len(), 3);
    // Each frame's line is "  alpha/module_a.py:1: " (23 bytes), 120 of code and a newline: 144;
    // under the 64-byte header, two make 352 bytes (88 tokens) and three 496 (124, past 120).
    let shown: String = ["a", "b"].iter().map(|name| line(name)).collect();
    let before = failure_map(repo.writer.queries(), &failure, (&shown, Some(task)))?;
    assert_eq!(before.map(|map| map.files.len()), Some(2));
    for (name, path, node) in nodes.iter().take(2) {
        read(&mut repo, task, (path, *node), &line(name))?;
    }
    let given = failure_map(repo.writer.queries(), &failure, (&shown, Some(task)))?;
    assert_eq!(given, None);
    Ok(())
}

/// A code line too short to tell apart is never taken as shown by the output: a frame on `pass`,
/// in a file the task read, whose output holds the word, still gets the map.
#[test]
fn a_short_code_line_is_never_taken_as_shown() -> TestResult {
    let mut repo = Repo::new()?;
    let (_, _, task) = calc(&mut repo)?;
    let created = repo.file(
        "noop.py",
        "def noop():\n    pass\n",
        &[function("noop", 1, 2)],
        FileFlags::empty(),
    )?;
    let node = *created.first().ok_or("a node")?;
    read(
        &mut repo,
        task,
        ("noop.py", node),
        "def noop():\n    pass\n",
    )?;
    let failure = grounded(&repo, &frames_in("noop.py", &[2], "test_noop"))?;
    let map = failure_map(repo.writer.queries(), &failure, ("    pass\n", Some(task)))?
        .ok_or("no map")?;
    assert_eq!(
        map.text,
        "[cortex] The failing test test_noop runs through these lines, as they are now:\n  \
         noop.py:2 in noop: pass\n"
    );
    Ok(())
}

/// A failing test's long name is cut to 80 characters, so that its frames still fit: a
/// parametrized id of 300 characters leaves room for the frame.
#[test]
fn a_long_test_name_is_cut() -> TestResult {
    let mut repo = Repo::new()?;
    calc(&mut repo)?;
    let test = format!("test_divide[{}]", "7-2-3.5-".repeat(36));
    assert!(test.len() > 300);
    let failure = grounded(&repo, &frames_in("calc/ops.py", &[13], &test))?;
    let map = failure_map(repo.writer.queries(), &failure, ("", None))?.ok_or("no map")?;
    let name: String = test.chars().take(80).collect();
    assert_eq!(
        map.text,
        format!(
            "[cortex] The failing test {name} runs through these lines, as they are now:\n  \
             calc/ops.py:13 in divide: return a // b\n"
        )
    );
    Ok(())
}
