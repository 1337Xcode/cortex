//! The command line an agent runs at one of its hooks: this program with `hook --agent <agent>
//! --event <event>`, written for the shell the agent runs commands through. A program whose path
//! holds no space and no character a shell reads (letters, digits, `.`, `_`, `-`, `/` and a drive's
//! `:`) is written bare, with `/` between folders, which every one of these shells runs as it is;
//! any other path is quoted for the agent's shell, and is not written for a shell Cortex does not
//! know, since a wrong quote would run something else.

/// The shell an agent hands a hook's command to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shell {
    /// A POSIX shell (`sh -c`, `bash -c`, or one embedded in the agent).
    Posix,
    /// Windows' `cmd /c`.
    Cmd,
    /// Not documented for this system.
    Unknown,
}

/// The line that runs `program` at `event` of `agent` through `shell`, or `None` when the path
/// needs quoting and the shell is not known.
pub(crate) fn hook_command(
    program: &str,
    agent: &str,
    event: &str,
    shell: Shell,
) -> Option<String> {
    let program = written(program, shell)?;
    Some(format!("{program} hook --agent {agent} --event {event}"))
}

/// `program` as `shell` reads it as one word.
fn written(program: &str, shell: Shell) -> Option<String> {
    // Only a Windows path's backslashes are separators; elsewhere a backslash is part of a name.
    let is_windows = program.as_bytes().get(1..3) == Some(b":\\".as_slice());
    let forward = if is_windows {
        program.replace('\\', "/")
    } else {
        program.to_owned()
    };
    if forward
        .chars()
        .all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-' | '/' | ':'))
    {
        return Some(forward);
    }
    match shell {
        Shell::Posix => Some(format!("'{}'", forward.replace('\'', r"'\''"))),
        // A Windows path holds no double quote, so one pair quotes it whole.
        Shell::Cmd if !program.contains('"') => Some(format!("\"{program}\"")),
        Shell::Cmd | Shell::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain path is written bare, with `/` between folders, whatever the shell, the unknown one
    /// included.
    #[test]
    fn a_plain_path_is_written_bare() {
        for shell in [Shell::Posix, Shell::Cmd, Shell::Unknown] {
            assert_eq!(
                hook_command(r"C:\Users\dev\.cargo\bin\cortex.exe", "qwen", "Stop", shell)
                    .as_deref(),
                Some("C:/Users/dev/.cargo/bin/cortex.exe hook --agent qwen --event Stop")
            );
        }
        assert_eq!(
            hook_command(
                "/usr/local/bin/cortex",
                "crush",
                "PostToolUse",
                Shell::Posix
            )
            .as_deref(),
            Some("/usr/local/bin/cortex hook --agent crush --event PostToolUse")
        );
    }

    /// A path with a space or a quote is quoted for each known shell, by hand: POSIX in single
    /// quotes with `'\''` for a quote, `cmd` in double quotes; for a shell not known, nothing is
    /// written. A backslash in a path that is no Windows path is part of a name, kept.
    #[test]
    fn another_path_is_quoted_for_its_shell() {
        assert_eq!(
            hook_command("/home/Ann Lee/bin/cortex", "junie", "Stop", Shell::Posix).as_deref(),
            Some("'/home/Ann Lee/bin/cortex' hook --agent junie --event Stop")
        );
        assert_eq!(
            hook_command("/home/o'neil/cortex", "qwen", "Stop", Shell::Posix).as_deref(),
            Some(r"'/home/o'\''neil/cortex' hook --agent qwen --event Stop")
        );
        assert_eq!(
            hook_command(r"/opt/a\b/cortex", "qwen", "Stop", Shell::Posix).as_deref(),
            Some(r"'/opt/a\b/cortex' hook --agent qwen --event Stop")
        );
        let windows = r"C:\Users\Ann Lee\.cargo\bin\cortex.exe";
        assert_eq!(
            hook_command(windows, "junie", "Stop", Shell::Cmd).as_deref(),
            Some(r#""C:\Users\Ann Lee\.cargo\bin\cortex.exe" hook --agent junie --event Stop"#)
        );
        assert_eq!(hook_command(windows, "qwen", "Stop", Shell::Unknown), None);
    }
}
