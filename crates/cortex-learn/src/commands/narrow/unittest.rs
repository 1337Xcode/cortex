//! unittest: which arguments select tests, and how a narrow command names test modules. With no
//! argument, or `discover`, it runs every test the discovery finds; a module, class or method
//! name, `-k` or discovery's `-p` pattern selects.

use super::{SuiteRun, Whole};

/// The flags that only shape the run.
const NON_SELECTING: &[&str] = &[
    "-v",
    "--verbose",
    "-q",
    "--quiet",
    "-f",
    "--failfast",
    "-c",
    "--catch",
    "-b",
    "--buffer",
    "--locals",
];

/// The discovery flags that name where to look, with a value as the next word. A start folder
/// is the suite's own, not a selection.
const DISCOVERY_FLAGS: &[&str] = &["-s", "--start-directory", "-t", "--top-level-directory"];

/// Whether the arguments after `unittest` select nothing.
pub(super) fn whole(args: &[&str]) -> Option<Whole> {
    let mut at = 0;
    let mut is_discovering = false;
    while let Some(&arg) = args.get(at) {
        at += 1;
        if arg == "discover" && !is_discovering {
            is_discovering = true;
        } else if arg == "--durations" || (DISCOVERY_FLAGS.contains(&arg) && is_discovering) {
            // Each takes a value, which selects nothing.
            at += 1;
        } else if !NON_SELECTING.contains(&arg) {
            return None;
        }
    }
    Some(Whole::default())
}

/// The dotted module name of the Python file `file`, when its folders and name are identifiers.
fn module_of(file: &str) -> Option<String> {
    let stem = file.strip_suffix(".py")?;
    let names: Vec<&str> = stem.split('/').collect();
    let is_module =
        names.iter().all(|name| is_identifier(name)) && names.last() != Some(&"__init__");
    is_module.then(|| names.join("."))
}

/// Whether `name` is a Python identifier made of ASCII letters, digits and `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The modules of the Python files among `files`, in order.
pub(super) fn narrow(_run: &SuiteRun, files: &[&str]) -> Option<Vec<String>> {
    let modules: Vec<String> = files.iter().filter_map(|file| module_of(file)).collect();
    (!modules.is_empty()).then_some(modules)
}
