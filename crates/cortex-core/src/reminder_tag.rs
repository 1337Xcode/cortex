//! The markup an agent keeps for its own words: the tags Claude Code wraps its reminders in. Text
//! Cortex gives an agent never holds one, whoever wrote it: the agent would get it altered (Claude
//! Code writes such a tag's `<` as `&lt;` in a hook's context), and text that passes for the
//! agent's own reminders could steer it.

/// The tags agents keep for their own reminders, as written in lower case: Claude Code's opening and
/// closing `system-reminder` tags. Matched whatever their case and whatever follows the name.
pub const REMINDER_TAGS: [&str; 2] = ["<system-reminder", "</system-reminder"];

/// Whether `text` holds a tag of [`REMINDER_TAGS`], in any case.
#[must_use]
pub fn holds_reminder_tag(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    REMINDER_TAGS.iter().any(|tag| lowered.contains(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An opening tag, a closing one and one in mixed case with an attribute are held; the tag's
    /// name without its `<` is not.
    #[test]
    fn tags_are_found_in_any_case() {
        for holding in [
            "x = '<system-reminder>'",
            "</system-reminder>",
            "<System-Reminder data=\"1\">",
        ] {
            assert!(holds_reminder_tag(holding), "{holding}");
        }
        assert!(!holds_reminder_tag("a system-reminder tag"));
        assert!(!holds_reminder_tag("<system reminder>"));
    }
}
