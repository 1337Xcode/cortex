//! What the hook prints for Claude Code to add to its context: Claude Code's hook JSON,
//! `{"hookSpecificOutput":{"hookEventName":...,"additionalContext":...}}`, alone on stdout.

use serde_json::Value;

/// The most characters Claude Code takes whole as a hook's `additionalContext`. Past it, Claude
/// Code keeps the text in a file and gives the agent the file's path and the text's first 2,000
/// characters, which is not the text Cortex gives. Counted in UTF-16 code units, as Claude Code, a
/// JavaScript program, measures a string; never fewer than the text's characters.
pub(crate) const MAX_CONTEXT_CHARS: usize = 10_000;

/// The tags Claude Code rewrites in a hook's `additionalContext`, its `<` written as `&lt;`, so
/// that text from a hook cannot pass for its own reminders. Matched whatever their case, and
/// whatever follows the name: a text the agent would get altered is not given.
const REWRITTEN_TAGS: [&str; 2] = ["<system-reminder", "</system-reminder"];

/// `text` as the hook of `event_name` prints it for Claude Code to add to its context, the two
/// strings written as JSON strings; none when Claude Code would not take the text as it is: past
/// [`MAX_CONTEXT_CHARS`], or holding a tag of [`REWRITTEN_TAGS`].
pub(crate) fn context(event_name: &str, text: &str) -> Option<String> {
    // INVARIANT: budgeted. A text past the agent's limit is not given in part.
    if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
        return None;
    }
    // INVARIANT: true-at-render. A code line the agent would get altered is not given.
    let lowered = text.to_ascii_lowercase();
    if REWRITTEN_TAGS.iter().any(|tag| lowered.contains(tag)) {
        return None;
    }
    // A JSON value is displayed as its compact JSON text, its strings escaped; that cannot fail,
    // and the fields keep the order Claude Code's documentation writes them in.
    let (event_name, text) = (Value::from(event_name), Value::from(text));
    Some(format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":{event_name},"additionalContext":{text}}}}}"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The event's name and the text are written as JSON strings: a quote, a line break and an
    /// escape character escaped, the middle dot as it is.
    #[test]
    fn the_text_is_written_as_claude_code_s_hook_json() {
        assert_eq!(
            context(
                "UserPromptSubmit",
                "[cortex] Work order · \"x\"\nnext\u{1b}[0m\n"
            )
            .as_deref(),
            Some(
                r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"[cortex] Work order · \"x\"\nnext\u001b[0m\n"}}"#
            )
        );
    }

    /// Claude Code takes 10,000 characters whole: a text of 10,000 is given and one of 10,001 is
    /// not. An accented letter is two bytes and one character; a character past the basic plane
    /// (U+1D11E) is two to Claude Code, so 5,000 of them are given and 5,001 are not.
    #[test]
    fn a_text_past_claude_code_s_limit_is_not_given() {
        assert!(context("SessionStart", &"x".repeat(10_000)).is_some());
        assert_eq!(context("SessionStart", &"x".repeat(10_001)), None);
        assert!(context("SessionStart", &"\u{e9}".repeat(10_000)).is_some());
        assert!(context("SessionStart", &"\u{1d11e}".repeat(5_000)).is_some());
        assert_eq!(context("SessionStart", &"\u{1d11e}".repeat(5_001)), None);
    }

    /// A text holding a tag Claude Code rewrites, opening or closing, in any case, is not given;
    /// one that only names it is.
    #[test]
    fn a_text_claude_code_would_rewrite_is_not_given() {
        for rewritten in [
            "x = '<system-reminder>'\n",
            "</system-reminder>\n",
            "<System-Reminder data=\"1\">\n",
        ] {
            assert_eq!(context("UserPromptSubmit", rewritten), None, "{rewritten}");
        }
        assert!(context("UserPromptSubmit", "a system-reminder tag\n").is_some());
    }
}
