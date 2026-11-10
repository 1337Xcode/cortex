//! unittest: which arguments select tests, and how a narrow command names test modules. With no
//! argument, or `discover`, it runs every test the discovery finds; a module, class or method
//! name, `-k` or discovery's `-p` pattern selects. Discovery's start and top folders (`-s`, `-t`)
//! put another folder first on the import path, so a module named from the line's folder may not
//! import as it did: they leave the run uncertain.

use super::{Narrowed, SuiteRun, Whole};

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

/// The flag that takes a value as the next word and selects nothing.
const DURATIONS: &str = "--durations";

/// Whether the arguments after `unittest` select nothing; then the flags among them are the run's
/// options, `discover` left out, as the narrow command names modules instead.
pub(super) fn whole(args: &[&str]) -> Option<Whole> {
    let mut options: Vec<String> = Vec::new();
    let mut at = 0;
    let mut is_discovering = false;
    while let Some(&arg) = args.get(at) {
        at += 1;
        if arg == "discover" && !is_discovering {
            is_discovering = true;
        } else if arg == DURATIONS {
            options.push(arg.to_owned());
            options.push((*args.get(at)?).to_owned());
            at += 1;
        } else if NON_SELECTING.contains(&arg) {
            options.push(arg.to_owned());
        } else {
            return None;
        }
    }
    Some(Whole {
        options,
        ..Whole::default()
    })
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
pub(super) fn narrow(_run: &SuiteRun, files: &[&str]) -> Option<Narrowed> {
    let mut found = Narrowed::default();
    for (place, file) in files.iter().enumerate() {
        if let Some(module) = module_of(file) {
            found.add(place, module);
        }
    }
    found.found()
}
