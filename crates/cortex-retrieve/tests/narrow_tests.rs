//! The narrow tests of a task and the hint a whole-suite run gets, from a store built here: the
//! edited file's own tests first, then the test files tied to it, the strongest first, at most 3;
//! and the hint within its cap, its command left out once a file is. Every expected value is
//! typed out by hand from the files below.

mod common;

use common::{Def, Repo, TestResult, event, function, record, tool};
use cortex_core::{EdgeType, FileFlags, HookKind, Identifier, SymbolKind, ToolKind};
use cortex_retrieve::{MAX_NARROW_HINT_TOKENS, NarrowTest, narrow_hint, narrow_tests};

/// A test function `name` on lines 1 and 2.
fn test_def(name: &str) -> Def<'_> {
    Def {
        kind: SymbolKind::Test,
        ..function(name, 1, 2)
    }
}

/// `calc/ops.py`, edited in session `s1`'s task, and four test files: `tests/test_ops.py`, its
/// own test by name, tied to it at 0.5; `tests/test_util.py` at 0.9; `tests/test_main.py` at 0.8;
/// `tests/test_far.py` at 0.7. A plain file `main.py` tied at 1.0 holds no test.
#[test]
fn own_tests_come_first_then_the_strongest() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "line\nline\n";
    let ops = repo.file(
        "calc/ops.py",
        text,
        &[function("divide", 1, 2)],
        FileFlags::empty(),
    )?;
    let ops = *ops.first().ok_or("a node")?;
    let mut tied = |path: &str, weight: f64, is_test: bool| -> TestResult {
        let defs = if is_test {
            vec![test_def("test_it")]
        } else {
            vec![function("run", 1, 2)]
        };
        let node = *repo
            .file(path, text, &defs, FileFlags::empty())?
            .first()
            .ok_or("node")?;
        repo.link((node, ops), EdgeType::Structural, (weight, 1, 0))
    };
    tied("tests/test_ops.py", 0.5, true)?;
    tied("tests/test_util.py", 0.9, true)?;
    tied("tests/test_main.py", 0.8, true)?;
    tied("tests/test_far.py", 0.7, true)?;
    tied("main.py", 1.0, false)?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(("s1", 2_100), ToolKind::Edit, ("calc/ops.py", &[(2, 2)]))?,
        ],
    )?;
    let reader = repo.reader()?;
    let queries = reader.queries();
    let task = queries
        .tasks_of_session(&Identifier::new("s1")?)?
        .first()
        .ok_or("a task")?
        .record
        .task_id;
    let tests = narrow_tests(queries, task)?;
    let paths: Vec<&str> = tests.iter().map(|test| test.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "tests/test_ops.py",
            "tests/test_util.py",
            "tests/test_main.py"
        ]
    );
    Ok(())
}

/// The hint names the tests and the command that runs them; when it would pass its cap, files are
/// left out from the last, and the command with them, since it no longer runs only those named.
#[test]
fn the_hint_keeps_within_its_cap() -> TestResult {
    let mut repo = Repo::new()?;
    let node = *repo
        .file("tests/test_ops.py", "x\n", &[], FileFlags::empty())?
        .first()
        .ok_or("node")?;
    let test = |path: &str| -> TestResult<NarrowTest> {
        Ok(NarrowTest {
            path: cortex_core::RepoPath::new(path)?,
            node,
        })
    };
    let two = [test("tests/test_ops.py")?, test("tests/test_main.py")?];
    assert_eq!(
        narrow_hint(&two, Some("pytest tests/test_ops.py tests/test_main.py"))
            .map(|(text, _)| text)
            .as_deref(),
        Some(
            "[cortex] Tests for the files changed in this task: tests/test_ops.py, \
             tests/test_main.py (`pytest tests/test_ops.py tests/test_main.py`).\n"
        )
    );
    assert_eq!(
        narrow_hint(&two, None).map(|(text, _)| text).as_deref(),
        Some(
            "[cortex] Tests for the files changed in this task: tests/test_ops.py, \
             tests/test_main.py.\n"
        )
    );
    // Three paths of 90 characters: the three and their command pass 80 tokens (320 bytes); two
    // without the command do too (about 250 bytes, 63 tokens, fit); so two are named, no command.
    let long = |name: &str| format!("tests/{}/{name}.py", "deep".repeat(20));
    let three = [
        test(&long("test_a"))?,
        test(&long("test_b"))?,
        test(&long("test_c"))?,
    ];
    let command = format!(
        "pytest {} {} {}",
        long("test_a"),
        long("test_b"),
        long("test_c")
    );
    let (hint, named) = narrow_hint(&three, Some(&command)).ok_or("no hint")?;
    assert_eq!(named, 2);
    assert!(
        cortex_core::estimate_tokens(&hint) <= MAX_NARROW_HINT_TOKENS,
        "{hint}"
    );
    assert!(!hint.contains('`'), "{hint}");
    assert!(
        hint.contains(&long("test_b")) && !hint.contains(&long("test_c")),
        "{hint}"
    );
    assert_eq!(narrow_hint(&[], Some("pytest")), None);
    Ok(())
}
