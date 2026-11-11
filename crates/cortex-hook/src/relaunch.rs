//! The second half of a daemon start on Windows: the daemon started anew, holding no handle of the
//! process that asked for it.
//!
//! On Windows, `std::process::Command` creates every child with handle inheritance on, so a child
//! gets every inheritable handle of its parent, whatever its standard streams are set to. An agent
//! that leaves its own handles inheritable passes them to its hooks, and a hook to the daemon it
//! starts. Codex does, and at its exit waits for a pipe whose other end the daemon would then hold
//! for as long as it runs.
//!
//! So on Windows a start runs `cortex daemon --relaunch` the usual way: a hidden console, a process
//! group of its own, out of the agent's job when the job allows. That process starts
//! `cortex daemon` itself through `CreateProcessW` with inheritance off, and exits. The daemon
//! keeps the first process's console, group, job and folder, and none of its handles: the
//! inherited handles live only as long as the first process, which does nothing else. A start
//! that fails there is written to the hook log, as a hook's own failed start is. On Unix none of
//! this is needed, since a child's standard streams take descriptors 0 to 2 and Rust opens every
//! other descriptor close-on-exec.

/// The flag of `cortex daemon` that starts the daemon anew instead of running it.
pub const RELAUNCH_FLAG: &str = "--relaunch";

/// Starts `program daemon --root <root>`, with `--adopt` for [`Purpose::Adopt`], with no handle of
/// this process, and returns once it is created. It is not waited for, and its process and thread handles are left to this process's
/// exit, a moment later. A failure is also written to the hook log in the user folder of `home`,
/// when there is one, since this process's own error output goes nowhere.
///
/// # Errors
///
/// The program's or the root's path is not Unicode, or the process could not be created.
#[cfg(windows)]
pub fn relaunch_daemon(
    program: &std::path::Path,
    root: &std::path::Path,
    purpose: crate::spawn::Purpose,
    home: Option<&std::path::Path>,
) -> std::io::Result<()> {
    let started = start_anew(program, root, purpose);
    if let (Err(error), Some(home)) = (&started, home) {
        let now = std::time::SystemTime::now();
        let line = format!(
            "{} - -: the daemon could not be started anew: {error}",
            cortex_core::unix_millis(now)
        );
        let logs = cortex_core::UserFolder::in_home(home).logs_folder();
        // A log that cannot be written leaves the error to the caller, who returns it.
        drop(crate::log::write(&logs, now, &line));
    }
    started
}

/// Starts `program daemon --root <root>` as [`relaunch_daemon`] does, without the log.
///
/// # Errors
///
/// As [`relaunch_daemon`].
#[cfg(windows)]
fn start_anew(
    program: &std::path::Path,
    root: &std::path::Path,
    purpose: crate::spawn::Purpose,
) -> std::io::Result<()> {
    let adopt = (purpose == crate::spawn::Purpose::Adopt).then_some(crate::spawn::ADOPT_FLAG);
    let words = [
        program.to_str(),
        Some("daemon"),
        Some("--root"),
        root.to_str(),
    ];
    let words: Option<Vec<&str>> = words.into_iter().chain(adopt.map(Some)).collect();
    let words = words.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the program's or the repository's path is not Unicode",
        )
    })?;
    // The crate's `current_dir` hands the call a folder name it has already freed, so it is never
    // called: the daemon starts in this process's folder, the root, which the start set. Its
    // `Child` closes the process's handles only when waited for, which this process never does.
    CreateProcessW::Command::new(command_line(&words))
        // INVARIANT: fail-open. The daemon holds no handle of the agent, which would otherwise
        // wait for it.
        .inherit_handles(false)
        .spawn()
        .map(drop)
}

/// The command line that gives a Windows program exactly `words` as its arguments, as the C
/// runtime and Rust read a command line: a word with a space or a tab, or an empty one, is quoted;
/// a quote is escaped with a backslash; the backslashes before a quote, and before a closing quote,
/// are doubled. Words are separated by one space.
#[cfg(any(windows, test))]
fn command_line(words: &[&str]) -> String {
    let mut line = String::new();
    for word in words {
        if !line.is_empty() {
            line.push(' ');
        }
        let is_quoted = word.is_empty() || word.contains([' ', '\t']);
        if is_quoted {
            line.push('"');
        }
        let mut backslashes = 0;
        for character in word.chars() {
            if character == '\\' {
                backslashes += 1;
            } else {
                if character == '"' {
                    line.extend(std::iter::repeat_n('\\', backslashes + 1));
                }
                backslashes = 0;
            }
            line.push(character);
        }
        if is_quoted {
            line.extend(std::iter::repeat_n('\\', backslashes));
            line.push('"');
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked by the rules `CommandLineToArgvW` documents: 2n backslashes before a quote give n
    /// backslashes and the quote's meaning; 2n + 1 give n backslashes and a literal quote;
    /// backslashes anywhere else are literal.
    /// - `C:\plain` has no space: left as it is.
    /// - `C:\Program Files\cortex.exe` has a space: quoted; its backslashes are before no quote.
    /// - `C:\a repo\` ends in a backslash before the closing quote: doubled to `\\"`, which reads
    ///   back as one backslash and the end of the word.
    /// - `say "hi"` holds quotes: each becomes `\"`; the word is quoted for its space.
    /// - `a\"b` holds a backslash before a quote: 1 becomes 2 + 1 = 3, `a\\\"b`, which reads back
    ///   as `a\"b`.
    /// - the empty word is `""`.
    #[test]
    fn words_are_quoted_as_windows_reads_them() {
        assert_eq!(command_line(&[r"C:\plain"]), r"C:\plain");
        assert_eq!(
            command_line(&[r"C:\Program Files\cortex.exe", "daemon"]),
            r#""C:\Program Files\cortex.exe" daemon"#
        );
        assert_eq!(
            command_line(&["--root", r"C:\a repo\"]),
            r#"--root "C:\a repo\\""#
        );
        assert_eq!(command_line(&[r#"say "hi""#]), r#""say \"hi\"""#);
        assert_eq!(command_line(&[r#"a\"b"#]), r#"a\\\"b"#);
        assert_eq!(command_line(&["", "x"]), r#""" x"#);
    }

    /// A start that fails is returned, and written to the hook log of the home folder given.
    #[cfg(windows)]
    #[test]
    fn a_failed_start_is_logged() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let missing = home.path().join("no-such-program.exe");
        let purpose = crate::spawn::Purpose::Serve;
        assert!(relaunch_daemon(&missing, home.path(), purpose, Some(home.path())).is_err());
        let logs = cortex_core::UserFolder::in_home(home.path()).logs_folder();
        let log = std::fs::read_to_string(logs.join("hook.log"))?;
        assert!(
            log.contains("the daemon could not be started anew"),
            "{log}"
        );
        Ok(())
    }
}
