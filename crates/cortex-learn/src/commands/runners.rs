//! The runners of checks Cortex knows, one row each: the programs that run it and what their
//! arguments say about the check. The rows are the common runners of tests, type checks and
//! linters, and the build command of each ecosystem whose tests they run. A program asked only for
//! its help or version (`--help`, `-h`, `--version`) runs no check.

use super::argv::{Invocation, program_name};
use super::{Check, CommandKind, tail};

/// One runner: its name as kept with the event, the programs that run it, and the check their
/// arguments ask for.
struct Runner {
    /// The runner's name.
    name: &'static str,
    /// The programs, by file name in lower case without an extension.
    programs: &'static [&'static str],
    /// What the arguments ask for.
    rule: Rule,
}

/// How a runner's arguments decide the check.
enum Rule {
    /// The program runs this check whatever its arguments.
    Always(CommandKind),
    /// The arguments decide, or ask for no check.
    Arguments(fn(&[&str]) -> Option<CommandKind>),
    /// A package manager: a script, by its name, or another runner it runs.
    Scripts,
}

/// The runners. A program appears in one row only.
const RUNNERS: &[Runner] = &[
    Runner {
        name: "npm",
        programs: &["npm"],
        rule: Rule::Scripts,
    },
    Runner {
        name: "pnpm",
        programs: &["pnpm"],
        rule: Rule::Scripts,
    },
    Runner {
        name: "yarn",
        programs: &["yarn"],
        rule: Rule::Scripts,
    },
    Runner {
        name: "bun",
        programs: &["bun"],
        rule: Rule::Scripts,
    },
    Runner {
        name: "vitest",
        programs: &["vitest"],
        rule: Rule::Always(CommandKind::Test),
    },
    Runner {
        name: "jest",
        programs: &["jest"],
        rule: Rule::Always(CommandKind::Test),
    },
    Runner {
        name: "pytest",
        programs: &["pytest", "py.test"],
        rule: Rule::Always(CommandKind::Test),
    },
    Runner {
        name: "unittest",
        programs: &["unittest"],
        rule: Rule::Always(CommandKind::Test),
    },
    Runner {
        name: "go",
        programs: &["go"],
        rule: Rule::Arguments(go),
    },
    Runner {
        name: "cargo",
        programs: &["cargo"],
        rule: Rule::Arguments(cargo),
    },
    Runner {
        name: "mvn",
        programs: &["mvn", "mvnw"],
        rule: Rule::Arguments(maven),
    },
    Runner {
        name: "gradle",
        programs: &["gradle", "gradlew"],
        rule: Rule::Arguments(gradle),
    },
    Runner {
        name: "rspec",
        programs: &["rspec"],
        rule: Rule::Always(CommandKind::Test),
    },
    Runner {
        name: "phpunit",
        programs: &["phpunit"],
        rule: Rule::Always(CommandKind::Test),
    },
    Runner {
        name: "dotnet",
        programs: &["dotnet"],
        rule: Rule::Arguments(dotnet),
    },
    Runner {
        name: "make",
        programs: &["make", "gmake"],
        rule: Rule::Arguments(make),
    },
    Runner {
        name: "tsc",
        programs: &["tsc", "vue-tsc"],
        rule: Rule::Arguments(tsc),
    },
    Runner {
        name: "eslint",
        programs: &["eslint"],
        rule: Rule::Always(CommandKind::Lint),
    },
    Runner {
        name: "ruff",
        programs: &["ruff"],
        rule: Rule::Always(CommandKind::Lint),
    },
    Runner {
        name: "mypy",
        programs: &["mypy", "dmypy"],
        rule: Rule::Always(CommandKind::TypeCheck),
    },
];

/// The options of the package managers that take a value as the next word.
const SCRIPTS_VALUE_OPTIONS: &[&str] = &[
    "--prefix",
    "-C",
    "--dir",
    "--cwd",
    "--filter",
    "-F",
    "--workspace",
    "-w",
];

/// The arguments that ask a program for its help or version rather than a check.
const NOT_A_CHECK: &[&str] = &["--help", "-h", "--version"];

/// The check `invocation` runs, if it is one of a runner the table knows.
pub(super) fn check(invocation: &Invocation<'_>) -> Option<Check> {
    check_of(&invocation.program, &invocation.arg_texts())
}

/// The check the program `program` runs with `args`, if a runner the table knows runs one.
fn check_of(program: &str, args: &[&str]) -> Option<Check> {
    if args.iter().any(|arg| NOT_A_CHECK.contains(arg)) {
        return None;
    }
    let runner = RUNNERS
        .iter()
        .find(|runner| runner.programs.contains(&program))?;
    let found = |kind| Check {
        runner: runner.name,
        kind,
    };
    match runner.rule {
        Rule::Always(kind) => Some(found(kind)),
        Rule::Arguments(decide) => decide(args).map(found),
        Rule::Scripts => scripts(runner.name, args),
    }
}

/// The check a package manager runs with `args`: its test command, a script by name (`run` before
/// it, or the name alone for any but npm), or a runner it runs (`exec`, `dlx` or `x` before it, or
/// the program's name alone for any but npm).
fn scripts(name: &'static str, args: &[&str]) -> Option<Check> {
    let args = after_options(args, SCRIPTS_VALUE_OPTIONS);
    let found = |kind| Check { runner: name, kind };
    match *args.first()? {
        "test" | "t" | "tst" => Some(found(CommandKind::Test)),
        "run" | "run-script" => script_kind(first_operand(tail(args, 1), &[])?).map(found),
        "exec" | "dlx" | "x" => {
            let rest = after_options(tail(args, 1), &[]);
            check_of(&program_name(rest.first()?), tail(rest, 1))
        }
        _ if name == "npm" => None,
        word => script_kind(word)
            .map(found)
            .or_else(|| check_of(&program_name(word), tail(args, 1))),
    }
}

/// The check a package script runs, by its name: `test`, `build`, `typecheck` or `type-check`,
/// `lint`, each also with a `:` and more after it (`test:unit`).
fn script_kind(script: &str) -> Option<CommandKind> {
    let family = script.split(':').next().unwrap_or(script);
    match family {
        "test" | "tests" => Some(CommandKind::Test),
        "build" => Some(CommandKind::Build),
        "typecheck" | "type-check" => Some(CommandKind::TypeCheck),
        "lint" => Some(CommandKind::Lint),
        _ => None,
    }
}

/// `go`: `test` runs tests, `build` builds.
fn go(args: &[&str]) -> Option<CommandKind> {
    match first_operand(args, &["-C"])? {
        "test" => Some(CommandKind::Test),
        "build" => Some(CommandKind::Build),
        _ => None,
    }
}

/// `cargo`, a `+toolchain` before its command allowed: `test` or `t` and `nextest run` run tests,
/// `check` or `c` type-checks, `clippy` lints, `build` or `b` builds.
fn cargo(args: &[&str]) -> Option<CommandKind> {
    let args: Vec<&str> = args
        .iter()
        .copied()
        .skip_while(|arg| arg.starts_with('+'))
        .collect();
    let options = ["--color", "--config", "-Z", "-C"];
    let command = first_operand(&args, &options)?;
    match command {
        "test" | "t" => Some(CommandKind::Test),
        "nextest" => {
            let after = args.iter().position(|arg| *arg == "nextest")? + 1;
            matches!(first_operand(tail(&args, after), &[])?, "run" | "r")
                .then_some(CommandKind::Test)
        }
        "check" | "c" => Some(CommandKind::TypeCheck),
        "clippy" => Some(CommandKind::Lint),
        "build" | "b" => Some(CommandKind::Build),
        _ => None,
    }
}

/// Maven: a phase that runs the tests (`test` and every phase after it, or a `:test` goal), or one
/// that only compiles.
fn maven(args: &[&str]) -> Option<CommandKind> {
    let options = [
        "-f",
        "--file",
        "-pl",
        "--projects",
        "-P",
        "--activate-profiles",
        "-D",
        "--define",
        "-s",
        "--settings",
        "-gs",
        "--global-settings",
        "-rf",
        "--resume-from",
        "-T",
        "--threads",
        "-l",
        "--log-file",
        "-b",
        "--builder",
    ];
    let goals = operands(args, &options);
    if goals.iter().any(|goal| {
        matches!(
            *goal,
            "test" | "integration-test" | "verify" | "package" | "install" | "deploy"
        ) || goal.ends_with(":test")
    }) {
        Some(CommandKind::Test)
    } else if goals
        .iter()
        .any(|goal| matches!(*goal, "compile" | "test-compile"))
    {
        Some(CommandKind::Build)
    } else {
        None
    }
}

/// Gradle: a task, by the last part of its path, that runs tests (`test`, `check`, `build`, or a
/// name starting with `test`), or one that only compiles or assembles.
fn gradle(args: &[&str]) -> Option<CommandKind> {
    let options = [
        "-p",
        "--project-dir",
        "-b",
        "--build-file",
        "-c",
        "--settings-file",
        "-x",
        "--exclude-task",
        "-D",
        "-P",
        "--max-workers",
        "-I",
        "--init-script",
        "-g",
        "--gradle-user-home",
    ];
    let tasks: Vec<&str> = operands(args, &options)
        .into_iter()
        .map(|task| task.rsplit(':').next().unwrap_or(task))
        .collect();
    if tasks
        .iter()
        .any(|task| matches!(*task, "check" | "build") || task.starts_with("test"))
    {
        Some(CommandKind::Test)
    } else if tasks
        .iter()
        .any(|task| *task == "assemble" || *task == "classes" || task.starts_with("compile"))
    {
        Some(CommandKind::Build)
    } else {
        None
    }
}

/// `dotnet`: `test` runs tests, `build` builds.
fn dotnet(args: &[&str]) -> Option<CommandKind> {
    match first_operand(args, &[])? {
        "test" => Some(CommandKind::Test),
        "build" => Some(CommandKind::Build),
        _ => None,
    }
}

/// `make`: the targets `test`, `tests` or `check` run tests; no target, `all` or `build` builds.
/// Variables set on the command line are not targets.
fn make(args: &[&str]) -> Option<CommandKind> {
    let options = [
        "-C",
        "--directory",
        "-f",
        "--file",
        "--makefile",
        "-o",
        "--old-file",
        "-W",
        "--what-if",
        "-I",
        "--include-dir",
    ];
    let targets: Vec<&str> = operands(args, &options)
        .into_iter()
        .filter(|target| !target.contains('=') && !target.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    if targets
        .iter()
        .any(|target| matches!(*target, "test" | "tests" | "check"))
    {
        Some(CommandKind::Test)
    } else if targets.is_empty()
        || targets
            .iter()
            .any(|target| matches!(*target, "all" | "build"))
    {
        Some(CommandKind::Build)
    } else {
        None
    }
}

/// `tsc`: with `--noEmit` it type-checks; otherwise it builds, `-b` or not.
fn tsc(args: &[&str]) -> Option<CommandKind> {
    if args.iter().any(|arg| arg.eq_ignore_ascii_case("--noemit")) {
        Some(CommandKind::TypeCheck)
    } else if args.contains(&"--init") {
        None
    } else {
        Some(CommandKind::Build)
    }
}

/// `args` past the options at their start, the value of each option in `with_value` with it.
fn after_options<'a, 'b>(args: &'a [&'b str], with_value: &[&str]) -> &'a [&'b str] {
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        if *arg == "--" {
            return tail(args, at + 1);
        }
        if !arg.starts_with('-') {
            break;
        }
        at += if with_value.contains(arg) { 2 } else { 1 };
    }
    tail(args, at)
}

/// The first argument that is not an option or the value of one in `with_value`.
fn first_operand<'a>(args: &[&'a str], with_value: &[&str]) -> Option<&'a str> {
    operands(args, with_value).into_iter().next()
}

/// Every argument that is not an option or the value of one in `with_value`, in order.
fn operands<'a>(args: &[&'a str], with_value: &[&str]) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(&arg) = args.get(at) {
        if arg == "--" {
            break;
        }
        if arg.starts_with('-') {
            at += if with_value.contains(&arg) { 2 } else { 1 };
            continue;
        }
        found.push(arg);
        at += 1;
    }
    found
}

#[cfg(test)]
mod tests {
    use cortex_core::Identifier;

    use super::*;

    /// Every runner's name, and the name of a command the user's patterns name, is one an event's
    /// facts may keep.
    #[test]
    fn every_runner_s_name_is_an_identifier() {
        let names = RUNNERS
            .iter()
            .map(|runner| runner.name)
            .chain([super::super::CONFIGURED]);
        for name in names {
            assert!(Identifier::new(name).is_ok(), "{name}");
        }
    }
}
