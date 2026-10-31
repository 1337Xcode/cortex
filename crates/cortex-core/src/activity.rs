//! What the store keeps of agents' activity: a session's status, a task's outcome, an event's
//! kind and the few facts an event carries besides its path and lines.
//!
//! Every value here is a fixed name, a number or a bounded name; nothing holds a prompt, a
//! command, a search pattern or tool output, except the prompt excerpt a user may ask for.

use serde::{Deserialize, Serialize};

use crate::named::named_kind;
use crate::{CompactTrigger, Identifier, StartSource, ToolKind};

/// Most characters of a prompt kept when the user asks for an excerpt.
pub const PROMPT_EXCERPT_CHARS: usize = 200;

named_kind! {
    /// Whether a session still runs.
    SessionStatus {
        /// It has not ended.
        Live => "live",
        /// It ended.
        Ended => "ended",
        /// It stopped sending events without ending.
        Abandoned => "abandoned",
    }
}

named_kind! {
    /// How a task ended.
    TaskOutcome {
        /// A test, build or type check passed after the last edit.
        Success => "success",
        /// The last such check after the last edit failed.
        Failure => "failure",
        /// The edits were undone.
        Reverted => "reverted",
        /// The task was left before it ended.
        Abandoned => "abandoned",
        /// None of the above is known.
        Unknown => "unknown",
    }
}

named_kind! {
    /// What an event row records.
    EventKind {
        /// A session started, resumed, was cleared or compacted.
        SessionStart => "session_start",
        /// A prompt started a task.
        Prompt => "prompt",
        /// A file was read.
        Read => "read",
        /// Files were searched by content or by name.
        Search => "search",
        /// A file was edited or written.
        Edit => "edit",
        /// A shell command ran.
        Shell => "shell",
        /// Another tool ran.
        Tool => "tool",
        /// The conversation was compacted.
        Compact => "compact",
        /// The agent finished answering.
        Stop => "stop",
        /// A subagent started.
        SubagentStart => "subagent_start",
        /// A subagent finished.
        SubagentStop => "subagent_stop",
        /// The session ended.
        SessionEnd => "session_end",
    }
}

named_kind! {
    /// Who reported an event.
    EventSource {
        /// An agent's hook.
        Hook => "hook",
        /// The file watcher, for an edit no hook reported.
        Watcher => "watcher",
    }
}

/// The facts an event row keeps besides its kind, path and lines, stored as JSON. Every field is
/// optional and left out when absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMeta {
    /// Who reported the event, when not a hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<EventSource>,
    /// The tool that ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolKind>,
    /// The tool's exit code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Whether the tool succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_success: Option<bool>,
    /// Whether the user interrupted the tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_interrupted: Option<bool>,
    /// The size of what the tool printed, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
    /// Why the session started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_source: Option<StartSource>,
    /// What started a compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_trigger: Option<CompactTrigger>,
    /// Why the session ended, as the agent names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<Identifier>,
    /// The subagent's type, as the agent names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<Identifier>,
}

/// The start of a prompt, kept only when the user asks: at most [`PROMPT_EXCERPT_CHARS`]
/// characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptExcerpt(String);

impl PromptExcerpt {
    /// The first [`PROMPT_EXCERPT_CHARS`] characters of `prompt`.
    #[must_use]
    pub fn of(prompt: &str) -> Self {
        Self(prompt.chars().take(PROMPT_EXCERPT_CHARS).collect())
    }

    /// An excerpt read back from storage; `None` when it is longer than an excerpt may be.
    #[must_use]
    pub fn stored(text: &str) -> Option<Self> {
        (text.chars().count() <= PROMPT_EXCERPT_CHARS).then(|| Self(text.to_owned()))
    }

    /// The excerpt's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An excerpt keeps the first 200 characters, counted as characters; a stored text longer than
    /// that is not an excerpt.
    #[test]
    fn excerpts_keep_200_characters() {
        let prompt = "ü".repeat(PROMPT_EXCERPT_CHARS + 1);
        let excerpt = PromptExcerpt::of(&prompt);
        assert_eq!(excerpt.as_str().chars().count(), 200);
        assert_eq!(PromptExcerpt::stored(excerpt.as_str()), Some(excerpt));
        assert_eq!(PromptExcerpt::stored(&prompt), None);
    }

    /// The meta of a shell command that failed is written as the JSON typed here by hand, absent
    /// facts left out, and reads back.
    #[test]
    fn meta_keeps_only_the_facts_given() -> Result<(), serde_json::Error> {
        let meta = EventMeta {
            tool: Some(ToolKind::Shell),
            exit_code: Some(1),
            is_success: Some(false),
            output_bytes: Some(812),
            ..EventMeta::default()
        };
        let json = r#"{"tool":"shell","exit_code":1,"is_success":false,"output_bytes":812}"#;
        assert_eq!(serde_json::to_string(&meta)?, json);
        assert_eq!(serde_json::from_str::<EventMeta>(json)?, meta);
        assert_eq!(serde_json::to_string(&EventMeta::default())?, "{}");
        Ok(())
    }
}
