//! Real command lines, typed by hand, against what [`whole_suite`] keeps of them and what
//! [`narrow_command`] writes from that: each runner's whole and selecting forms, the launchers
//! agents use, the options repeated, the lines that are uncertain, the folder a `cd` moves to, the
//! files a command runs, the quoting and the cap.
//! Every expected value is written out from the rules in the module's documentation, never
//! taken from a run of the code.

use std::fmt::Write as _;

use std::path::Path;

use cortex_core::{RepoPath, ShellCommand};

use super::{MAX_NARROW_COMMAND_CHARS, SuiteRun, narrow_command, whole_suite};
use crate::commands::Commands;

/// Result type for these tests: any failure fails the test.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A narrow command line and the files it runs.
type Written = (String, Vec<String>);

/// A row of files asked for: the line, the folder it ran in, the files, the command and the files
/// it runs.
type CoverageRow<'a> = (&'a str, &'a str, &'a [&'a str], &'a str, &'a [&'a str]);

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
/// ran, the package and profile it names, its options and its test binary's arguments, each when
/// it has one.
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
    if !run.options.is_empty() {
        let _ = write!(text, " [options {}]", run.options.join(" "));
    }
    if !run.test_args.is_empty() {
        let _ = write!(text, " [test args {}]", run.test_args.join(" "));
    }
    text
}

/// The repository paths of `files`.
fn paths(files: &[&str]) -> Result<Vec<RepoPath>, cortex_core::PathError> {
    files.iter().map(|file| RepoPath::new(file)).collect()
}

/// The command line `narrow_command` writes for `files` after `line` was run in the folder `sub`.
fn narrowed(
    line: &str,
    sub: &str,
    files: &[&str],
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    Ok(narrowed_with_files(line, sub, files)?.map(|(line, _)| line))
}

/// The command line `narrow_command` writes for `files` after `line` was run in the folder `sub`,
/// with the files it says the command runs.
fn narrowed_with_files(
    line: &str,
    sub: &str,
    files: &[&str],
) -> Result<Option<Written>, Box<dyn std::error::Error>> {
    let Some(run) = whole_in(line, sub) else {
        return Err(format!("{line:?} was not a whole-suite run").into());
    };
    Ok(narrow_command(&run, &paths(files)?).map(|command| {
        let runs = command
            .files
            .iter()
            .map(|file| file.as_str().to_owned())
            .collect();
        (command.line, runs)
    }))
}

/// Whole-suite rows: a line and what it kept, as [`describe`] writes it, or nothing.
#[test]
#[allow(clippy::too_many_lines, reason = "one table of hand-written cases")]
fn whole_suite_table() {
    let rows: &[(&str, Option<&str>)] = &[
        // pytest: whole forms, options that do not select.
        ("pytest", Some("pytest: pytest")),
        ("pytest -q", Some("pytest: pytest [options -q]")),
        (
            "pytest -x -v --tb=short --no-header --color=yes",
            Some("pytest: pytest [options -x -v --tb=short --no-header --color=yes]"),
        ),
        (
            "pytest --import-mode=importlib -p no:randomly",
            Some("pytest: pytest [options --import-mode=importlib -p no:randomly]"),
        ),
        ("pytest -n 4 -q", Some("pytest: pytest [options -n 4 -q]")),
        (
            "pytest -p no:cacheprovider",
            Some("pytest: pytest [options -p no:cacheprovider]"),
        ),
        (
            "pytest --tb short -rA",
            Some("pytest: pytest [options --tb short -rA]"),
        ),
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
        // `--cov` takes the next word as its value, as pytest-cov's optional value does; coverage
        // is not repeated.
        ("pytest --cov src", Some("pytest: pytest")),
        (
            "pytest --cov=src --cov-fail-under=90 --junitxml=out.xml -q",
            Some("pytest: pytest [options -q]"),
        ),
        // A flag pytest has that is not listed, or a short letter that is not, tells nothing:
        // `--sw-skip` selects, `--fixtures` and `-h` run no test.
        ("pytest --sw-skip", None),
        ("pytest --fixtures", None),
        ("pytest -h", None),
        ("pytest --some-plugin-flag", None),
        ("pytest --verbose=2", None),
        ("pytest $TESTS", None),
        // pytest: launchers.
        (
            "python -m pytest -q",
            Some("pytest: python -m pytest [options -q]"),
        ),
        ("python3 -m pytest", Some("pytest: python3 -m pytest")),
        (
            "uv run pytest -q",
            Some("pytest: uv run pytest [options -q]"),
        ),
        (
            "uv run python -m pytest",
            Some("pytest: uv run python -m pytest"),
        ),
        ("poetry run pytest", Some("pytest: poetry run pytest")),
        (
            ".venv/bin/pytest -q",
            Some("pytest: .venv/bin/pytest [options -q]"),
        ),
        ("uv run --with rich pytest", None),
        ("/usr/bin/pytest", None),
        ("timeout 60 pytest", None),
        ("env CI=1 pytest", None),
        // pytest: variables, pipes, folders, several commands.
        ("CI=1 pytest", Some("pytest: CI=1 pytest")),
        ("API_TOKEN=abc pytest", None),
        (
            "pytest -q 2>&1 | tail -20",
            Some("pytest: pytest [options -q]"),
        ),
        ("cat ids | pytest", None),
        ("(cd x && pytest)", None),
        (
            "cd x && pytest -q",
            Some("pytest: pytest in x via x [options -q]"),
        ),
        (
            "ruff check . && pytest -q",
            Some("pytest: pytest [options -q]"),
        ),
        ("pytest -q && pytest -x", None),
        ("pytest -q; pytest -x", None),
        ("pytest && jest", None),
        // unittest.
        ("python -m unittest", Some("unittest: python -m unittest")),
        (
            "python -m unittest -v",
            Some("unittest: python -m unittest [options -v]"),
        ),
        (
            "python -m unittest discover -v",
            Some("unittest: python -m unittest [options -v]"),
        ),
        (
            "python -m unittest --durations 5",
            Some("unittest: python -m unittest [options --durations 5]"),
        ),
        // Another start or top folder changes how modules import.
        ("python -m unittest discover -s tests", None),
        ("python -m unittest discover -t .", None),
        ("python -m unittest discover -s tests -p 'test_*.py'", None),
        ("python -m unittest tests.test_ops", None),
        ("python -m unittest -v -k add", None),
        // go.
        ("go test ./...", Some("go: go test")),
        ("go test", Some("go: go test")),
        (
            "go test -race -count=1 ./...",
            Some("go: go test [options -race -count=1]"),
        ),
        (
            "go test -v -timeout 30s ./...",
            Some("go: go test [options -v -timeout 30s]"),
        ),
        (
            "go test -tags integration ./...",
            Some("go: go test [options -tags integration]"),
        ),
        (
            "go test -race -coverprofile=c.out -coverpkg ./... -json ./...",
            Some("go: go test [options -race]"),
        ),
        ("go test -c ./...", None),
        ("go test -n ./...", None),
        ("go test ./pkg/a", None),
        ("go test ./cmd/...", None),
        ("go test -run TestA ./...", None),
        ("go test -args -x", None),
        ("go build ./...", None),
        // cargo.
        ("cargo test", Some("cargo: cargo test")),
        ("cargo t", Some("cargo: cargo t")),
        ("cargo test --workspace", Some("cargo: cargo test")),
        (
            "cargo test --all -q",
            Some("cargo: cargo test [options -q]"),
        ),
        (
            "cargo test --locked -j 4",
            Some("cargo: cargo test [options --locked -j 4]"),
        ),
        (
            "cargo test -p cortex-learn",
            Some("cargo: cargo test [package cortex-learn]"),
        ),
        ("cargo test --release", Some("cargo: cargo test [release]")),
        (
            "cargo test -- --nocapture",
            Some("cargo: cargo test [test args -- --nocapture]"),
        ),
        (
            "cargo test -- --test-threads 1",
            Some("cargo: cargo test [test args -- --test-threads 1]"),
        ),
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
        (
            "npx jest --ci --coverage",
            Some("jest: npx jest [options --ci]"),
        ),
        (
            "jest --maxWorkers=2",
            Some("jest: jest [options --maxWorkers=2]"),
        ),
        ("pnpm exec jest", Some("jest: pnpm exec jest")),
        ("yarn jest --ci", Some("jest: yarn jest [options --ci]")),
        (
            "npm exec -- jest --ci",
            Some("jest: npm exec -- jest [options --ci]"),
        ),
        ("npx jest src/a.test.ts", None),
        ("npx jest -t add", None),
        ("npx vitest run", Some("vitest: npx vitest run")),
        ("npx vitest", Some("vitest: npx vitest run")),
        ("npx vitest run --coverage", Some("vitest: npx vitest run")),
        (
            "npx vitest run --reporter dot --silent",
            Some("vitest: npx vitest run [options --silent]"),
        ),
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
            Some("python -m pytest -q tests/test_ops.py"),
        ),
        (
            "python -m pytest --import-mode=importlib -x",
            &[ops],
            Some("python -m pytest --import-mode=importlib -x tests/test_ops.py"),
        ),
        (
            "pytest -W error::DeprecationWarning",
            &[ops],
            Some("pytest -W error::DeprecationWarning tests/test_ops.py"),
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
        ("pytest", &["tests/it's.py"], None),
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
            Some("go test -race ./pkg/a"),
        ),
        (
            "go test -tags integration ./...",
            &["pkg/a/a_test.go"],
            Some("go test -tags integration ./pkg/a"),
        ),
        // Windows PowerShell splits an option at its first `.`: such a word is quoted.
        (
            "go test -timeout=1.5m ./...",
            &["pkg/a/a_test.go"],
            Some("go test '-timeout=1.5m' ./pkg/a"),
        ),
        // No quote is portable inside a quoted word: an option holding one gives no command.
        ("pytest \"-W=error:it's\"", &[ops], None),
        (
            "python -m unittest -v",
            &[ops],
            Some("python -m unittest -v tests.test_ops"),
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
        (
            "cargo test --workspace --locked -- --test-threads=1",
            &["tests/a.rs"],
            Some("cargo test --locked --test a -- --test-threads=1"),
        ),
        // jest and vitest.
        (
            "npx jest",
            &["src/a.test.ts", "src/b.test.ts"],
            Some("npx jest src/a.test.ts src/b.test.ts"),
        ),
        (
            "jest --ci",
            &["src/a.test.ts"],
            Some("jest --ci src/a.test.ts"),
        ),
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

/// Words are written as a POSIX shell and PowerShell both pass them, or not at all.
#[test]
fn quoting_writes_words_as_both_shells_pass_them() {
    let rows = [
        ("tests/test_ops.py", Some("tests/test_ops.py")),
        ("./pkg/a", Some("./pkg/a")),
        ("a b", Some("'a b'")),
        ("", Some("''")),
        ("a;b", Some("'a;b'")),
        ("[id]", Some("'[id]'")),
        ("--tb=short", Some("--tb=short")),
        ("no:cacheprovider", Some("no:cacheprovider")),
        ("a,b", Some("'a,b'")),
        // An option holding a `.` is split there by Windows PowerShell unless quoted.
        ("-coverpkg=./...", Some("'-coverpkg=./...'")),
        ("--junitxml=out.xml", Some("'--junitxml=out.xml'")),
        // No escape of a quote reads the same in both shells, nor does a control character.
        ("it's", None),
        ("a\tb", None),
    ];
    for (text, quoted) in rows {
        assert_eq!(super::quote::word(text).as_deref(), quoted, "{text:?}");
    }
}

/// A command names only the files it runs: of the files asked for, those the runner cannot be told
/// by (a README to pytest, a unit test inside `src` to cargo), those of another package or outside
/// the folder the suite ran in are not among them, and two Go files of one package both are.
#[test]
fn narrow_command_says_which_files_it_runs() -> TestResult {
    let rows: &[CoverageRow<'_>] = &[
        (
            "pytest",
            "",
            &["tests/test_a.py", "README.md", "tests/test_b.py"],
            "pytest tests/test_a.py tests/test_b.py",
            &["tests/test_a.py", "tests/test_b.py"],
        ),
        (
            "cargo test",
            "",
            &["crates/x/src/a/tests.rs", "crates/x/tests/b.rs"],
            "cargo test --manifest-path crates/x/Cargo.toml --test b",
            &["crates/x/tests/b.rs"],
        ),
        (
            "cargo test",
            "",
            &["crates/a/tests/x.rs", "tests/x.rs"],
            "cargo test --manifest-path crates/a/Cargo.toml --test x",
            &["crates/a/tests/x.rs"],
        ),
        (
            "go test ./...",
            "",
            &["pkg/a/a_test.go", "pkg/a/b_test.go"],
            "go test ./pkg/a",
            &["pkg/a/a_test.go", "pkg/a/b_test.go"],
        ),
        (
            "pytest",
            "sub",
            &["other/tests/test_a.py", "sub/tests/test_b.py"],
            "pytest tests/test_b.py",
            &["sub/tests/test_b.py"],
        ),
    ];
    let mut wrong = Vec::new();
    for (line, sub, files, command, runs) in rows {
        let got = narrowed_with_files(line, sub, files)?;
        let expected = Some((
            (*command).to_owned(),
            runs.iter().map(|run| (*run).to_owned()).collect(),
        ));
        if got != expected {
            wrong.push(format!("{line:?} {files:?}: {got:?}, wanted {expected:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    Ok(())
}

/// A file dropped so that the command fits is not among the files it runs.
#[test]
fn a_file_dropped_to_fit_is_not_run() -> TestResult {
    let name = |letter: &str| format!("tests/test_{}.py", letter.repeat(80));
    let (a, b, c) = (name("a"), name("b"), name("c"));
    let got = narrowed_with_files("pytest", "", &[&a, &b, &c])?;
    assert_eq!(
        got,
        Some((format!("pytest {a} {b}"), vec![a.clone(), b.clone()]))
    );
    Ok(())
}
