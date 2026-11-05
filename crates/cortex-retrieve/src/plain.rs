//! Text that can be put in front of an agent as it is. Everything Cortex writes for an agent names
//! files and symbols by what the repository holds, and a repository is untrusted: a file name can
//! hold a line break, a control character, a character that reorders or hides text, or one of an
//! agent's reminder tags. The work order and the primer hold what they name to this one check, so a
//! crafted name cannot add a line, a hidden instruction or a tag to what the agent reads.

use cortex_core::{holds_reminder_tag, is_hidden};

/// Whether `character` is taken out of what Cortex writes for an agent: a control character other
/// than a tab, or one that reorders or hides text (`cortex_core::is_hidden`).
pub(crate) fn is_taken_out(character: char) -> bool {
    character != '\t' && (character.is_control() || is_hidden(character))
}

/// Whether `text` can be named to an agent as it is: no character [`is_taken_out`] names and none
/// of an agent's reminder tags.
pub(crate) fn is_plain(text: &str) -> bool {
    text.chars().all(|character| !is_taken_out(character)) && !holds_reminder_tag(text)
}
