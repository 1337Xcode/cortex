//! go: which arguments select tests, and how a narrow command names test packages. A package other
//! than `./...` and the flags that pick tests by name select, and `-c` and `-n`, which run none,
//! make the run uncertain; `go test` alone runs the folder's package and is taken as whole. Flags
//! are read with one or two dashes, `-count=1` or `-count 1`. The flags that report on the run or
//! measure its coverage are not repeated in the narrow command: its coverage profile or JSON would
//! replace the whole run's.

use super::{Narrowed, SuiteRun, Whole};

/// The test flags that select which tests run, `-C`, which moves the folder the packages are found
/// from, and `-c` and `-n`, which build or print the tests without running them.
const SELECTING: &[&str] = &[
    "run", "skip", "bench", "list", "fuzz", "args", "C", "c", "n",
];

/// The flags that report on the run or measure its coverage, kept out of the narrow command.
const REPORTING: &[&str] = &[
    "cover",
    "covermode",
    "coverpkg",
    "coverprofile",
    "json",
    "o",
    "outputdir",
    "cpuprofile",
    "memprofile",
    "blockprofile",
    "mutexprofile",
    "trace",
];

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

/// Whether the arguments after `go` are a test run of every package, or of the folder's; then its
/// flags, with their values, are the run's options.
pub(super) fn whole(args: &[&str]) -> Option<Whole> {
    let (&command, flags) = args.split_first()?;
    if command != "test" {
        return None;
    }
    let mut options: Vec<String> = Vec::new();
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
        let value = if !joined && VALUE_FLAGS.contains(&name) {
            at += 1;
            Some(*flags.get(at - 1)?)
        } else {
            None
        };
        if !REPORTING.contains(&name) {
            options.push(arg.to_owned());
            options.extend(value.map(str::to_owned));
        }
    }
    Some(Whole {
        tail: vec!["test".to_owned()],
        options,
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
pub(super) fn narrow(_run: &SuiteRun, files: &[&str]) -> Option<Narrowed> {
    let mut found = Narrowed::default();
    for (place, file) in files.iter().enumerate() {
        if let Some(package) = package_of(file) {
            found.add(place, package);
        }
    }
    found.found()
}
