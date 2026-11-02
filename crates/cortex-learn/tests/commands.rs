//! Telling what a shell command line ran, through the crate's entry point: real command lines of
//! each runner, with `cd`, variable assignments, `&&`, pipes and the programs that run another;
//! which commands a line's exit status vouches for; git restores and the paths they name; the
//! user's patterns; and lines the reader does not follow. Every expected value is read off the
//! line by hand.

use std::path::{Path, PathBuf};

use cortex_core::{CommandKind, RepoPath, ShellCommand};
use cortex_learn::{CONFIGURED, Check, Commands, Ran, Restore};

/// Result type for these tests: any failure fails the test.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The repository's top folder, as an agent on this platform names it. Nothing is read from it.
#[cfg(windows)]
const ROOT: &str = r"C:\repo";
/// The repository's top folder, as an agent on this platform names it. Nothing is read from it.
#[cfg(not(windows))]
const ROOT: &str = "/repo";

/// The folder `sub` under the top, or the top itself for an empty `sub`.
fn folder(sub: &str) -> PathBuf {
    sub.split('/')
        .filter(|name| !name.is_empty())
        .fold(PathBuf::from(ROOT), |path, name| path.join(name))
}

/// What `line`, run in the folder `sub` under the top, is told to have run, by `commands`.
fn told_by(commands: &Commands, line: &str, sub: &str) -> Vec<Ran> {
    commands.recognise(&ShellCommand::cut(line), Path::new(ROOT), &folder(sub))
}

/// What `line`, run at the top with no patterns of the user's, is told to have run.
fn told(line: &str) -> Result<Vec<Ran>, Box<dyn std::error::Error>> {
    Ok(told_by(&Commands::new(&[])?, line, ""))
}

/// A check of `runner` and `kind`.
fn check(runner: &'static str, kind: CommandKind) -> Ran {
    Ran::Check(Check { runner, kind })
}

/// A restore of `paths`.
fn restore_of(paths: &[&str]) -> Result<Ran, Box<dyn std::error::Error>> {
    Ok(Ran::Restore(Restore::Paths(
        paths
            .iter()
            .map(|path| RepoPath::new(path))
            .collect::<Result<_, _>>()?,
    )))
}

/// Each runner's common command lines are told as its check, wrappers, assignments and options
/// passed over.
#[test]
fn each_runner_is_told_by_its_command_line() -> TestResult {
    use CommandKind::{Build, Lint, Test, TypeCheck};
    let cases: &[(&str, &str, CommandKind)] = &[
        ("npm test", "npm", Test),
        ("npm t", "npm", Test),
        ("npm run test:unit -- --watch=false", "npm", Test),
        ("npm --prefix web test", "npm", Test),
        ("npm run build", "npm", Build),
        ("npm run typecheck", "npm", TypeCheck),
        ("npm run lint", "npm", Lint),
        ("pnpm --filter web test", "pnpm", Test),
        ("pnpm build", "pnpm", Build),
        ("pnpm vitest run", "vitest", Test),
        ("pnpm exec tsc --noEmit", "tsc", TypeCheck),
        ("yarn test", "yarn", Test),
        ("yarn jest --coverage", "jest", Test),
        ("yarn run lint", "yarn", Lint),
        ("bun test", "bun", Test),
        ("bun run build", "bun", Build),
        ("bunx vitest", "vitest", Test),
        ("vitest run src/math.test.ts", "vitest", Test),
        ("npx jest src/", "jest", Test),
        ("npx --yes jest", "jest", Test),
        ("./node_modules/.bin/jest -t parse", "jest", Test),
        ("pytest -q tests/", "pytest", Test),
        ("py.test", "pytest", Test),
        ("python -m pytest -x", "pytest", Test),
        ("python3 -m pytest -q tests/test_fail.py", "pytest", Test),
        ("uv run pytest", "pytest", Test),
        ("poetry run pytest -k parse", "pytest", Test),
        ("python -m unittest -q tests.test_ops", "unittest", Test),
        ("go test ./...", "go", Test),
        ("go build ./cmd/server", "go", Build),
        ("cargo test", "cargo", Test),
        ("cargo test -p cortex-learn --lib", "cargo", Test),
        ("cargo +nightly t", "cargo", Test),
        ("cargo nextest run --workspace", "cargo", Test),
        ("cargo check --all-targets", "cargo", TypeCheck),
        ("cargo clippy -- -D warnings", "cargo", Lint),
        ("cargo build --release", "cargo", Build),
        ("mvn -q test", "mvn", Test),
        ("./mvnw clean verify", "mvn", Test),
        ("mvn compile", "mvn", Build),
        ("./gradlew :app:test --info", "gradle", Test),
        ("gradle build", "gradle", Test),
        ("bundle exec rspec spec/models", "rspec", Test),
        ("vendor/bin/phpunit --filter Parser", "phpunit", Test),
        ("dotnet test", "dotnet", Test),
        ("dotnet build -c Release", "dotnet", Build),
        ("make test", "make", Test),
        ("make -C sub check", "make", Test),
        ("make", "make", Build),
        ("make -j 4 all", "make", Build),
        ("tsc --noEmit -p .", "tsc", TypeCheck),
        ("tsc -b", "tsc", Build),
        ("eslint src --ext .ts", "eslint", Lint),
        ("ruff check .", "ruff", Lint),
        ("mypy src", "mypy", TypeCheck),
        ("cargo.exe test", "cargo", Test),
        ("RUST_LOG=debug cargo test -q", "cargo", Test),
        ("env CI=1 npm test", "npm", Test),
        ("timeout 300 cargo test", "cargo", Test),
        ("timeout -k 5 60 pytest", "pytest", Test),
        ("time pytest", "pytest", Test),
        ("nice -n 10 make test", "make", Test),
    ];
    for &(line, runner, kind) in cases {
        assert_eq!(told(line)?, [check(runner, kind)], "{line}");
    }
    Ok(())
}

/// Commands that run no check Cortex knows tell nothing: other commands of the same programs, a
/// program asked for its help or version, and commands that only name a runner.
#[test]
fn other_commands_tell_nothing() -> TestResult {
    let lines = [
        "npm install",
        "npm build",
        "pnpm install --frozen-lockfile",
        "go vet ./...",
        "cargo fmt --check",
        "cargo xtask verify",
        "cargo nextest list",
        "mvn clean",
        "gradle dependencies",
        "make clean",
        "tsc --init",
        "python script.py",
        "python -c 'import pytest'",
        "cargo test --help",
        "pytest --version",
        "make -h",
        "command -v cargo",
        "echo cargo test",
        "git status",
        "$CARGO test",
        "ls tests",
    ];
    for line in lines {
        assert_eq!(told(line)?, [], "{line}");
    }
    Ok(())
}

/// Only what the line's exit status vouches for is told: a command that ends its pipeline, in the
/// last and-or list, run whenever what came before it in that list succeeded and followed by `&&`
/// alone, neither inverted nor in the background. A group's status is its last list's.
#[test]
fn only_what_the_exit_status_vouches_for_is_told() -> TestResult {
    use CommandKind::{Lint, Test, TypeCheck};
    let cargo_test = check("cargo", Test);
    let cases: Vec<(&str, Vec<Ran>)> = vec![
        (
            "cd crates/cortex-learn && cargo test",
            vec![cargo_test.clone()],
        ),
        ("cargo test && echo passed", vec![cargo_test.clone()]),
        (
            "cargo check && cargo clippy -- -D warnings",
            vec![check("cargo", TypeCheck), check("cargo", Lint)],
        ),
        (
            "cargo build &&\n  cargo test",
            vec![check("cargo", CommandKind::Build), cargo_test.clone()],
        ),
        ("cd sub; cargo test", vec![cargo_test.clone()]),
        ("cargo test # the whole workspace", vec![cargo_test.clone()]),
        ("cat input.txt | cargo test", vec![cargo_test.clone()]),
        (
            "false || cargo build && cargo test",
            vec![cargo_test.clone()],
        ),
        ("(cd sub && pytest)", vec![check("pytest", Test)]),
        ("{ cd sub; pytest; }", vec![check("pytest", Test)]),
        ("cargo test 2>&1", vec![cargo_test.clone()]),
        ("cargo test > test.log 2>&1", vec![cargo_test.clone()]),
        ("cargo test 2>&1 | tail -20", vec![]),
        ("python -m unittest -q 2>&1 | tail -20", vec![]),
        ("cargo test; echo done", vec![]),
        ("cargo test || true", vec![]),
        ("cargo test || cargo build", vec![]),
        ("true || cargo test", vec![]),
        ("! cargo test", vec![]),
        ("cargo test &", vec![]),
        ("cargo test & wait", vec![]),
        ("(cargo test) | tee log", vec![]),
    ];
    for (line, expected) in cases {
        assert_eq!(told(line)?, expected, "{line}");
    }
    Ok(())
}

/// A line the reader does not follow, or that is not well formed, tells nothing; a here-document's
/// lines are not commands.
#[test]
fn lines_the_reader_does_not_follow_tell_nothing() -> TestResult {
    let lines = [
        "if cargo test; then echo ok; fi",
        "for crate in a b; do cargo test -p $crate; done",
        "while true; do cargo test; done",
        "test() { cargo test; }",
        "cargo test 'unclosed",
        "cargo test \"unclosed",
        "cargo test $(unclosed",
        "(cargo test",
        "cargo test )",
        "&& cargo test",
        "cargo test |",
        "case x in a) cargo test;; esac",
        "cat <<EOF\ncargo test",
        "cat <<'EOF' > run.sh\ncargo test\nEOF\nsh run.sh",
    ];
    for line in lines {
        assert_eq!(told(line)?, [], "{line:?}");
    }
    assert_eq!(
        told("cat <<EOF > notes.txt\nnot a command\nEOF\ncargo test")?,
        [check("cargo", CommandKind::Test)]
    );
    Ok(())
}

/// Git restores name the paths they restore, from the folder they run in (moved by `cd` and
/// `git -C`): `git restore` (not of the index alone), `git checkout` with `--`, with a commit or
/// not; `git reset --hard` restores everything, as does a path naming the top. Paths outside the
/// repository, patterns and words that expand are left out; a restore with no path left, of parts
/// of files, or of another repository, is not told.
#[test]
fn git_restores_name_their_paths() -> TestResult {
    let commands = Commands::new(&[])?;
    let everything = Ran::Restore(Restore::Everything);
    let cases: Vec<(&str, &str, Vec<Ran>)> = vec![
        (
            "git checkout -- src/a.py",
            "",
            vec![restore_of(&["src/a.py"])?],
        ),
        (
            "git checkout HEAD~1 -- b.py a.py b.py",
            "",
            vec![restore_of(&["a.py", "b.py"])?],
        ),
        ("git restore a.py", "src", vec![restore_of(&["src/a.py"])?]),
        (
            "cd src && git restore ../lib/b.py c.py",
            "",
            vec![restore_of(&["lib/b.py", "src/c.py"])?],
        ),
        (
            "git restore --source=HEAD~2 --worktree -- a.py",
            "",
            vec![restore_of(&["a.py"])?],
        ),
        ("git restore -SW a.py", "", vec![restore_of(&["a.py"])?]),
        ("git restore -s main a.py", "", vec![restore_of(&["a.py"])?]),
        (
            "git -C src checkout -- x.py",
            "",
            vec![restore_of(&["src/x.py"])?],
        ),
        (
            "(cd src) && git checkout -- a.py",
            "",
            vec![restore_of(&["a.py"])?],
        ),
        (
            "cd src | cat && git checkout -- a.py",
            "",
            vec![restore_of(&["a.py"])?],
        ),
        (
            "{ cd src; } && git checkout -- a.py",
            "",
            vec![restore_of(&["src/a.py"])?],
        ),
        (
            "git --no-pager checkout -- docs",
            "",
            vec![restore_of(&["docs"])?],
        ),
        ("git checkout -- .", "src", vec![restore_of(&["src"])?]),
        ("git checkout -- .", "", vec![everything.clone()]),
        ("git restore :/", "src", vec![everything.clone()]),
        (
            "git restore :/lib/b.py",
            "src",
            vec![restore_of(&["lib/b.py"])?],
        ),
        ("git reset --hard", "", vec![everything.clone()]),
        ("git reset --hard HEAD~1", "src", vec![everything.clone()]),
        (
            "git checkout -- a.py ../outside.py",
            "",
            vec![restore_of(&["a.py"])?],
        ),
        ("git restore --staged a.py", "", vec![]),
        ("git restore -S a.py", "", vec![]),
        ("git restore -p a.py", "", vec![]),
        ("git checkout -p -- a.py", "", vec![]),
        ("git checkout main", "", vec![]),
        ("git checkout a.py", "", vec![]),
        ("git checkout -b fix -- a.py", "", vec![]),
        ("git checkout -- '*.py'", "", vec![]),
        ("git checkout -- $FILE", "", vec![]),
        ("git checkout -- ../outside.py", "", vec![]),
        ("git reset --soft HEAD~1", "", vec![]),
        ("git stash", "", vec![]),
        ("git --git-dir=/elsewhere/.git checkout -- a.py", "", vec![]),
        ("cd && git checkout -- a.py", "", vec![]),
        ("git checkout -- a.py; echo restored", "", vec![]),
        (
            "git checkout -- a.py && cargo test",
            "",
            vec![restore_of(&["a.py"])?, check("cargo", CommandKind::Test)],
        ),
    ];
    for (line, sub, expected) in cases {
        assert_eq!(told_by(&commands, line, sub), expected, "{line} in {sub:?}");
    }
    Ok(())
}

/// The user's patterns name tests, matched against a command's words after its assignments,
/// joined by single spaces, where the line vouches for the command; the runners Cortex knows come
/// first.
#[test]
fn the_user_s_patterns_name_tests() -> TestResult {
    let commands = Commands::new(&[r"^\./scripts/test\.sh".to_owned(), "^make e2e$".to_owned()])?;
    let configured = check(CONFIGURED, CommandKind::Test);
    let cases: Vec<(&str, Vec<Ran>)> = vec![
        ("./scripts/test.sh --fast", vec![configured.clone()]),
        ("CI=1 ./scripts/test.sh", vec![configured.clone()]),
        ("make   e2e", vec![configured.clone()]),
        ("make e2e | tee log", vec![]),
        ("./scripts/build.sh", vec![]),
        ("cargo test", vec![check("cargo", CommandKind::Test)]),
    ];
    for (line, expected) in cases {
        assert_eq!(told_by(&commands, line, ""), expected, "{line}");
    }
    Ok(())
}

/// A pattern that is not a regular expression is refused, naming it.
#[test]
fn a_pattern_that_is_not_a_regular_expression_is_refused() {
    let refused = Commands::new(&["(unclosed".to_owned()]).err();
    assert!(refused.is_some_and(|error| {
        error
            .to_string()
            .starts_with(r#"tests.extra_patterns: "(unclosed""#)
    }),);
}

/// Lines built from these commands and operators, for the properties below.
const POOL: &[&str] = &[
    "cargo test",
    "npm test",
    "pytest -q",
    "make",
    "echo ok",
    "ls src",
    "git status",
    "cd sub",
    "true",
];

/// What joins the commands of a built line.
const JOINS: &[&str] = &[" && ", " || ", "; ", " | ", " & ", "\n"];

/// A line of one to six commands from [`POOL`], joined by [`JOINS`].
fn built_line() -> impl proptest::strategy::Strategy<Value = String> {
    use proptest::prelude::*;
    (
        proptest::sample::select(POOL),
        proptest::collection::vec(
            (
                proptest::sample::select(JOINS),
                proptest::sample::select(POOL),
            ),
            0..6,
        ),
    )
        .prop_map(|(first, rest)| {
            rest.into_iter()
                .fold(first.to_owned(), |line, (join, command)| {
                    format!("{line}{join}{command}")
                })
        })
}

proptest::proptest! {
    /// Any text up to the length an event keeps reads without a panic, and twice the same.
    #[test]
    fn any_line_reads_the_same_twice(line in "(?s).{0,1000}") {
        let commands = Commands::new(&[]).map_err(|error| {
            proptest::test_runner::TestCaseError::fail(error.to_string())
        })?;
        let first = told_by(&commands, &line, "src");
        proptest::prop_assert_eq!(first, told_by(&commands, &line, "src"));
    }

    /// A line followed by `; true` exits as `true` does, so it vouches for nothing; followed by
    /// `&& cargo test`, it vouches for what it did and for `cargo test`; in parentheses or braces,
    /// for what it did alone.
    #[test]
    fn what_comes_after_a_line_decides_what_it_vouches_for(line in built_line()) {
        let commands = Commands::new(&[]).map_err(|error| {
            proptest::test_runner::TestCaseError::fail(error.to_string())
        })?;
        let alone = told_by(&commands, &line, "");
        proptest::prop_assert_eq!(told_by(&commands, &format!("{line}; true"), ""), vec![]);
        let mut then_tested = alone.clone();
        then_tested.push(check("cargo", CommandKind::Test));
        proptest::prop_assert_eq!(
            told_by(&commands, &format!("{line} && cargo test"), ""),
            then_tested
        );
        proptest::prop_assert_eq!(told_by(&commands, &format!("({line})"), ""), alone.clone());
        proptest::prop_assert_eq!(told_by(&commands, &format!("{{ {line}; }}"), ""), alone);
    }
}

/// Groups inside groups are followed 16 deep, which no real line reaches; a line nesting deeper,
/// to the length an event keeps, tells nothing, and reading it does not exhaust the stack.
#[test]
fn groups_nested_past_sixteen_tell_nothing() -> TestResult {
    let nested = |depth: usize| format!("{}cargo test{}", "(".repeat(depth), ")".repeat(depth));
    assert_eq!(told(&nested(16))?, [check("cargo", CommandKind::Test)]);
    assert_eq!(told(&nested(17))?, []);
    assert_eq!(told(&"(".repeat(1_000))?, []);
    assert_eq!(told(&"{ ".repeat(500))?, []);
    Ok(())
}
