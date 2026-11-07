//! The Amp adapter, built from Amp's plugin documentation (read 2026-11-06) and not yet run live.
//! Amp's hooks are a plugin's handlers, so Cortex writes a plugin (`plugins/amp.js`) that sends
//! each event in Claude Code's hook JSON, under the Amp event's name, and gives Amp the text back:
//! a thread's primer and its prompt's work order after the user's message, a read's hint and a
//! tool's warnings after a tool's text output. The thread is the session; the working folder is
//! the plugin's process's. A shell's command is its input's `cmd`, a file tool's file its `path`;
//! a call's result is the event's `status`.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::plugin::{PluginFile, plain, script};
use crate::shaped::normalize;
use crate::shaped::table::{Outcome, PromptField, ToolTable};

/// The glue wiring the plugin's core to Amp's plugin API.
const GLUE: &str = include_str!("../plugins/amp.js");

/// Amp's tools Cortex reads, by their names; a shell's command moved to Claude Code's place.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("Read", ToolKind::Read),
        ("edit_file", ToolKind::Edit),
        ("create_file", ToolKind::Write),
        ("apply_patch", ToolKind::ApplyPatch),
        ("Grep", ToolKind::Grep),
        ("glob", ToolKind::Glob),
        ("Bash", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[("/tool_input/cmd", "/tool_input/command")],
    outcome: Outcome::ResponseSuccess,
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// The events the plugin sends, by Amp's names for them.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "session.start",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "agent.start",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: true,
    },
    HookEntry {
        event_name: "tool.call",
        matcher: None,
        hook: HookKind::PreTool,
        hard_timeout: Duration::from_millis(300),
        gives_context: true,
    },
    HookEntry {
        event_name: "tool.result",
        matcher: None,
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "agent.end",
        matcher: None,
        hook: HookKind::Stop,
        hard_timeout: Duration::from_millis(300),
        gives_context: false,
    },
];

/// Amp.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Amp;

impl Adapter for Amp {
    fn agent(&self) -> Agent {
        Agent::Amp
    }

    fn hooks(&self) -> &'static [HookEntry] {
        HOOKS
    }

    fn versions(&self) -> Option<Versions> {
        None
    }

    fn normalize(
        &self,
        event_name: &str,
        payload: &[u8],
        locate: &dyn Fn(&Path) -> Option<Located>,
        stamp: Stamp,
    ) -> Result<Option<Event>, NormalizeError> {
        let Some(entry) = HOOKS.iter().find(|entry| entry.event_name == event_name) else {
            return Ok(None);
        };
        normalize::event((Agent::Amp, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        plain(text)
    }

    fn plugin_files(&self, program: &str) -> Vec<PluginFile> {
        vec![PluginFile {
            path: "cortex.js",
            text: script(Agent::Amp, "Amp", (GLUE, HOOKS), program),
        }]
    }
}
