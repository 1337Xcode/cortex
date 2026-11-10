//! Real command lines, typed by hand, against what [`whole_suite`] keeps of them and what
//! [`narrow_command`] writes from that: each runner's whole and selecting forms, the launchers
//! agents use, the lines that are uncertain, the folder a `cd` moves to, the quoting and the cap.
//! Every expected value is written out from the rules in the module's documentation, never
//! taken from a run of the code.

use std::fmt::Write as _;

use std::path::Path;

use cortex_core::{RepoPath, ShellCommand};

use super::{MAX_NARROW_COMMAND_CHARS, SuiteRun, narrow_command, whole_suite};
use crate::commands::Commands;

/// Result type for these tests: any failure fails the test.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The repository's top folder, as an agent on this platform names it. Nothing is read from it.
#[cfg(windows)]
const ROOT: &str = r"C:\repo";
/// The repository's top folder, as an agent on this platform names it. Nothing is read from it.
#[cfg(not(windows))]
const ROOT: &str = "/repo";

/// What `line`, started in the folder `sub` of the repository (empty for its top), kept.
fn whole_in(line: &str, sub: &str) -> Option<SuiteRun> {
    let root = Path::new(ROOT);
    let cwd = if sub.is_empty() {
        root.to_path_buf()
    } else {
        root.join(sub)
    };
    whole_suite(&Commands::default(), &ShellCommand::cut(line), root, &cwd)
}

/// A run written as one line: the runner, then the variables and launcher words, then where it
/// ran and the package and profile it names, each when it has one.
fn describe(run: &SuiteRun) -> String {
    let mut text = format!("{}:", run.runner);
    for word in run.env.iter().chain(&run.launcher) {
        text.push(' ');
        text.push_str(word);
    }
    if !run.dir.is_root() {
        let _ = write!(text, " in {}", run.dir.as_str());
    }
    if let Some(enter) = &run.enter {
        let _ = write!(text, " via {enter}");
    }
    if let Some(package) = &run.package {
        let _ = write!(text, " [package {package}]");
    }
    if run.is_release {
        text.push_str(" [release]");
    }
    text
}

/// The repository paths of `files`.
fn paths(files: &[&str]) -> Result<Vec<RepoPath>, cortex_core::PathError> {
    files.iter().map(|file| RepoPath::new(file)).collect()
}

/// What `narrow_command` writes for `files` after `line` was run in the folder `sub`.
fn narrowed(
    line: &str,
    sub: &str,
    files: &[&str],
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let Some(run) = whole_in(line, sub) else {
        return Err(format!("{line:?} was not a whole-suite run").into());
    };
    Ok(narrow_command(&run, &paths(files)?))
}

/// Whole-suite rows: a line and what it kept, as [`describe`] writes it, or nothing.
#[test]
#[allow(clippy::too_many_lines, reason = "one table of hand-written cases")]
fn whole_suite_table() {
    let rows: &[(&str, Option<&str>)] = &[
        // pytest: whole forms, options that do not select.
        ("pytest", Some("pytest: pytest")),
        ("pytest -q", Some("pytest: pytest")),
        (
            "pytest -x -v --tb=short --no-header --color=yes",
            Some("pytest: pytest"),
        ),
        ("pytest -n 4 -q", Some("pytest: pytest")),
        ("pytest -p no:cacheprovider", Some("pytest: pytest")),
        ("pytest --tb short -rA", Some("pytest: pytest")),
        // pytest: forms that select, or that are not certain.
        ("pytest tests/test_ops.py", None),
        ("pytest tests/test_ops.py::test_add", None),
        ("pytest -k add", None),
        ("pytest -xk add", None),
        // The expression joined to its flag: no word after it reads as a path, so only the flag
        // tells that the run selects.
        ("pytest -kadd", None),
        ("pytest -q -mslow", None),
        ("pytest -m slow", None),
        ("pytest --lf", None),
        ("pytest --collect-only", None),
        ("pytest --cov src", None),
        ("pytest $TESTS", None),
        // pytest: launchers.
        ("python -m pytest -q", Some("pytest: python -m pytest")),
        ("python3 -m pytest", Some("pytest: python3 -m pytest")),
        ("uv run pytest -q", Some("pytest: uv run pytest")),
        (
            "uv run python -m pytest",
            Some("pytest: uv run python -m pytest"),
        ),
        ("poetry run pytest", Some("pytest: poetry run pytest")),
        (".venv/bin/pytest -q", Some("pytest: .venv/bin/pytest")),
        ("uv run --with rich pytest", None),
        ("/usr/bin/pytest", None),
        ("timeout 60 pytest", None),
        ("env CI=1 pytest", None),
        // pytest: variables, pipes, folders, several commands.
        ("CI=1 pytest", Some("pytest: CI=1 pytest")),
        ("API_TOKEN=abc pytest", None),
        ("pytest -q 2>&1 | tail -20", Some("pytest: pytest")),
        ("cat ids | pytest", None),
        ("(cd x && pytest)", None),
        ("cd x && pytest -q", Some("pytest: pytest in x via x")),
        ("ruff check . && pytest -q", Some("pytest: pytest")),
        ("pytest -q && pytest -x", None),
        ("pytest -q; pytest -x", None),
        ("pytest && jest", None),
        // unittest.
        ("python -m unittest", Some("unittest: python -m unittest")),
        (
            "python -m unittest -v",
            Some("unittest: python -m unittest"),
        ),
        (
            "python -m unittest discover -s tests",
            Some("unittest: python -m unittest"),
        ),
        ("python -m unittest discover -s tests -p 'test_*.py'", None),
        ("python -m unittest tests.test_ops", None),
        ("python -m unittest -v -k add", None),
        // go.
        ("go test ./...", Some("go: go test")),
        ("go test", Some("go: go test")),
        ("go test -race -count=1 ./...", Some("go: go test")),
        ("go test -v -timeout 30s ./...", Some("go: go test")),
        ("go test ./pkg/a", None),
        ("go test ./cmd/...", None),
        ("go test -run TestA ./...", None),
        ("go test -args -x", None),
        ("go build ./...", None),
        // cargo.
        ("cargo test", Some("cargo: cargo test")),
        ("cargo t", Some("cargo: cargo t")),
        ("cargo test --workspace", Some("cargo: cargo test")),
        ("cargo test --all -q", Some("cargo: cargo test")),
        (
            "cargo test -p cortex-learn",
            Some("cargo: cargo test [package cortex-learn]"),
        ),
        ("cargo test --release", Some("cargo: cargo test [release]")),
        ("cargo test -- --nocapture", Some("cargo: cargo test")),
        ("cargo +nightly test", Some("cargo: cargo +nightly test")),
        ("cargo test parse", None),
        ("cargo test --lib", None),
        ("cargo test --test coordination", None),
        ("cargo test -- --ignored", None),
        ("cargo test -p a -p b", None),
        ("cargo test --features x", None),
        ("cargo nextest run", None),
        ("cargo build", None),
        // jest and vitest.
        ("npx jest", Some("jest: npx jest")),
        ("npx jest --ci --coverage", Some("jest: npx jest")),
        ("jest --maxWorkers=2", Some("jest: jest")),
        ("pnpm exec jest", Some("jest: pnpm exec jest")),
        ("yarn jest --ci", Some("jest: yarn jest")),
        ("npm exec -- jest --ci", Some("jest: npm exec -- jest")),
        ("npx jest src/a.test.ts", None),
        ("npx jest -t add", None),
        ("npx vitest run", Some("vitest: npx vitest run")),
        ("npx vitest", Some("vitest: npx vitest run")),
        ("npx vitest run --coverage", Some("vitest: npx vitest run")),
        ("npx vitest run src/a.test.ts", None),
        ("npx vitest related src/a.ts", None),
        ("npx vitest run -t add", None),
        // Package scripts do not say their runner; other checks are not tests.
        ("npm test", None),
        ("pnpm test", None),
        ("yarn test", None),
        ("npm run test", None),
        ("make test", None),
        ("tsc --noEmit", None),
        ("ls", None),
        ("if true; then pytest; fi", None),
    ];
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|(line, expected)| {
            let got = whole_in(line, "").map(|run| describe(&run));
            (got.as_deref() != *expected).then(|| format!("{line:?}: {got:?}, wanted {expected:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A line started in a folder of the repository, and one that moves, keep where the check ran.
#[test]
fn whole_suite_follows_the_folder() {
    let rows: &[(&str, &str, Option<&str>)] = &[
        ("pytest", "sub", Some("pytest: pytest in sub")),
        (
            "cd tests && pytest",
            "sub",
            Some("pytest: pytest in sub/tests via tests"),
        ),
        ("cd .. && pytest", "sub", None),
        ("cd $HOME && pytest", "", None),
        ("{ cd x; } && pytest", "", None),
        (
            "cd x; cd y; pytest",
            "",
            Some("pytest: pytest in x/y via x/y"),
        ),
    ];
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|(line, sub, expected)| {
            let got = whole_in(line, sub).map(|run| describe(&run));
            (got.as_deref() != *expected).then(|| format!("{line:?}: {got:?}, wanted {expected:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Narrow rows: a whole-suite line, the test files changed, and the command for them.
#[test]
#[allow(clippy::too_many_lines, reason = "one table of hand-written cases")]
fn narrow_command_table() -> TestResult {
    let ops = "tests/test_ops.py";
    let main = "tests/test_main.py";
    let rows: &[(&str, &[&str], Option<&str>)] = &[
        // pytest.
        ("pytest", &[ops], Some("pytest tests/test_ops.py")),
        (
            "pytest",
            &[ops, main],
            Some("pytest tests/test_ops.py tests/test_main.py"),
        ),
        (
            "python -m pytest -q",
            &[ops],
            Some("python -m pytest tests/test_ops.py"),
        ),
        (
            "uv run pytest",
            &[ops, main],
            Some("uv run pytest tests/test_ops.py tests/test_main.py"),
        ),
        ("CI=1 pytest", &[ops], Some("CI=1 pytest tests/test_ops.py")),
        ("pytest", &["README.md"], None),
        (
            "pytest",
            &["README.md", "tests/test_a.py"],
            Some("pytest tests/test_a.py"),
        ),
        (
            "pytest",
            &["tests/test ops.py"],
            Some("pytest 'tests/test ops.py'"),
        ),
        (
            "pytest",
            &["tests/it's.py"],
            Some("pytest 'tests/it'\\''s.py'"),
        ),
        ("pytest", &["-x.py"], None),
        ("pytest", &[], None),
        ("pytest", &[ops, ops], Some("pytest tests/test_ops.py")),
        (
            "cd x && pytest",
            &["x/tests/test_a.py", "y/tests/test_b.py"],
            Some("cd x && pytest tests/test_a.py"),
        ),
        // unittest.
        (
            "python -m unittest",
            &[ops],
            Some("python -m unittest tests.test_ops"),
        ),
        (
            "python -m unittest",
            &[ops, main],
            Some("python -m unittest tests.test_ops tests.test_main"),
        ),
        ("python -m unittest", &["tests/test-ops.py"], None),
        ("python -m unittest", &["pkg/__init__.py"], None),
        (
            "python -m unittest",
            &["test_x.py"],
            Some("python -m unittest test_x"),
        ),
        // go.
        (
            "go test ./...",
            &["pkg/a/a_test.go", "pkg/b/b_test.go"],
            Some("go test ./pkg/a ./pkg/b"),
        ),
        (
            "go test ./...",
            &["pkg/a/a_test.go", "pkg/a/b_test.go"],
            Some("go test ./pkg/a"),
        ),
        ("go test ./...", &["a_test.go"], Some("go test .")),
        ("go test ./...", &["pkg/a/a.go"], None),
        (
            "go test -race ./...",
            &["pkg/a/a_test.go"],
            Some("go test ./pkg/a"),
        ),
        (
            "cd svc && go test ./...",
            &["svc/x/x_test.go"],
            Some("cd svc && go test ./x"),
        ),
        // cargo.
        (
            "cargo test",
            &["tests/api.rs"],
            Some("cargo test --test api"),
        ),
        (
            "cargo test",
            &["tests/api.rs", "tests/cli.rs"],
            Some("cargo test --test api --test cli"),
        ),
        ("cargo test", &["src/lib.rs"], None),
        ("cargo test", &["src/tests/x.rs"], None),
        ("cargo test", &["tests/common/mod.rs"], None),
        (
            "cargo test",
            &["crates/learn/tests/task_end.rs"],
            Some("cargo test --manifest-path crates/learn/Cargo.toml --test task_end"),
        ),
        (
            "cargo test",
            &["crates/a/tests/x.rs", "crates/b/tests/y.rs"],
            Some("cargo test --manifest-path crates/a/Cargo.toml --test x"),
        ),
        (
            "cargo test",
            &["tests/api.rs", "crates/a/tests/x.rs"],
            Some("cargo test --test api"),
        ),
        (
            "cargo test -p learn",
            &["crates/learn/tests/t.rs", "crates/other/tests/u.rs"],
            Some("cargo test -p learn --test t"),
        ),
        (
            "cargo test -p my-crate",
            &["crates/my_crate/tests/t.rs"],
            Some("cargo test -p my-crate --test t"),
        ),
        (
            "cargo test --release",
            &["tests/api.rs"],
            Some("cargo test --release --test api"),
        ),
        (
            "cargo +nightly test",
            &["tests/a.rs"],
            Some("cargo +nightly test --test a"),
        ),
        // jest and vitest.
        (
            "npx jest",
            &["src/a.test.ts", "src/b.test.ts"],
            Some("npx jest src/a.test.ts src/b.test.ts"),
        ),
        ("jest --ci", &["src/a.test.ts"], Some("jest src/a.test.ts")),
        (
            "pnpm exec jest",
            &["src/a.test.ts"],
            Some("pnpm exec jest src/a.test.ts"),
        ),
        ("npx jest", &["src/[id].test.tsx"], None),
        ("npx jest", &["src/a.py"], None),
        (
            "npx vitest",
            &["src/a.test.ts"],
            Some("npx vitest run src/a.test.ts"),
        ),
        (
            "npx vitest run",
            &["src/[id].test.tsx"],
            Some("npx vitest run 'src/[id].test.tsx'"),
        ),
        ("npx vitest run --coverage", &["a.py"], None),
    ];
    let mut wrong = Vec::new();
    for (line, files, expected) in rows {
        let got = narrowed(line, "", files)?;
        if got.as_deref() != *expected {
            wrong.push(format!("{line:?} {files:?}: {got:?}, wanted {expected:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    Ok(())
}

/// A suite run from a folder below the top names files from that folder.
#[test]
fn narrow_command_names_files_from_the_run_folder() -> TestResult {
    let got = narrowed("pytest", "sub", &["sub/tests/t.py", "other/t.py"])?;
    assert_eq!(got.as_deref(), Some("pytest tests/t.py"));
    Ok(())
}

/// Three long names do not fit in 200 characters, two do (7 + 94 + 1 + 94 = 196), and one name
/// of 200 characters alone does not.
#[test]
fn narrow_command_drops_files_from_the_end_to_fit() -> TestResult {
    assert_eq!(MAX_NARROW_COMMAND_CHARS, 200);
    let name = |letter: &str| format!("tests/test_{}.py", letter.repeat(80));
    let (a, b, c) = (name("a"), name("b"), name("c"));
    let got = narrowed("pytest", "", &[&a, &b, &c])?;
    assert_eq!(got, Some(format!("pytest {a} {b}")));
    let long = format!("tests/{}.py", "a".repeat(200));
    assert_eq!(narrowed("pytest", "", &[&long])?, None);
    Ok(())
}

/// A run of a runner without a way to name files writes nothing.
#[test]
fn narrow_command_has_nothing_for_an_unknown_runner() -> TestResult {
    let Some(mut run) = whole_in("pytest", "") else {
        return Err("pytest was not a whole-suite run".into());
    };
    run.runner = "npm";
    assert_eq!(narrow_command(&run, &paths(&["tests/test_ops.py"])?), None);
    Ok(())
}

/// Words are written as the shell passes them.
#[test]
fn quoting_writes_words_as_the_shell_passes_them() {
    let rows = [
        ("tests/test_ops.py", "tests/test_ops.py"),
        ("a b", "'a b'"),
        ("", "''"),
        ("a;b", "'a;b'"),
        ("[id]", "'[id]'"),
        ("it's", "'it'\\''s'"),
    ];
    for (text, quoted) in rows {
        assert_eq!(super::quote::word(text), quoted, "{text:?}");
    }
}
