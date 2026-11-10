//! Whether a command runs inside a coding agent's shell rather than a person's terminal, by the
//! variables an agent sets on the commands its tools run. Text in a repository can lead an agent to
//! run any command, so what a command takes as the user's own word it takes only from outside.

use std::ffi::OsString;

/// The variables a coding agent sets on the commands its tools run: Claude Code's for a tool call
/// or a hook (`CLAUDE_CODE_CHILD_SESSION`, in its documentation of environment variables), and
/// Codex's for a shell command (`CODEX_THREAD_ID` and `CODEX_TOOL_CALL_ID`, in its source). A
/// terminal a person types in, an editor's included, sets none of them.
pub const AGENT_SHELL_MARKERS: [&str; 3] = [
    "CLAUDE_CODE_CHILD_SESSION",
    "CODEX_THREAD_ID",
    "CODEX_TOOL_CALL_ID",
];

/// Whether the environment `var` reads holds one of [`AGENT_SHELL_MARKERS`], set and not empty.
///
/// ```
/// use cortex_core::is_agent_shell;
/// assert!(is_agent_shell(|name| (name == "CODEX_THREAD_ID").then(|| "t1".into())));
/// assert!(!is_agent_shell(|_| None));
/// ```
pub fn is_agent_shell(var: impl Fn(&str) -> Option<OsString>) -> bool {
    AGENT_SHELL_MARKERS
        .iter()
        .any(|name| var(name).is_some_and(|value| !value.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each marker alone, set, is an agent's shell; set empty, or none set, or another variable
    /// such as the editor's `CLAUDECODE`, is not.
    #[test]
    fn each_marker_set_is_an_agent_s_shell() {
        for marker in AGENT_SHELL_MARKERS {
            assert!(
                is_agent_shell(|name| (name == marker).then(|| "1".into())),
                "{marker}"
            );
            assert!(
                !is_agent_shell(|name| (name == marker).then(OsString::new)),
                "{marker}"
            );
        }
        assert!(!is_agent_shell(|_| None));
        assert!(!is_agent_shell(
            |name| (name == "CLAUDECODE").then(|| "1".into())
        ));
    }
}
