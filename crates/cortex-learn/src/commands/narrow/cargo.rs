//! cargo: which arguments select tests, and how a narrow command names integration test files.
//! Anything that restricts the targets (`--lib`, `--test`, `--doc`), the packages (`--exclude`,
//! `--manifest-path`) or the tests (a name filter, libtest's `--ignored`, `--skip`) selects, and a
//! flag that shapes the build (`--features`, `--profile`, `--target`) makes the narrow command
//! differ from the run, so each leaves the run uncertain. Only a list of flags known to leave the
//! tests alone keeps it whole, and exactly one `-p crate` is the whole of that package.
//!
//! A test file maps to a command only when it is `<package folder>/tests/<name>.rs`, which is the
//! integration test target `<name>`: unit tests live inside source files and cannot be named by
//! file. A package other than the folder's own is reached with `--manifest-path`, or with `-p`
//! when the whole run named a package. The run's flags are repeated, except those that widen it to
//! every package or target (`--workspace`, `--all`, `--all-targets`), and so are the test binary's
//! arguments after `--`.

use super::{Narrowed, SuiteRun, Whole};

/// The flags that leave the tests alone and have no value.
const PLAIN_FLAGS: &[&str] = &[
    "--workspace",
    "--all",
    "--all-targets",
    "--locked",
    "--frozen",
    "--offline",
    "--no-fail-fast",
    "--keep-going",
    "-q",
    "--quiet",
    "-v",
    "-vv",
    "--verbose",
];

/// The flags that leave the tests alone and take a value, `=` or the next word.
const VALUE_FLAGS: &[&str] = &["-j", "--jobs", "--color", "--message-format"];

/// The test binary's flags that leave the tests alone and take no value.
const LIBTEST_PLAIN: &[&str] = &[
    "--nocapture",
    "--no-capture",
    "--show-output",
    "--include-ignored",
    "-q",
    "--quiet",
];

/// The test binary's flag that sets the threads, `=` or the next word.
const LIBTEST_THREADS: &str = "--test-threads";

/// The flags that widen a run to every package or target, which a narrow command does not repeat.
const WIDENING: &[&str] = &["--workspace", "--all", "--all-targets"];

/// Whether the arguments after `cargo` are a test run that selects no tests.
pub(super) fn whole(args: &[&str]) -> Option<Whole> {
    let toolchains = args.iter().take_while(|arg| arg.starts_with('+')).count();
    let (toolchain, rest) = args.split_at(toolchains);
    let (&command, flags) = rest.split_first()?;
    if !matches!(command, "test" | "t") {
        return None;
    }
    let mut found = Whole::default();
    let mut at = 0;
    while let Some(&arg) = flags.get(at) {
        at += 1;
        if arg == "--" {
            let test_args = flags.get(at..)?;
            if !libtest_is_whole(test_args) {
                return None;
            }
            found.test_args = ["--"]
                .iter()
                .chain(test_args)
                .map(|arg| (*arg).to_owned())
                .collect();
            break;
        }
        match arg {
            "--release" | "-r" => found.is_release = true,
            "-p" | "--package" => {
                let name = *flags.get(at)?;
                at += 1;
                set_package(&mut found, name)?;
            }
            _ if arg.starts_with("--package=") => {
                set_package(&mut found, arg.strip_prefix("--package=")?)?;
            }
            _ if WIDENING.contains(&arg) => {}
            _ if PLAIN_FLAGS.contains(&arg) || is_joined_value(arg) => {
                found.options.push(arg.to_owned());
            }
            _ if VALUE_FLAGS.contains(&arg) => {
                found.options.push(arg.to_owned());
                found.options.push((*flags.get(at)?).to_owned());
                at += 1;
            }
            _ => return None,
        }
    }
    Some(tail_of(found, toolchain, command))
}

/// `found` with the toolchain words and the command as the tail the narrow command repeats.
fn tail_of(mut found: Whole, toolchain: &[&str], command: &str) -> Whole {
    found.tail = toolchain
        .iter()
        .copied()
        .chain([command])
        .map(str::to_owned)
        .collect();
    found
}

/// Whether `arg` is a value flag written `--flag=value`.
fn is_joined_value(arg: &str) -> bool {
    arg.split_once('=')
        .is_some_and(|(name, _)| VALUE_FLAGS.contains(&name))
}

/// Records `name` as the one package; `None` for a second package or a pattern.
fn set_package(found: &mut Whole, name: &str) -> Option<()> {
    if found.package.is_some() || !is_crate_name(name) {
        return None;
    }
    found.package = Some(name.to_owned());
    Some(())
}

/// Whether `name` is a plain crate or test target name: letters, digits, `_` and `-`.
fn is_crate_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether the arguments after `--`, which go to the test binary, select no tests.
fn libtest_is_whole(args: &[&str]) -> bool {
    let mut at = 0;
    while let Some(&arg) = args.get(at) {
        at += 1;
        if LIBTEST_PLAIN.contains(&arg) {
            continue;
        }
        if arg == LIBTEST_THREADS {
            at += 1;
        } else if !arg.starts_with("--test-threads=") {
            return false;
        }
    }
    true
}

/// The package folder and the target name of the test file `file`, when it is
/// `<folder>/tests/<name>.rs`; the folder is empty for the folder's own package.
fn test_target(file: &str) -> Option<(&str, &str)> {
    let stem = file.strip_suffix(".rs")?;
    let names: Vec<&str> = stem.split('/').collect();
    if names.contains(&"src") {
        return None;
    }
    let (name, folders) = names.split_last()?;
    let (&folder_name, outer) = folders.split_last()?;
    if folder_name != "tests" || !is_crate_name(name) {
        return None;
    }
    let joined_len: usize = outer.iter().map(|part| part.len() + 1).sum();
    Some((file.get(..joined_len.saturating_sub(1))?, name))
}

/// Whether a package at `folder` (below the run's folder) is the crate `name`, `-` and `_` alike.
fn is_package(run: &SuiteRun, folder: &str, name: &str) -> bool {
    let own = if folder.is_empty() {
        run.dir.file_name()
    } else {
        folder.rsplit('/').next().unwrap_or_default()
    };
    !own.is_empty() && own.replace('-', "_") == name.replace('-', "_")
}

/// The arguments that run the integration tests among `files`: the package, then a `--test` for
/// each file of the first package that maps. Files of other packages are left out, as one command
/// names one package.
pub(super) fn narrow(run: &SuiteRun, files: &[&str]) -> Option<Narrowed> {
    let mut package: Option<&str> = None;
    let mut names: Vec<&str> = Vec::new();
    let mut runs: Vec<usize> = Vec::new();
    for (place, file) in files.iter().enumerate() {
        let Some((folder, name)) = test_target(file) else {
            continue;
        };
        let is_named = run
            .package
            .as_deref()
            .is_none_or(|crate_name| is_package(run, folder, crate_name));
        if !is_named || package.is_some_and(|chosen| chosen != folder) {
            continue;
        }
        package = Some(folder);
        runs.push(place);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let folder = package?;
    let mut args: Vec<String> = Vec::new();
    if run.is_release {
        args.push("--release".to_owned());
    }
    if let Some(crate_name) = &run.package {
        args.extend(["-p".to_owned(), crate_name.clone()]);
    } else if !folder.is_empty() {
        args.extend(["--manifest-path".to_owned(), format!("{folder}/Cargo.toml")]);
    }
    for name in names {
        args.extend(["--test".to_owned(), name.to_owned()]);
    }
    Some(Narrowed { args, runs })
}
