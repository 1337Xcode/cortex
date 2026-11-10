//! The programs whose run is known to write no file, to tell a shell line that may have changed
//! the repository's files from one that cannot have. The table is short and each entry certain:
//! a program missing from it, or run with an option that writes or runs another program, is taken
//! to write. A redirection or a command substitution is the line's own matter (`shell::ReadLine`).

use super::argv::Invocation;

/// The programs that write no file whatever their arguments.
const READING: &[&str] = &["cat", "echo", "grep", "head", "ls", "pwd", "tail", "wc"];

/// The programs that write no file unless given one of the options listed: `find` runs commands
/// or writes lists with them, and `rg` runs a preprocessor.
const READING_UNLESS: &[(&str, &[&str])] = &[
    (
        "find",
        &[
            "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf",
            "-fls",
        ],
    ),
    ("rg", &["--pre"]),
];

/// The git commands that write no file, given as git's first argument, unless an argument writes
/// their output to a file or runs an outside diff program.
const GIT_READING: &[&str] = &["diff", "log", "show", "status"];

/// The options of a reading git command that write a file or run another program.
const GIT_WRITING_OPTIONS: &[&str] = &["--output", "--ext-diff", "--textconv"];

/// Whether `invocation` is known to write no file.
pub(super) fn is_read_only(invocation: &Invocation<'_>) -> bool {
    let args = invocation.arg_texts();
    let program = invocation.program.as_str();
    if READING.contains(&program) {
        return true;
    }
    if let Some((_, writing)) = READING_UNLESS.iter().find(|(name, _)| *name == program) {
        return !args
            .iter()
            .any(|arg| writing.iter().any(|option| is_option(arg, option)));
    }
    if program == "git" {
        let is_reading = args
            .first()
            .is_some_and(|command| GIT_READING.contains(command));
        let writes = args.iter().any(|arg| {
            GIT_WRITING_OPTIONS
                .iter()
                .any(|option| is_option(arg, option))
        });
        return is_reading && !writes;
    }
    false
}

/// Whether `arg` is `option`, alone or with its value after `=`.
fn is_option(arg: &str, option: &str) -> bool {
    arg.strip_prefix(option)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('='))
}
