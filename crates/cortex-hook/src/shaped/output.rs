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

/// `message` shown to the user and never the model, in the field both Claude Code and Codex read
/// for it (`systemMessage`), beside `text` for the context of the hook of `event_name` as
/// [`context`] writes it, when there is one.
pub(crate) fn with_message(event_name: &str, text: Option<&str>, message: &str) -> String {
    let message = Value::from(message);
    match text {
        Some(text) => {
            let (event_name, text) = (Value::from(event_name), Value::from(text));
            format!(
                r#"{{"systemMessage":{message},"hookSpecificOutput":{{"hookEventName":{event_name},"additionalContext":{text}}}}}"#
            )
        }
        None => format!(r#"{{"systemMessage":{message}}}"#),
    }
}

/// `text` as a hook prints it for an agent that reads it from `field` at the top of the object:
/// `additionalContext` for OpenHands and Copilot, `additional_context` for Cursor, `context` for
/// Crush and Hermes.
pub(crate) fn top_level(field: &str, text: &str) -> String {
    format!("{{{}:{}}}", Value::from(field), Value::from(text))
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

    /// A message for the user comes first, as its own field, with the context after it when there
    /// is one, both written as JSON strings.
    #[test]
    fn a_message_for_the_user_is_its_own_field() {
        assert_eq!(
            with_message("SessionStart", Some("primer\n"), "Cortex: \"x\" failed"),
            r#"{"systemMessage":"Cortex: \"x\" failed","hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"primer\n"}}"#
        );
        assert_eq!(
            with_message("SessionStart", None, "Cortex: y failed"),
            r#"{"systemMessage":"Cortex: y failed"}"#
        );
    }
}
