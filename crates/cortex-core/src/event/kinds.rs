//! The named values a normalized event carries: which agent sent it, from which point in the
//! agent's loop, why a session started, which kind of tool ran, and what started a compaction.

use crate::named::named_kind;

named_kind! {
    /// The agent that sent an event.
    Agent {
        /// Claude Code.
        Claude => "claude",
        /// Codex CLI.
        Codex => "codex",
        /// Cursor.
        Cursor => "cursor",
        /// Gemini CLI.
        Gemini => "gemini",
        /// Qwen Code.
        Qwen => "qwen",
        /// Factory's Droid.
        Droid => "droid",
        /// Devin's command-line agent.
        Devin => "devin",
        /// OpenHands.
        OpenHands => "openhands",
        /// TRAE.
        Trae => "trae",
        /// JetBrains Junie's command-line agent.
        Junie => "junie",
        /// Augment's Auggie command-line agent.
        Augment => "augment",
        /// The opencode agent.
        Opencode => "opencode",
        /// Any other agent.
        Other => "other",
    }
}

named_kind! {
    /// The point in the agent's loop an event comes from.
    HookKind {
        /// A session started, resumed, was cleared or was compacted.
        SessionStart => "session_start",
        /// The user submitted a prompt.
        Prompt => "prompt",
        /// A tool is about to run.
        PreTool => "pre_tool",
        /// A tool ran, successfully or not.
        PostTool => "post_tool",
        /// The conversation is about to be compacted.
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
    /// Why a session started.
    StartSource {
        /// A new session.
        Startup => "startup",
        /// An earlier session taken up again.
        Resume => "resume",
        /// A session that replaces one the user cleared.
        Clear => "clear",
        /// The same session after its conversation was compacted.
        Compact => "compact",
        /// A new session that branches from an earlier one.
        Fork => "fork",
    }
}

named_kind! {
    /// The kind of tool an event concerns.
    ToolKind {
        /// Reading a file.
        Read => "read",
        /// Searching file contents.
        Grep => "grep",
        /// Finding files by name.
        Glob => "glob",
        /// Replacing text in a file.
        Edit => "edit",
        /// Writing a whole file.
        Write => "write",
        /// Several replacements in one file.
        MultiEdit => "multi_edit",
        /// Applying a patch.
        ApplyPatch => "apply_patch",
        /// Running a shell command.
        Shell => "shell",
        /// Any other tool.
        Other => "other",
    }
}

impl ToolKind {
    /// Whether the tool changes a file: an edit, a write, several edits, or a patch.
    #[must_use]
    pub fn is_edit(self) -> bool {
        matches!(
            self,
            Self::Edit | Self::Write | Self::MultiEdit | Self::ApplyPatch
        )
    }
}

named_kind! {
    /// What started a compaction.
    CompactTrigger {
        /// The user asked for it.
        Manual => "manual",
        /// The agent's context filled up.
        Auto => "auto",
    }
}
