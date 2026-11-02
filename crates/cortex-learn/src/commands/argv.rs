//! A simple command's program and arguments, as the check it runs is told from them. The variable
//! assignments before the command are passed over, and so are the programs that only run another
//! after their own options: `env`, `time`, `command`, `exec`, `nohup`, `nice` and `timeout`;
//! `npx`, `bunx` and `pnpx`; `uv`, `poetry`, `pipenv`, `pdm`, `hatch` and `rye` with `run`;
//! `bundle exec`; and Python with `-m`, whose module is the program. A program is named by its
//! file name in lower case, without `.exe`, `.cmd`, `.bat` or `.ps1`.

use super::shell::Word;
use super::tail;

/// The options, by program run before another, that take a value as the next word.
const VALUE_OPTIONS: &[(&str, &[&str])] = &[
    ("env", &["-u", "--unset", "-C", "--chdir"]),
    ("exec", &["-a"]),
    ("nice", &["-n", "--adjustment"]),
    ("timeout", &["-s", "--signal", "-k", "--kill-after"]),
    ("npx", &["-p", "--package", "-c", "--call"]),
    ("bunx", &["-p", "--package"]),
    ("pnpx", &["-p", "--package"]),
    (
        "uv",
        &[
            "--with",
            "--python",
            "-p",
            "--project",
            "--directory",
            "--extra",
            "--group",
            "--env-file",
            "--package",
        ],
    ),
    ("poetry", &["-C", "--directory", "-P", "--project"]),
];

/// The programs that run the one their arguments name after `run`.
const RUN_WRAPPERS: &[&str] = &["uv", "poetry", "pipenv", "pdm", "hatch", "rye"];

/// The interpreter options of Python that take a value as the next word.
const PYTHON_VALUE_OPTIONS: &[&str] = &["-X", "-W", "-Q"];

/// What a simple command runs: the program's name and the words after it.
#[derive(Debug)]
pub(super) struct Invocation<'a> {
    /// The program, by its file name in lower case without its extension.
    pub(super) program: String,
    /// The words after the program.
    pub(super) args: &'a [Word],
}

impl Invocation<'_> {
    /// The arguments' texts, as written once their quotes are removed.
    pub(super) fn arg_texts(&self) -> Vec<&str> {
        self.args.iter().map(|word| word.text.as_str()).collect()
    }
}

/// What the command of `words` runs, past the assignments before it and the programs that only
/// run another. `None` when it runs nothing (assignments alone, `command -v` asking where a
/// program is, Python running code or a script rather than a module), or when the word that names
/// the program is not literal.
pub(super) fn invocation(words: &[Word]) -> Option<Invocation<'_>> {
    let mut rest = after_assignments(words);
    loop {
        let name = program_name(rest.first()?.literal()?);
        let options = value_options(&name);
        let after = match name.as_str() {
            "env" => after_assignments(skip_options(tail(rest, 1), options)),
            "command" if is_word(rest.get(1), "-v") || is_word(rest.get(1), "-V") => return None,
            "time" | "command" | "exec" | "nohup" | "nice" | "npx" | "bunx" | "pnpx" => {
                skip_options(tail(rest, 1), options)
            }
            "timeout" => tail(skip_options(tail(rest, 1), options), 1),
            "bundle" if is_word(rest.get(1), "exec") => skip_options(tail(rest, 2), &[]),
            _ if RUN_WRAPPERS.contains(&name.as_str()) && is_word(rest.get(1), "run") => {
                skip_options(tail(rest, 2), options)
            }
            _ if is_python(&name) => match python_module(tail(rest, 1))? {
                PythonRun::Module(at) => tail(rest, 1 + at),
                PythonRun::Interpreter => break,
            },
            _ => break,
        };
        rest = after;
    }
    Some(Invocation {
        program: program_name(rest.first()?.literal()?),
        args: tail(rest, 1),
    })
}

/// The command's text as the user's patterns are matched against: its words after the assignments
/// before it, joined by single spaces.
pub(super) fn command_text(words: &[Word]) -> String {
    after_assignments(words)
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The name a program's word gives it: its file name in lower case, without `.exe`, `.cmd`,
/// `.bat` or `.ps1`.
pub(super) fn program_name(word: &str) -> String {
    let file = word
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(word)
        .to_lowercase();
    for extension in [".exe", ".cmd", ".bat", ".ps1"] {
        if let Some(stem) = file.strip_suffix(extension) {
            return stem.to_owned();
        }
    }
    file
}

/// `words` from the first that is not a variable assignment.
pub(super) fn after_assignments(words: &[Word]) -> &[Word] {
    let at = words
        .iter()
        .position(|word| !is_assignment(word))
        .unwrap_or(words.len());
    tail(words, at)
}

/// Whether `word` is there and is literally `text`.
fn is_word(word: Option<&Word>, text: &str) -> bool {
    word.and_then(Word::literal) == Some(text)
}

/// The options of `program` that take a value as the next word.
fn value_options(program: &str) -> &'static [&'static str] {
    VALUE_OPTIONS
        .iter()
        .find(|(name, _)| *name == program)
        .map_or(&[], |(_, options)| options)
}

/// Whether `word` assigns a variable: a name of letters, digits and `_`, not starting with a
/// digit, then `=`.
fn is_assignment(word: &Word) -> bool {
    let Some((name, _)) = word.text.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `words` past the options at their start, the value of each option in `with_value` with it, and
/// a `--` that ends them.
fn skip_options<'a>(words: &'a [Word], with_value: &[&str]) -> &'a [Word] {
    let mut at = 0;
    while let Some(word) = words.get(at) {
        let text = word.text.as_str();
        if text == "--" {
            return tail(words, at + 1);
        }
        if !text.starts_with('-') || text == "-" {
            break;
        }
        at += if with_value.contains(&text) { 2 } else { 1 };
    }
    tail(words, at)
}

/// Whether the program `name` is a Python interpreter: `python`, `python3`, `python3.12`, `py`.
fn is_python(name: &str) -> bool {
    name == "py"
        || name
            .strip_prefix("python")
            .is_some_and(|version| version.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

/// What a Python interpreter runs.
enum PythonRun {
    /// A module, `-m` naming it: the module's word is at this place in the arguments.
    Module(usize),
    /// Anything else: a script, code given with `-c`, or the interpreter itself.
    Interpreter,
}

/// What the Python interpreter given `args` runs; `None` when a word before the module is not
/// literal.
fn python_module(args: &[Word]) -> Option<PythonRun> {
    let mut at = 0;
    while let Some(word) = args.get(at) {
        let text = word.literal()?;
        if text == "-m" {
            return Some(PythonRun::Module(at + 1));
        }
        if !text.starts_with('-') || text == "-c" || text == "-" {
            return Some(PythonRun::Interpreter);
        }
        at += if PYTHON_VALUE_OPTIONS.contains(&text) {
            2
        } else {
            1
        };
    }
    Some(PythonRun::Interpreter)
}
