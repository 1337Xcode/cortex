//! go: which arguments select tests, and how a narrow command names test packages. A package other
//! than `./...` and the flags that pick tests by name select; `go test` alone runs the folder's
//! package and is taken as whole. Flags are read with one or two dashes, `-count=1` or `-count 1`.

use super::{SuiteRun, Whole};

/// The test flags that select which tests run, and `-C`, which moves the folder the packages are
/// found from.
const SELECTING: &[&str] = &["run", "skip", "bench", "list", "fuzz", "args", "C"];

/// The flags that take their value as the next word when it is not joined with `=`.
const VALUE_FLAGS: &[&str] = &[
    "count",
    "timeout",
    "coverprofile",
    "covermode",
    "coverpkg",
    "p",
    "parallel",
    "cpu",
    "tags",
    "exec",
    "o",
    "outputdir",
    "ldflags",
    "gcflags",
    "asmflags",
    "toolexec",
    "overlay",
    "modfile",
    "pkgdir",
    "mod",
    "vet",
];

/// The package pattern that stands for every package of the module.
const EVERY_PACKAGE: &str = "./...";

/// Whether the arguments after `go` are a test run of every package, or of the folder's.
pub(super) fn whole(args: &[&str]) -> Option<Whole> {
    let (&command, flags) = args.split_first()?;
    if command != "test" {
        return None;
    }
    let mut at = 0;
    while let Some(&arg) = flags.get(at) {
        at += 1;
        let Some(flag) = arg.strip_prefix('-') else {
            if arg != EVERY_PACKAGE {
                return None;
            }
            continue;
        };
        let bare = flag.trim_start_matches('-');
        let (name, joined) = bare
            .split_once('=')
            .map_or((bare, false), |(name, _)| (name, true));
        if SELECTING.contains(&name) {
            return None;
        }
        at += usize::from(!joined && VALUE_FLAGS.contains(&name));
    }
    Some(Whole {
        tail: vec!["test".to_owned()],
        ..Whole::default()
    })
}

/// The package folder of the Go test file `file`, written `./folder`, or `.` at the top.
fn package_of(file: &str) -> Option<String> {
    if !file.ends_with("_test.go") {
        return None;
    }
    Some(match file.rsplit_once('/') {
        Some((folder, _)) => format!("./{folder}"),
        None => ".".to_owned(),
    })
}

/// The package folders of the Go test files among `files`, each once, in order.
pub(super) fn narrow(_run: &SuiteRun, files: &[&str]) -> Option<Vec<String>> {
    let mut packages: Vec<String> = Vec::new();
    for package in files.iter().filter_map(|file| package_of(file)) {
        if !packages.contains(&package) {
            packages.push(package);
        }
    }
    (!packages.is_empty()).then_some(packages)
}
