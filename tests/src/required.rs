//! The hook payload fixtures each captured agent version must have, and what each must hold.
//!
//! A fixture's file name promises a variant: a tool, a session source, an end reason, a test run
//! that passed or failed. Each entry lists the fields that show the payload is that variant, as
//! JSON pointers into the payload, so a copied or mislabelled fixture fails the check.

use serde_json::Value;

use self::Expect::{Contains, Is, Present, StartsWith};

/// What one field of a required fixture must hold, at a JSON pointer such as `/tool_input/offset`.
#[derive(Debug, Clone, Copy)]
pub enum Expect {
    /// The string at the pointer equals the text.
    Is(&'static str, &'static str),
    /// The string at the pointer starts with the text.
    StartsWith(&'static str, &'static str),
    /// The string at the pointer contains the text.
    Contains(&'static str, &'static str),
    /// The pointer holds a value of any kind.
    Present(&'static str),
}

/// One fixture an agent version must have: its file name without `.json`, and what it must hold.
pub type Required = (&'static str, &'static [Expect]);

/// The fixtures each captured agent version must have: every event and tool the adapters handle,
/// as `(agent, version, fixtures)`.
pub const REQUIRED: &[(&str, &str, &[Required])] = &[
    (
        "claude",
        "2.1.312",
        &[
            ("session-start-startup", &[Is("/source", "startup")]),
            ("session-start-resume", &[Is("/source", "resume")]),
            ("session-start-compact", &[Is("/source", "compact")]),
            ("session-start-clear", &[Is("/source", "clear")]),
            ("user-prompt-submit", &[Present("/prompt")]),
            (
                "pre-tool-use-read",
                &[Is("/tool_name", "Read"), Present("/tool_input/file_path")],
            ),
            (
                "pre-tool-use-grep",
                &[Is("/tool_name", "Grep"), Present("/tool_input/pattern")],
            ),
            (
                "pre-tool-use-glob",
                &[Is("/tool_name", "Glob"), Present("/tool_input/pattern")],
            ),
            (
                "pre-tool-use-edit",
                &[
                    Is("/tool_name", "Edit"),
                    Present("/tool_input/old_string"),
                    Present("/tool_input/new_string"),
                ],
            ),
            (
                "pre-tool-use-write",
                &[Is("/tool_name", "Write"), Present("/tool_input/content")],
            ),
            (
                "pre-tool-use-bash",
                &[Is("/tool_name", "Bash"), Present("/tool_input/command")],
            ),
            (
                "pre-tool-use-agent",
                &[
                    Is("/tool_name", "Agent"),
                    Present("/tool_input/subagent_type"),
                ],
            ),
            (
                "pre-tool-use-grep-subagent",
                &[
                    Is("/tool_name", "Grep"),
                    Is("/agent_type", "Explore"),
                    Present("/agent_id"),
                ],
            ),
            (
                "post-tool-use-read",
                &[
                    Is("/tool_name", "Read"),
                    Present("/tool_response/file/content"),
                ],
            ),
            (
                "post-tool-use-grep",
                &[
                    Is("/tool_name", "Grep"),
                    Present("/tool_response/filenames"),
                ],
            ),
            (
                "post-tool-use-glob",
                &[
                    Is("/tool_name", "Glob"),
                    Present("/tool_response/filenames"),
                ],
            ),
            (
                "post-tool-use-edit",
                &[
                    Is("/tool_name", "Edit"),
                    Present("/tool_response/structuredPatch"),
                ],
            ),
            (
                "post-tool-use-write",
                &[
                    Is("/tool_name", "Write"),
                    Present("/tool_response/filePath"),
                ],
            ),
            (
                "post-tool-use-bash",
                &[Is("/tool_name", "Bash"), Present("/tool_response/stdout")],
            ),
            (
                "post-tool-use-agent",
                &[
                    Is("/tool_name", "Agent"),
                    Is("/tool_response/status", "completed"),
                ],
            ),
            (
                "post-tool-use-grep-subagent",
                &[
                    Is("/tool_name", "Grep"),
                    Is("/agent_type", "Explore"),
                    Present("/agent_id"),
                ],
            ),
            (
                "post-tool-use-failure-bash",
                &[
                    Is("/tool_name", "Bash"),
                    StartsWith("/error", "Exit code 3"),
                ],
            ),
            (
                "post-tool-use-bash-test-passing",
                &[
                    Is("/tool_name", "Bash"),
                    Contains("/tool_input/command", "unittest"),
                    Contains("/tool_response/stdout", "\nOK"),
                ],
            ),
            (
                "post-tool-use-failure-bash-test",
                &[
                    Is("/tool_name", "Bash"),
                    Contains("/tool_input/command", "unittest"),
                    StartsWith("/error", "Exit code 1"),
                    Contains("/error", "FAILED (failures=1)"),
                ],
            ),
            (
                "pre-tool-use-read-range",
                &[
                    Is("/tool_name", "Read"),
                    Present("/tool_input/offset"),
                    Present("/tool_input/limit"),
                ],
            ),
            (
                "post-tool-use-read-range",
                &[
                    Is("/tool_name", "Read"),
                    Present("/tool_input/offset"),
                    Present("/tool_input/limit"),
                    Present("/tool_response/file/startLine"),
                ],
            ),
            (
                "pre-tool-use-powershell",
                &[
                    Is("/tool_name", "PowerShell"),
                    Present("/tool_input/command"),
                ],
            ),
            (
                "post-tool-use-powershell",
                &[
                    Is("/tool_name", "PowerShell"),
                    Present("/tool_response/stdout"),
                ],
            ),
            (
                "post-tool-use-failure-powershell",
                &[
                    Is("/tool_name", "PowerShell"),
                    StartsWith("/error", "Exit code 1"),
                ],
            ),
            ("pre-compact-manual", &[Is("/trigger", "manual")]),
            (
                "post-compact-manual",
                &[Is("/trigger", "manual"), Present("/compact_summary")],
            ),
            (
                "subagent-start",
                &[Is("/agent_type", "Explore"), Present("/agent_id")],
            ),
            (
                "subagent-stop",
                &[
                    Is("/agent_type", "Explore"),
                    Present("/agent_transcript_path"),
                ],
            ),
            (
                "subagent-stop-internal",
                &[Is("/agent_type", ""), Present("/agent_transcript_path")],
            ),
            ("stop", &[Present("/stop_hook_active")]),
            ("session-end", &[Is("/reason", "other")]),
            ("session-end-clear", &[Is("/reason", "clear")]),
        ],
    ),
    (
        "codex",
        "0.165.0",
        &[
            ("session-start-startup", &[Is("/source", "startup")]),
            (
                "session-start-resume",
                &[
                    Is("/source", "resume"),
                    StartsWith("/transcript_path", "\\\\?\\"),
                ],
            ),
            ("session-start-compact", &[Is("/source", "compact")]),
            ("user-prompt-submit", &[Present("/prompt")]),
            (
                "pre-tool-use-bash-grep",
                &[
                    Is("/tool_name", "Bash"),
                    StartsWith("/tool_input/command", "git grep"),
                ],
            ),
            (
                "pre-tool-use-bash-read",
                &[
                    Is("/tool_name", "Bash"),
                    StartsWith("/tool_input/command", "Get-Content"),
                ],
            ),
            (
                "pre-tool-use-bash-test",
                &[
                    Is("/tool_name", "Bash"),
                    Contains("/tool_input/command", "unittest"),
                ],
            ),
            (
                "pre-tool-use-bash-subagent",
                &[
                    Is("/tool_name", "Bash"),
                    Is("/agent_type", "default"),
                    Present("/agent_id"),
                ],
            ),
            (
                "pre-tool-use-apply-patch",
                &[
                    Is("/tool_name", "apply_patch"),
                    Contains("/tool_input/command", "*** Update File: "),
                ],
            ),
            (
                "pre-tool-use-spawn-agent",
                &[
                    Is("/tool_name", "collaborationspawn_agent"),
                    Present("/tool_input/task_name"),
                ],
            ),
            (
                "pre-tool-use-wait-agent",
                &[
                    Is("/tool_name", "collaborationwait_agent"),
                    Present("/tool_input/timeout_ms"),
                ],
            ),
            (
                "post-tool-use-bash-grep",
                &[
                    Is("/tool_name", "Bash"),
                    StartsWith("/tool_input/command", "git grep"),
                    Present("/tool_response"),
                ],
            ),
            (
                "post-tool-use-bash-read",
                &[
                    Is("/tool_name", "Bash"),
                    StartsWith("/tool_input/command", "Get-Content"),
                    Present("/tool_response"),
                ],
            ),
            (
                "post-tool-use-bash-test-failing",
                &[
                    Is("/tool_name", "Bash"),
                    Contains("/tool_input/command", "unittest"),
                    Contains("/tool_response", "FAILED (failures=1)"),
                ],
            ),
            (
                "post-tool-use-bash-test-passing",
                &[
                    Is("/tool_name", "Bash"),
                    Contains("/tool_input/command", "unittest"),
                    Contains("/tool_response", "\nOK"),
                ],
            ),
            (
                "post-tool-use-bash-nonzero-exit",
                &[
                    Is("/tool_name", "Bash"),
                    Contains("/tool_input/command", "sys.exit(3)"),
                    Is("/tool_response", ""),
                ],
            ),
            (
                "post-tool-use-bash-subagent",
                &[
                    Is("/tool_name", "Bash"),
                    Is("/agent_type", "default"),
                    Present("/agent_id"),
                ],
            ),
            (
                "post-tool-use-apply-patch",
                &[
                    Is("/tool_name", "apply_patch"),
                    Contains("/tool_input/command", "*** Update File: "),
                    StartsWith("/tool_response", "Exit code: 0"),
                ],
            ),
            (
                "post-tool-use-apply-patch-add",
                &[
                    Is("/tool_name", "apply_patch"),
                    Contains("/tool_input/command", "*** Add File: "),
                    StartsWith("/tool_response", "Exit code: 0"),
                ],
            ),
            (
                "post-tool-use-spawn-agent",
                &[
                    Is("/tool_name", "collaborationspawn_agent"),
                    Contains("/tool_response", "task_name"),
                ],
            ),
            (
                "post-tool-use-wait-agent",
                &[
                    Is("/tool_name", "collaborationwait_agent"),
                    Contains("/tool_response", "Wait completed"),
                ],
            ),
            ("pre-compact-auto", &[Is("/trigger", "auto")]),
            ("post-compact-auto", &[Is("/trigger", "auto")]),
            (
                "subagent-start",
                &[Is("/agent_type", "default"), Present("/agent_id")],
            ),
            (
                "subagent-stop",
                &[
                    Is("/agent_type", "default"),
                    Present("/agent_transcript_path"),
                ],
            ),
            ("stop", &[Present("/stop_hook_active")]),
            ("session-end", &[Is("/reason", "other")]),
        ],
    ),
];

/// Every expectation `payload` does not meet, one line each.
#[must_use]
pub fn unmet(payload: &Value, expects: &[Expect]) -> Vec<String> {
    expects
        .iter()
        .filter(|expect| !is_met(payload, **expect))
        .map(|expect| {
            let pointer = match *expect {
                Is(pointer, _)
                | StartsWith(pointer, _)
                | Contains(pointer, _)
                | Present(pointer) => pointer,
            };
            let found = payload.pointer(pointer).map(|value| {
                let mut text = value.to_string();
                text.truncate(text.floor_char_boundary(80));
                text
            });
            format!("{expect:?} does not hold; found {found:?}")
        })
        .collect()
}

/// Whether `payload` meets one expectation.
fn is_met(payload: &Value, expect: Expect) -> bool {
    let text_at = |pointer: &str| payload.pointer(pointer).and_then(Value::as_str);
    match expect {
        Is(pointer, text) => text_at(pointer) == Some(text),
        StartsWith(pointer, text) => text_at(pointer).is_some_and(|found| found.starts_with(text)),
        Contains(pointer, text) => text_at(pointer).is_some_and(|found| found.contains(text)),
        Present(pointer) => payload.pointer(pointer).is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A payload with a nested field, used by the checks below.
    fn payload() -> Value {
        serde_json::json!({
            "tool_name": "Read",
            "tool_input": {"file_path": "C:\\repo\\a.py", "offset": 2},
            "error": "Exit code 1\nFAILED (failures=1)"
        })
    }

    /// Each kind of expectation holds on a payload that has the field and the text.
    #[test]
    fn met_expectations_report_nothing() {
        let expects = [
            Is("/tool_name", "Read"),
            Present("/tool_input/offset"),
            StartsWith("/error", "Exit code 1"),
            Contains("/error", "FAILED (failures=1)"),
        ];
        assert_eq!(unmet(&payload(), &expects), Vec::<String>::new());
    }

    /// A wrong value, a missing nested field, a wrong prefix, missing text and a non-string value
    /// are each reported once.
    #[test]
    fn unmet_expectations_are_reported() {
        let expects = [
            Is("/tool_name", "Grep"),
            Present("/tool_input/limit"),
            StartsWith("/error", "Exit code 3"),
            Contains("/error", "OK"),
            Is("/tool_input/offset", "2"),
        ];
        let problems = unmet(&payload(), &expects);
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert_eq!(
            problems.first().map(String::as_str),
            Some(r#"Is("/tool_name", "Grep") does not hold; found Some("\"Read\"")"#)
        );
        assert_eq!(
            problems.get(1).map(String::as_str),
            Some(r#"Present("/tool_input/limit") does not hold; found None"#)
        );
    }
}
