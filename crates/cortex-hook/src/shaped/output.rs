//! What the hook prints for an agent to add to its context, in the hook JSON Claude Code defined:
//! `{"hookSpecificOutput":{"hookEventName":...,"additionalContext":...}}`, alone on stdout. How
//! much text an agent takes whole, and what text it would alter, is the agent's own: its adapter
//! checks the text before it is written.

use serde_json::Value;

/// `text` as the hook of `event_name` prints it for the agent to add to its context, the two
/// strings written as JSON strings.
pub(crate) fn context(event_name: &str, text: &str) -> String {
    // A JSON value is displayed as its compact JSON text, its strings escaped; that cannot fail,
    // and the fields keep the order Claude Code's documentation writes them in.
    let (event_name, text) = (Value::from(event_name), Value::from(text));
    format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":{event_name},"additionalContext":{text}}}}}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The event's name and the text are written as JSON strings: a quote, a line break and an
    /// escape character escaped, the middle dot as it is.
    #[test]
    fn the_text_is_written_as_the_hook_json() {
        assert_eq!(
            context(
                "UserPromptSubmit",
                "[cortex] Work order · \"x\"\nnext\u{1b}[0m\n"
            ),
            r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"[cortex] Work order · \"x\"\nnext\u001b[0m\n"}}"#
        );
    }
}
