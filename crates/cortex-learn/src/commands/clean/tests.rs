//! What a handoff keeps of real command lines, typed by hand: each detector, each way of setting
//! the environment on bash, zsh and `PowerShell`, harmless arguments kept as written, paths from
//! the machine's top rewritten or refused, the fallbacks and the cap; and properties that no
//! secret of any detector's shape, nor a user's name in a path, survives, wherever it stands in a
//! command or its environment.
//!
//! A fixture writes each secret as `«name»`, filled in from [`secret`] when the test runs. Every
//! secret is assembled there from pieces, so that no token of a service's shape is written whole
//! in the source, where a secret scanner would rightly refuse it.

use std::path::Path;

use cortex_core::ShellCommand;
use proptest::prelude::*;

use super::{Cleaned, REDACTED, clean_command};
use crate::commands::{CONFIGURED, Commands};

/// Result type for these tests: any failure fails the test.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The repository's top folder, as an agent on this platform names it. Nothing is read from it.
#[cfg(windows)]
const ROOT: &str = r"C:\repo";
/// The repository's top folder, as an agent on this platform names it. Nothing is read from it.
#[cfg(not(windows))]
const ROOT: &str = "/repo";

/// What `line`, run at the top of the repository, keeps, by `commands`.
fn cleaned_by(commands: &Commands, line: &str) -> Option<Cleaned> {
    let root = Path::new(ROOT);
    clean_command(commands, &ShellCommand::cut(line), root, root)
}

/// The secret a fixture's `«name»` stands for. The tokens of a known shape have bodies of few
/// distinct characters, so only their shape gives them away and not their entropy.
fn secret(name: &str) -> Option<String> {
    let value = match name {
        "ghp" => format!("{}{}{}", "gh", "p_", "ab12".repeat(9)),
        "gho" => format!("{}{}{}", "gh", "o_", "cd34".repeat(9)),
        "pat" => format!("{}{}{}", "github", "_pat_", "ab12_cd34_".repeat(6)),
        "sk" => format!("{}{}{}", "s", "k-proj-", "ab12".repeat(8)),
        "skant" => format!("{}{}{}", "s", "k-ant-api03-", "cd34".repeat(8)),
        "xoxb" => format!("{}{}{}", "xo", "xb-1234-5678-", "abcd".repeat(3)),
        "akia" => format!("{}{}{}", "AK", "IA", "ABCD1234".repeat(2)),
        "aiza" => format!("{}{}{}x", "AI", "zaSy", "ab12".repeat(8)),
        // `{"alg":"HS256"}`, `{"sub":"123"}` and `signature`, each in base64url.
        "jwt" => "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.c2lnbmF0dXJl".to_owned(),
        "bearer" => "q1w2e3r4".to_owned(),
        "pw" => "hunter2hunter2".to_owned(),
        "userinfo" => "deploy:hunter2hunter2".to_owned(),
        // Eight characters twice and four once: 3.5219 bits, above the 3.4575 bits of 20.
        "above" => "gh12gh12jk34jk34mn56".to_owned(),
        // Thirty-two distinct characters: 5 bits, above the 4 bits of 32.
        "random" => "Zx7Qp2LmN8vR4tK9sW1yB6dF3hJ0gC5a".to_owned(),
        // Hex alone, 3.9210 bits: above the 3.2 bits of hex.
        "sha" => "24b19f9c3a1e5d7f0b2c4e6a8d0f1b3c5e7a9d0f".to_owned(),
        _ => return None,
    };
    Some(value)
}

/// `template` with each `«name»` filled in, and the secrets filled in.
fn fill(template: &str) -> Result<(String, Vec<String>), String> {
    let mut line = String::new();
    let mut secrets = Vec::new();
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('«') {
        let (name, after) = after.split_once('»').ok_or("a « without its »")?;
        let value = secret(name).ok_or_else(|| format!("no secret named {name}"))?;
        line.push_str(before);
        line.push_str(&value);
        secrets.push(value);
        rest = after;
    }
    line.push_str(rest);
    Ok((line, secrets))
}

/// The command `text` kept.
#[allow(
    clippy::unnecessary_wraps,
    reason = "compared with what `clean_command` returns"
)]
fn command(text: &str) -> Option<Cleaned> {
    Some(Cleaned::Command(text.to_owned()))
}

/// The runner `runner` and the test file `path` kept.
#[allow(
    clippy::unnecessary_wraps,
    reason = "compared with what `clean_command` returns"
)]
fn test_path(runner: &'static str, path: &str) -> Option<Cleaned> {
    Some(Cleaned::TestPath {
        runner,
        path: path.to_owned(),
    })
}

/// The runner `runner` kept alone.
#[allow(
    clippy::unnecessary_wraps,
    reason = "compared with what `clean_command` returns"
)]
fn runner(runner: &'static str) -> Option<Cleaned> {
    Some(Cleaned::Runner(runner))
}

/// Each fixture line and what is kept of it, read off the line by hand.
#[allow(
    clippy::too_many_lines,
    reason = "one row per fixture is clearer than any split"
)]
fn fixtures() -> Vec<(&'static str, Option<Cleaned>)> {
    vec![
        // Tokens of a known shape, as an option's value or after `=`.
        (
            "pytest tests/test_ops.py --ci-ref=«ghp»",
            command("pytest tests/test_ops.py --ci-ref=<redacted>"),
        ),
        (
            "pytest tests/test_ops.py --ci-ref «gho»",
            command("pytest tests/test_ops.py --ci-ref <redacted>"),
        ),
        (
            "cargo test --ci-ref=«pat»",
            command("cargo test --ci-ref=<redacted>"),
        ),
        (
            "pytest tests/test_llm.py --llm «sk»",
            command("pytest tests/test_llm.py --llm <redacted>"),
        ),
        (
            "pytest tests/test_llm.py --llm=«skant»",
            command("pytest tests/test_llm.py --llm=<redacted>"),
        ),
        (
            "pytest tests/test_notify.py --slack «xoxb»",
            command("pytest tests/test_notify.py --slack <redacted>"),
        ),
        (
            "pytest tests/test_s3.py --aws-id «akia»",
            command("pytest tests/test_s3.py --aws-id <redacted>"),
        ),
        (
            "pytest tests/test_maps.py --maps «aiza»",
            command("pytest tests/test_maps.py --maps <redacted>"),
        ),
        // A JSON Web Token whole: entropy alone would keep its payload and signature.
        (
            "pytest tests/test_auth.py --jwt «jwt»",
            command("pytest tests/test_auth.py --jwt <redacted>"),
        ),
        // `sk-` starting a short word is no key.
        (
            "pytest tests/test_ops.py -k sk-learn",
            command("pytest tests/test_ops.py -k sk-learn"),
        ),
        // Bearer: the word alone points at the next; inside a word, its value follows.
        (
            "pytest tests/test_api.py --header Bearer «bearer»",
            command("pytest tests/test_api.py --header Bearer <redacted>"),
        ),
        (
            "pytest tests/test_api.py --auth-header 'Bearer «bearer»'",
            command("pytest tests/test_api.py --auth-header 'Bearer <redacted>'"),
        ),
        // A credential's option, its value as the next word or after `=`.
        (
            "pytest tests/test_ops.py --token «pw»",
            command("pytest tests/test_ops.py --token <redacted>"),
        ),
        (
            "pytest --token=«pw» tests/test_ops.py",
            command("pytest --token=<redacted> tests/test_ops.py"),
        ),
        (
            "pytest tests/test_ops.py --api-key «pw» -q",
            command("pytest tests/test_ops.py --api-key <redacted> -q"),
        ),
        (
            "pytest tests/test_ops.py --api-key=«pw»",
            command("pytest tests/test_ops.py --api-key=<redacted>"),
        ),
        (
            "pytest tests/test_ops.py --password «pw»",
            command("pytest tests/test_ops.py --password <redacted>"),
        ),
        (
            "pytest tests/test_ops.py --password=«pw»",
            command("pytest tests/test_ops.py --password=<redacted>"),
        ),
        (
            "cargo test --secret «pw»",
            command("cargo test --secret <redacted>"),
        ),
        (
            "npm test -- --client-secret=«pw»",
            command("npm test -- --client-secret=<redacted>"),
        ),
        // An option that turns a credential off takes no value.
        (
            "pytest tests/test_ops.py --no-auth -q",
            command("pytest tests/test_ops.py --no-auth -q"),
        ),
        // `-p` is a plugin to pytest and a package to cargo, never a password.
        (
            "pytest -p xdist tests/test_ops.py",
            command("pytest -p xdist tests/test_ops.py"),
        ),
        (
            "cargo test -p cortex-core --lib",
            command("cargo test -p cortex-core --lib"),
        ),
        // A credential's name and value in one argument, by each end of name.
        (
            "make test API_KEY=«pw»",
            command("make test API_KEY=<redacted>"),
        ),
        (
            "make test GITHUB_TOKEN=«pw»",
            command("make test GITHUB_TOKEN=<redacted>"),
        ),
        (
            "make test DB_SECRET=«pw»",
            command("make test DB_SECRET=<redacted>"),
        ),
        (
            "make test ADMIN_PASSWORD=«pw»",
            command("make test ADMIN_PASSWORD=<redacted>"),
        ),
        (
            "make test LDAP_PASSWD=«pw»",
            command("make test LDAP_PASSWD=<redacted>"),
        ),
        ("make test VERBOSE=1", command("make test VERBOSE=1")),
        (
            "mvn test -Ddb.password=«pw»",
            command("mvn test -Ddb.password=<redacted>"),
        ),
        (
            "dotnet test -p:ApiToken=«pw»",
            command("dotnet test -p:ApiToken=<redacted>"),
        ),
        (
            "pytest tests/test_ops.py --header 'X-Api-Key: «pw»'",
            command("pytest tests/test_ops.py --header 'X-Api-Key: <redacted>'"),
        ),
        // A credential named after a URL in the same word: the URL ends at the quote, the comma
        // or the space, and the name after it is read.
        (
            r#"npm test -- --env='{"API_URL":"https://x","API_KEY":"«pw»"}'"#,
            command(r#"npm test -- '--env={"API_URL":"https://x","API_KEY":<redacted>'"#),
        ),
        (
            "pytest tests/test_ops.py -o 'env=BASE=https://h,API_TOKEN=«pw»'",
            command("pytest tests/test_ops.py -o env=BASE=https://h,API_TOKEN=<redacted>"),
        ),
        (
            "pytest tests/test_ops.py --header 'X-Url: https://h X-Api-Key: «pw»'",
            command("pytest tests/test_ops.py --header 'X-Url: https://h X-Api-Key: <redacted>'"),
        ),
        // A credential's pair inside a URL with no `?` before it: in the fragment, after `&`,
        // after `+`; the pair after it kept.
        (
            "pytest tests/test_ops.py '--cb=https://h/cb#access_token=«pw»&state=1'",
            command("pytest tests/test_ops.py '--cb=https://h/cb#access_token=<redacted>&state=1'"),
        ),
        (
            "pytest tests/test_ops.py '--url=https://h&API_TOKEN=«pw»'",
            command("pytest tests/test_ops.py '--url=https://h&API_TOKEN=<redacted>'"),
        ),
        (
            "pytest tests/test_ops.py '--env=URL=https://h+API_KEY=«pw»'",
            command("pytest tests/test_ops.py --env=URL=https://h+API_KEY=<redacted>"),
        ),
        // URLs: user information redacted, the scheme and host kept; credential query values.
        (
            "pytest tests/test_sync.py --remote https://«userinfo»@git.example.com/repo.git",
            command(
                "pytest tests/test_sync.py --remote https://<redacted>@git.example.com/repo.git",
            ),
        ),
        (
            "pytest tests/test_sync.py --remote https://git.example.com/repo.git",
            command("pytest tests/test_sync.py --remote https://git.example.com/repo.git"),
        ),
        (
            "pytest --base-url 'https://api.example.com?access_token=«pw»&page=2'",
            command("pytest --base-url 'https://api.example.com?access_token=<redacted>&page=2'"),
        ),
        // Entropy: just above and just below the bound at 20 characters, a random 32, hex.
        (
            "pytest tests/test_ops.py --seed «above»",
            command("pytest tests/test_ops.py --seed <redacted>"),
        ),
        (
            "pytest tests/test_ops.py --seed gh12jk34mngh12jk34mn",
            command("pytest tests/test_ops.py --seed gh12jk34mngh12jk34mn"),
        ),
        (
            "pytest tests/test_ops.py --run-id «random»",
            command("pytest tests/test_ops.py --run-id <redacted>"),
        ),
        (
            "pytest tests/test_ops.py --commit «sha»",
            command("pytest tests/test_ops.py --commit <redacted>"),
        ),
        (
            "pytest tests/test_ops.py --commit 24b19f9",
            command("pytest tests/test_ops.py --commit 24b19f9"),
        ),
        // Hex of five characters four times each: log2 5 = 2.3219 bits, under 3.2.
        (
            "pytest tests/test_ops.py --commit aaaa1111bbbb2222cccc",
            command("pytest tests/test_ops.py --commit aaaa1111bbbb2222cccc"),
        ),
        // Real test paths and filters: the longest run here, 32 characters, has 3.6258 bits of a
        // bound of 4; the cargo filter's run of 35 holds no digit.
        (
            "pytest tests/api/v2/test_users_endpoint.py -q",
            command("pytest tests/api/v2/test_users_endpoint.py -q"),
        ),
        (
            "cargo test -p cortex-core ranking::tests::warnings_rank_by_kind_then_distance",
            command(
                "cargo test -p cortex-core ranking::tests::warnings_rank_by_kind_then_distance",
            ),
        ),
        (
            "pytest 'tests/test_ops.py::TestX::test_y[param-1]' -q",
            command("pytest 'tests/test_ops.py::TestX::test_y[param-1]' -q"),
        ),
        // The environment, set before the runner in each shell's way, is never kept.
        ("FOO=1 pytest -q", command("pytest -q")),
        (
            "API_TOKEN=«pw» pytest tests/test_ops.py",
            command("pytest tests/test_ops.py"),
        ),
        ("env A=1 B=2 pytest -q", command("pytest -q")),
        ("env -i PATH=/usr/bin pytest -q", command("pytest -q")),
        ("env -u HOME SECRET=«pw» cargo test", command("cargo test")),
        (
            "timeout 600 env SECRET=«pw» pytest -q",
            command("pytest -q"),
        ),
        ("TOKEN=$(cat .token) pytest -q", command("pytest -q")),
        ("export GH_TOKEN=«pw»; pytest -q", command("pytest -q")),
        ("$env:API_KEY='«pw»'; pytest -q", command("pytest -q")),
        (
            "$env:DB_PASSWORD=\"«pw»\"; cargo test -p cortex-core",
            command("cargo test -p cortex-core"),
        ),
        ("set API_TOKEN=«pw» && pytest -q", command("pytest -q")),
        // Harmless arguments kept as written; the programs that only run another left out.
        (
            "pytest tests/test_ops.py -k divide -q",
            command("pytest tests/test_ops.py -k divide -q"),
        ),
        ("go test ./...", command("go test ./...")),
        (
            "go test ./internal/storage/... -run TestPostgres",
            command("go test ./internal/storage/... -run TestPostgres"),
        ),
        (
            "npm test -- --watch=false",
            command("npm test -- --watch=false"),
        ),
        ("python -m pytest", command("python -m pytest")),
        (
            "python -m pytest tests/test_ops.py -x",
            command("python -m pytest tests/test_ops.py -x"),
        ),
        // The interpreter by its name alone, whatever version or folder it was run from.
        (
            "/usr/local/bin/python3.12 -m unittest tests.test_ops -q",
            command("python -m unittest tests.test_ops -q"),
        ),
        // A program by its name alone: a folder can hold a user's name.
        (
            "/home/dev/.venv/bin/pytest tests/test_ops.py",
            command("pytest tests/test_ops.py"),
        ),
        ("./gradlew test", command("gradlew test")),
        ("uv run pytest -q", command("pytest -q")),
        (
            "npx vitest run src/math.test.ts",
            command("vitest run src/math.test.ts"),
        ),
        ("cd sub && pytest -q", command("pytest -q")),
        ("pytest -q | tail -n 20", command("pytest -q")),
        ("make test || cargo test -q", command("cargo test -q")),
        // Uncertain: only the runner and the test file are kept.
        (
            "(cd sub && pytest tests/test_ops.py)",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "{ pytest tests/test_ops.py; }",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "cat ids.txt | pytest tests/test_ops.py",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest tests/test_ops.py -k $FILTER",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest tests/test_ops.py::test_x $EXTRA",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest tests/test_ops.py::test_y[param-1]",
            test_path("pytest", "tests/test_ops.py"),
        ),
        ("pytest $(git diff --name-only)", runner("pytest")),
        ("pytest `cat tests.txt`", runner("pytest")),
        ("pytest tests/test_*.py", runner("pytest")),
        ("cargo test --token", runner("cargo")),
        (
            "pytest tests/test_ops.py --token -q",
            test_path("pytest", "tests/test_ops.py"),
        ),
        ("pytest --password=", runner("pytest")),
        (
            "pytest tests/test_ops.py --header 'X-Api-Key:'",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest tests/test_sync.py --mirror https://ci:«pw»/x@git.example.com",
            test_path("pytest", "tests/test_sync.py"),
        ),
        (
            "pytest tests/test_ops.py -k 'a\tb'",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest tests/test_ops.py -k 'x\u{202E}y'",
            test_path("pytest", "tests/test_ops.py"),
        ),
        ("pytest 'tests/test_\u{202E}ops.py'", runner("pytest")),
        // A path from the machine's top outside the repository, from a home folder or on another
        // machine, which may hold a user's name; a path where it cannot be rewritten; a `file:`
        // URL. On Windows the paths without a drive are not placed, and elsewhere those with one.
        (
            "python -m pytest /home/alice/repo/tests/test_ops.py -x",
            runner("pytest"),
        ),
        (
            "cargo test --manifest-path /Users/alice/x/Cargo.toml",
            runner("cargo"),
        ),
        (
            "pytest --rootdir=/home/alice/repo tests/test_ops.py",
            test_path("pytest", "tests/test_ops.py"),
        ),
        ("make test ROOT=/home/alice/repo", runner("make")),
        (
            "pytest tests/test_ops.py '--basetemp=~/tmp'",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest tests/test_ops.py --basetemp '~alice/tmp'",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            r"pytest tests/test_ops.py --basetemp '\\server\share\alice'",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            r"pytest 'C:\Users\alice\repo\tests\test_ops.py'",
            runner("pytest"),
        ),
        (
            "pytest C:/Users/alice/repo/tests/test_ops.py -x",
            runner("pytest"),
        ),
        (
            "pytest tests/test_ops.py -c/home/alice/pytest.ini",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest tests/test_ops.py --cov=src,/home/alice/lib",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest tests/test_ops.py '--opt=ROOT=/home/alice'",
            test_path("pytest", "tests/test_ops.py"),
        ),
        (
            "pytest '/repo/tests/test_ops.py::test_x[/home/alice]'",
            runner("pytest"),
        ),
        (
            "pytest tests/test_ops.py --remote file:///home/alice/repo.git",
            test_path("pytest", "tests/test_ops.py"),
        ),
        // A drive with a doubled slash is a path, not a URL; a drive with no separator after it
        // is read from a folder the line does not tell.
        (
            "pytest C://Users/bob/proj/tests/test_x.py",
            runner("pytest"),
        ),
        (
            "pytest tests/test_x.py --basetemp=C://Users/bob/tmp",
            test_path("pytest", "tests/test_x.py"),
        ),
        (
            "pytest tests/test_x.py '-k=C://Users/bob'",
            test_path("pytest", "tests/test_x.py"),
        ),
        (r"pytest 'C:..\..\bob\tests\test_x.py'", runner("pytest")),
        (
            r"pytest tests/test_x.py --basetemp 'C:bob\tmp'",
            test_path("pytest", "tests/test_x.py"),
        ),
        // A relative path whose `..` climbs above the repository's top, read from the top; one
        // that stays inside is kept as written.
        ("pytest ../../bob/proj/tests/test_x.py", runner("pytest")),
        (
            "pytest tests/test_x.py --rootdir=../bob",
            test_path("pytest", "tests/test_x.py"),
        ),
        (
            "pytest tests/test_x.py -c../../bob/pytest.ini",
            test_path("pytest", "tests/test_x.py"),
        ),
        (
            "pytest tests/../tests/test_ops.py",
            command("pytest tests/../tests/test_ops.py"),
        ),
        // Two runs of one check and neither the line's exit: which one is not known.
        (
            "pytest tests/test_a.py; pytest tests/test_b.py &",
            runner("pytest"),
        ),
        // Nothing: a line the reader refuses, no runner's check, or two checks and no failure.
        ("pytest 'tests/test_ops.py", None),
        ("./run_checks.sh --token «pw»", None),
        (
            "curl -H 'Authorization: Bearer «pw»' https://api.example.com",
            None,
        ),
        ("echo done", None),
        ("pytest -q && cargo build", None),
        // The cap: `pytest` and nine paths of 20 characters, each after a space, make 6 + 9 * 21
        // = 195 characters, which fit; a tenth makes 216, so the command is cut after the ninth
        // and ends ` ...`: 195 + 4 = 199.
        (
            concat!(
                "pytest tests/test_mod_01.py tests/test_mod_02.py tests/test_mod_03.py",
                " tests/test_mod_04.py tests/test_mod_05.py tests/test_mod_06.py",
                " tests/test_mod_07.py tests/test_mod_08.py tests/test_mod_09.py",
            ),
            command(concat!(
                "pytest tests/test_mod_01.py tests/test_mod_02.py tests/test_mod_03.py",
                " tests/test_mod_04.py tests/test_mod_05.py tests/test_mod_06.py",
                " tests/test_mod_07.py tests/test_mod_08.py tests/test_mod_09.py",
            )),
        ),
        (
            concat!(
                "pytest tests/test_mod_01.py tests/test_mod_02.py tests/test_mod_03.py",
                " tests/test_mod_04.py tests/test_mod_05.py tests/test_mod_06.py",
                " tests/test_mod_07.py tests/test_mod_08.py tests/test_mod_09.py",
                " tests/test_mod_10.py",
            ),
            command(concat!(
                "pytest tests/test_mod_01.py tests/test_mod_02.py tests/test_mod_03.py",
                " tests/test_mod_04.py tests/test_mod_05.py tests/test_mod_06.py",
                " tests/test_mod_07.py tests/test_mod_08.py tests/test_mod_09.py ...",
            )),
        ),
    ]
}

/// Each fixture line keeps the form typed beside it, and none of its secrets.
#[test]
fn each_fixture_keeps_its_hand_written_form_and_no_secret() -> TestResult {
    let commands = Commands::default();
    let rows = fixtures();
    assert!(rows.len() >= 45, "{} rows", rows.len());
    for (template, expected) in rows {
        let (line, secrets) = fill(template)?;
        let cleaned = cleaned_by(&commands, &line);
        assert_eq!(cleaned, expected, "{template}");
        let stored = cleaned
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        assert!(stored.chars().count() <= 200, "{template}");
        for secret in &secrets {
            assert!(!stored.contains(secret.as_str()), "{template}");
        }
    }
    Ok(())
}

/// Lines naming paths from the machine's top inside the repository, whose top is [`ROOT`], and
/// what is kept of them run at the top: the way to each path from there, read off by hand.
fn inside_fixtures() -> Vec<(String, Option<Cleaned>)> {
    let mut rows = vec![
        (
            format!("python -m pytest '{ROOT}/tests/test_ops.py' -x"),
            command("python -m pytest tests/test_ops.py -x"),
        ),
        (
            format!("cargo test --manifest-path '{ROOT}/crates/core/Cargo.toml'"),
            command("cargo test --manifest-path crates/core/Cargo.toml"),
        ),
        (
            format!("pytest '--rootdir={ROOT}' tests/test_ops.py"),
            command("pytest --rootdir=. tests/test_ops.py"),
        ),
        (
            format!("make test 'TOP={ROOT}/calc'"),
            command("make test TOP=calc"),
        ),
        (
            format!("pytest tests/test_ops.py '-c{ROOT}/pytest.ini'"),
            command("pytest tests/test_ops.py -cpytest.ini"),
        ),
        (
            format!("pytest '{ROOT}/tests/test_ops.py::TestX::test_y' -q"),
            command("pytest tests/test_ops.py::TestX::test_y -q"),
        ),
        (
            format!("pytest '{ROOT}/tests/../tests/test_ops.py'"),
            command("pytest tests/test_ops.py"),
        ),
        // Out of the repository by `..`, or into a folder whose name only starts as the top's.
        (
            format!("pytest '{ROOT}/../alice/tests/test_ops.py'"),
            runner("pytest"),
        ),
        (
            format!("pytest '{ROOT}2/tests/test_ops.py'"),
            runner("pytest"),
        ),
    ];
    // Windows: either separator, and the top in any letters' case; the names after it as written.
    #[cfg(windows)]
    rows.extend([
        (
            r"pytest 'C:\repo\tests\test_ops.py'".to_owned(),
            command("pytest tests/test_ops.py"),
        ),
        (
            "pytest C:/repo/tests/test_ops.py".to_owned(),
            command("pytest tests/test_ops.py"),
        ),
        (
            r"pytest 'c:\REPO\Tests\test_ops.py'".to_owned(),
            command("pytest Tests/test_ops.py"),
        ),
        (
            "pytest c:/Repo/tests/test_ops.py".to_owned(),
            command("pytest tests/test_ops.py"),
        ),
        (
            "pytest C://repo//tests/test_ops.py".to_owned(),
            command("pytest tests/test_ops.py"),
        ),
        // Read as a URL, the drive's text would hide the path in its test id.
        (
            "pytest 'C://repo/tests/test_ops.py::test_y[/home/alice]'".to_owned(),
            runner("pytest"),
        ),
        (
            r"pytest '--rootdir=C:\REPO\tests'".to_owned(),
            command("pytest --rootdir=tests"),
        ),
        (
            "pytest /repo/tests/test_ops.py".to_owned(),
            runner("pytest"),
        ),
    ]);
    // Elsewhere a path's case is part of its name, and a drive places nothing.
    #[cfg(not(windows))]
    rows.extend([
        (
            "pytest /repo/tests/test_ops.py".to_owned(),
            command("pytest tests/test_ops.py"),
        ),
        (
            "pytest /REPO/tests/test_ops.py".to_owned(),
            runner("pytest"),
        ),
        (
            "pytest C://repo/tests/test_ops.py".to_owned(),
            runner("pytest"),
        ),
        (
            r"pytest 'C:\repo\tests\test_ops.py'".to_owned(),
            runner("pytest"),
        ),
    ]);
    rows
}

/// A path from the machine's top inside the repository is kept as the way to it from the top.
#[test]
fn a_path_inside_the_repository_is_kept_from_where_the_command_ran() {
    let commands = Commands::default();
    for (line, expected) in inside_fixtures() {
        assert_eq!(cleaned_by(&commands, &line), expected, "{line}");
    }
}

/// The way to a path is written from the folder the command ran in: the agent's working folder,
/// moved by each `cd` before the command. A folder the line does not tell places nothing.
#[test]
fn a_path_is_written_from_the_folder_the_command_ran_in() {
    let commands = Commands::default();
    let root = Path::new(ROOT);
    let calc = root.join("calc");
    let kept =
        |line: String, cwd: &Path| clean_command(&commands, &ShellCommand::cut(&line), root, cwd);
    assert_eq!(
        kept(format!("pytest '{ROOT}/calc/tests/test_ops.py'"), &calc),
        command("pytest tests/test_ops.py")
    );
    assert_eq!(
        kept(format!("pytest '{ROOT}/tests/test_ops.py'"), &calc),
        command("pytest ../tests/test_ops.py")
    );
    assert_eq!(
        kept(format!("pytest '{ROOT}/calc'"), &calc),
        command("pytest .")
    );
    assert_eq!(
        kept(
            format!("cd calc && pytest '{ROOT}/calc/tests/test_ops.py'"),
            root
        ),
        command("pytest tests/test_ops.py")
    );
    assert_eq!(
        kept(
            format!("cd $DIR && pytest '{ROOT}/calc/tests/test_ops.py'"),
            root
        ),
        runner("pytest")
    );
    // A relative `..` stays inside from `calc` and is kept; one more climbs out; from a folder
    // the line does not tell, any `..` might.
    assert_eq!(
        kept("pytest ../tests/test_ops.py".to_owned(), &calc),
        command("pytest ../tests/test_ops.py")
    );
    assert_eq!(
        kept("pytest ../../bob/tests/test_ops.py".to_owned(), &calc),
        runner("pytest")
    );
    assert_eq!(
        kept("cd $DIR && pytest ../tests/test_ops.py".to_owned(), root),
        runner("pytest")
    );
}

/// A check of the user's patterns keeps its runner's name and test file, never its command: the
/// pattern does not say which words are the check's arguments.
#[test]
fn a_check_of_the_user_s_patterns_keeps_its_test_file_alone() -> TestResult {
    let commands = Commands::new(&[r"^\./run_tests\.sh".to_owned()])?;
    let (line, _) = fill("./run_tests.sh tests/test_ops.py --token «pw»")?;
    assert_eq!(
        cleaned_by(&commands, &line),
        Some(Cleaned::TestPath {
            runner: CONFIGURED,
            path: "tests/test_ops.py".to_owned(),
        })
    );
    Ok(())
}

/// Each form is stored as its documentation says.
#[test]
fn each_form_is_stored_as_written() {
    let kept = Cleaned::Command("pytest --token <redacted>".to_owned());
    assert_eq!(kept.to_string(), "pytest --token <redacted>");
    let path = Cleaned::TestPath {
        runner: "pytest",
        path: "tests/test_ops.py".to_owned(),
    };
    assert_eq!(path.to_string(), "pytest tests/test_ops.py");
    assert_eq!(Cleaned::Runner("cargo").to_string(), "cargo");
}

/// Realistic commands of the runners: the words that lead (the program and its subcommand), the
/// arguments a secret may be put among, and what is kept of the command as it stands.
const BASES: &[(&str, &[&str], &str)] = &[
    (
        "pytest",
        &["tests/test_ops.py", "-k", "divide", "-q"],
        "pytest tests/test_ops.py -k divide -q",
    ),
    (
        "cargo test",
        &["-p", "cortex-core", "--lib"],
        "cargo test -p cortex-core --lib",
    ),
    (
        "go test",
        &["./...", "-run", "TestDivide"],
        "go test ./... -run TestDivide",
    ),
    (
        "npm test",
        &["--", "--watch=false"],
        "npm test -- --watch=false",
    ),
    (
        "python -m pytest",
        &["-x", "tests"],
        "python -m pytest -x tests",
    ),
    (
        "npx vitest run",
        &["src/math.test.ts"],
        "vitest run src/math.test.ts",
    ),
    ("make test", &["VERBOSE=1"], "make test VERBOSE=1"),
    (
        "mvn test",
        &["-Dtest=DivideTest"],
        "mvn test -Dtest=DivideTest",
    ),
];

/// The options whose value is a credential.
const CREDENTIAL_OPTIONS: &[&str] = &[
    "--token",
    "--api-key",
    "--password",
    "--secret",
    "--auth-token",
    "--client-secret",
    "--access-key",
];

/// The ends of credentials' variable names.
const KEY_ENDS: &[&str] = &["_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "PASSWD"];

/// Paths from the machine's top outside the repository, `{user}` standing for a user's name.
const OUTSIDE_PATHS: &[&str] = &[
    "/home/{user}/repo/tests/test_ops.py",
    "--rootdir=/Users/{user}/repo",
    "C:/Users/{user}/repo/tests",
    "-c/home/{user}/pytest.ini",
    "../../{user}/proj/tests/test_x.py",
    "--rootdir=../../{user}",
    "C://Users/{user}/repo/tests",
    "--basetemp=C://Users/{user}/tmp",
    "C:../../{user}/tests",
];

/// A credential's value: long enough not to be found in a command by chance.
fn credential_value() -> impl Strategy<Value = String> {
    "[A-Za-z0-9]{12,30}"
}

/// A token of a known shape with a random body. The prefixes are assembled from pieces, as in
/// [`secret`].
fn shaped_token() -> impl Strategy<Value = String> {
    prop_oneof![
        "[A-Za-z0-9]{20,40}".prop_map(|body| format!("{}{}{body}", "gh", "p_")),
        "[A-Za-z0-9]{20,40}".prop_map(|body| format!("{}{}{body}", "gh", "s_")),
        "[A-Za-z0-9_]{20,60}".prop_map(|body| format!("{}{}{body}", "github", "_pat_")),
        "[A-Za-z0-9_-]{20,60}[0-9]".prop_map(|body| format!("{}{}{body}", "s", "k-")),
        "[A-Za-z0-9_-]{20,60}[0-9]".prop_map(|body| format!("{}{}{body}", "s", "k-ant-")),
        "[A-Za-z0-9-]{10,40}".prop_map(|body| format!("{}{}{body}", "xo", "xb-")),
        "[A-Z0-9]{16}".prop_map(|body| format!("{}{}{body}", "AK", "IA")),
        "[A-Za-z0-9_-]{30,40}".prop_map(|body| format!("{}{}{body}", "AI", "za")),
        "[A-Za-z0-9_-]{20,30}".prop_map(|body| format!("{}{}{body}", "gl", "pat-")),
    ]
}

/// A JSON Web Token with a random header, payload and signature.
fn web_token() -> impl Strategy<Value = String> {
    let part = "[A-Za-z0-9_-]{8,30}";
    (part, part, part)
        .prop_map(|(header, payload, signature)| format!("eyJ{header}.eyJ{payload}.{signature}"))
}

/// A random string of no known shape: 20 to 36 distinct letters and digits, both present, so its
/// entropy is the most its length allows.
fn random_token() -> impl Strategy<Value = String> {
    let letters: Vec<char> = ('a'..='z').chain('A'..='Z').collect();
    let digits: Vec<char> = ('0'..='9').collect();
    (
        prop::sample::subsequence(letters, 18..=30),
        prop::sample::subsequence(digits, 2..=6),
    )
        .prop_map(|(mut chars, digits)| {
            chars.extend(digits);
            chars
        })
        .prop_shuffle()
        .prop_map(|chars| chars.into_iter().collect::<String>())
}

/// A 40-character hex id: every hex digit twice and eight of them a third time, which gives 3.97
/// bits, above the hex bound of 3.2.
fn commit_id() -> impl Strategy<Value = String> {
    let hex: Vec<char> = "0123456789abcdef".chars().collect();
    let all = hex.clone();
    prop::sample::subsequence(hex, 8)
        .prop_map(move |extra| {
            let mut chars = all.clone();
            chars.extend(all.iter());
            chars.extend(extra);
            chars
        })
        .prop_shuffle()
        .prop_map(|chars| chars.into_iter().collect::<String>())
}

/// The words that carry a secret into a command, in each form a detector reads, and the secrets.
fn carrier() -> impl Strategy<Value = (Vec<String>, Vec<String>)> {
    prop_oneof![
        shaped_token().prop_map(|token| (vec![token.clone()], vec![token])),
        shaped_token().prop_map(|token| (vec![format!("--ci-ref={token}")], vec![token])),
        (prop::sample::select(CREDENTIAL_OPTIONS), credential_value())
            .prop_map(|(option, value)| (vec![option.to_owned(), value.clone()], vec![value])),
        (prop::sample::select(CREDENTIAL_OPTIONS), credential_value())
            .prop_map(|(option, value)| (vec![format!("{option}={value}")], vec![value])),
        (
            "[A-Z]{1,8}",
            prop::sample::select(KEY_ENDS),
            credential_value()
        )
            .prop_map(|(head, end, value)| (vec![format!("{head}{end}={value}")], vec![value])),
        (credential_value(), credential_value()).prop_map(|(user, password)| {
            let url = format!("https://{user}:{password}@git.example.com/acme/repo.git");
            (vec![url], vec![format!("{user}:{password}"), password])
        }),
        credential_value()
            .prop_map(|value| (vec!["Bearer".to_owned(), value.clone()], vec![value])),
        web_token().prop_map(|token| (vec![token.clone()], vec![token])),
        random_token().prop_map(|token| (vec![token.clone()], vec![token])),
        commit_id().prop_map(|id| (vec![id.clone()], vec![id])),
        (prop::sample::select(OUTSIDE_PATHS), "[0-9]{6}").prop_map(|(path, digits)| {
            let user = format!("user{digits}");
            (vec![path.replace("{user}", &user)], vec![user])
        }),
    ]
}

/// A secret of any shape, to set in the environment.
fn any_secret() -> impl Strategy<Value = String> {
    prop_oneof![
        credential_value(),
        shaped_token(),
        web_token(),
        random_token()
    ]
}

/// The command `lead` with `args`, `carried` put among them before the argument at `at`.
fn line_with(lead: &str, args: &[&str], at: usize, carried: &[String]) -> String {
    let (before, after) = args.split_at(at.min(args.len()));
    let words: Vec<String> = before
        .iter()
        .copied()
        .map(String::from)
        .chain(carried.iter().cloned())
        .chain(after.iter().copied().map(String::from))
        .collect();
    format!("{lead} {}", words.join(" "))
}

proptest! {
    /// However a secret of any detector's shape, or a path outside the repository with a user's
    /// name in it, is put among a runner's arguments, the stored form never holds the secret or
    /// the name, a command kept holds [`REDACTED`] in its place, and nothing kept is longer than
    /// the cap.
    #[test]
    fn no_secret_among_the_arguments_is_stored(
        base in prop::sample::select(BASES),
        at in any::<prop::sample::Index>(),
        (carried, secrets) in carrier(),
    ) {
        let (lead, args, _) = base;
        let line = line_with(lead, args, at.index(args.len() + 1), &carried);
        let cleaned = cleaned_by(&Commands::default(), &line);
        prop_assert!(cleaned.is_some(), "{line}");
        let is_command = matches!(cleaned, Some(Cleaned::Command(_)));
        let stored = cleaned.as_ref().map(ToString::to_string).unwrap_or_default();
        for secret in &secrets {
            prop_assert!(!stored.contains(secret.as_str()));
        }
        prop_assert!(stored.chars().count() <= 200);
        if is_command && !stored.ends_with(" ...") {
            prop_assert!(stored.contains(REDACTED));
        }
    }

    /// However the environment is set before a runner's command, in bash, zsh, `PowerShell` or
    /// the Windows command prompt, the runner's command alone is kept and the value never is.
    #[test]
    fn no_secret_in_the_environment_is_stored(
        base in prop::sample::select(BASES),
        form in 0..5_usize,
        name in "[A-Z][A-Z_]{0,10}",
        value in any_secret(),
    ) {
        let (lead, args, kept) = base;
        let command = format!("{lead} {}", args.join(" "));
        let line = match form {
            0 => format!("{name}={value} {command}"),
            1 => format!("env {name}={value} {command}"),
            2 => format!("$env:{name}='{value}'; {command}"),
            3 => format!("export {name}={value}; {command}"),
            _ => format!("set {name}={value} && {command}"),
        };
        let cleaned = cleaned_by(&Commands::default(), &line);
        prop_assert_eq!(cleaned, Some(Cleaned::Command(kept.to_owned())));
    }
}
