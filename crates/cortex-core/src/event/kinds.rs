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
        /// GitHub Copilot's agent: Copilot CLI and Visual Studio Code's agent mode.
        Copilot => "copilot",
        /// Google Antigravity: its app, its `agy` command-line agent and its editor.
        Antigravity => "antigravity",
        /// Kiro: its editor and its `kiro-cli` command-line agent.
        Kiro => "kiro",
        /// Cline's Visual Studio Code extension.
        Cline => "cline",
        /// Charm's Crush.
        Crush => "crush",
        /// Nous Research's Hermes Agent.
        Hermes => "hermes",
        /// The opencode agent.
        Opencode => "opencode",
        /// Kilo Code: its command-line agent and its editor extensions.
        Kilo => "kilo",
        /// Sourcegraph's Amp.
        Amp => "amp",
        /// OpenClaw's Gateway and its agents.
        OpenClaw => "openclaw",
        /// Any other agent.
        Other => "other",
    }
}

impl Agent {
    /// The agent's name as people write it, the one place text for a person or an agent names it:
    /// `Claude Code`, `Codex`, `Gemini CLI`.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::Gemini => "Gemini CLI",
            Self::Qwen => "Qwen Code",
            Self::Droid => "Droid",
            Self::Devin => "Devin",
            Self::OpenHands => "OpenHands",
            Self::Trae => "TRAE",
            Self::Junie => "Junie",
            Self::Augment => "Augment",
            Self::Copilot => "GitHub Copilot",
            Self::Antigravity => "Antigravity",
            Self::Kiro => "Kiro",
            Self::Cline => "Cline",
            Self::Crush => "Crush",
            Self::Hermes => "Hermes Agent",
            Self::Opencode => "opencode",
            Self::Kilo => "Kilo Code",
            Self::Amp => "Amp",
            Self::OpenClaw => "OpenClaw",
            Self::Other => "Another agent",
        }
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
