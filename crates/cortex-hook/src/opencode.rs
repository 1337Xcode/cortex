//! The OpenCode and Kilo Code adapters, built from their plugin documentation (read 2026-11-06)
//! and not yet run live. Their hooks are a plugin's code, so Cortex writes a plugin
//! (`plugins/opencode.js`, for OpenCode 1.x's plugin API, which Kilo Code shares) that sends each
//! event in Claude Code's hook JSON, under the plugin hook's name, and gives the agent the text
//! back: a session's primer and the latest prompt's work order in the system prompt of its
//! requests, a read's hint and a tool's warnings after the tool's output. Neither agent has an
//! event at a session's start; the plugin sends one at the first prompt it sees in a session. A
//! read names its file in `filePath`; the other tools' inputs are not documented, and a call's
//! result is not given. OpenCode 2's plugin API is another and is not covered.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::plugin::{PluginFile, plain, script};
use crate::shaped::normalize;
use crate::shaped::table::{Outcome, PromptField, ToolTable};

/// The glue wiring the plugin's core to the agents' plugin API.
const GLUE: &str = include_str!("../plugins/opencode.js");

/// The agents' tools Cortex reads, by their names; a read's file moved to Claude Code's place.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read", ToolKind::Read),
        ("edit", ToolKind::Edit),
        ("write", ToolKind::Write),
        ("apply_patch", ToolKind::ApplyPatch),
        ("grep", ToolKind::Grep),
        ("glob", ToolKind::Glob),
        ("bash", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[("/tool_input/filePath", "/tool_input/file_path")],
    outcome: Outcome::Unknown,
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// The events the plugin sends, by its hooks' names (`session.start` the plugin's own).
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "session.start",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "chat.message",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: true,
    },
    HookEntry {
        event_name: "tool.execute.before",
        matcher: None,
        hook: HookKind::PreTool,
        hard_timeout: Duration::from_millis(300),
        gives_context: true,
    },
    HookEntry {
        event_name: "tool.execute.after",
        matcher: None,
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "experimental.session.compacting",
        matcher: None,
        hook: HookKind::Compact,
        hard_timeout: Duration::from_millis(300),
        gives_context: false,
    },
];

/// OpenCode.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Opencode;

/// Kilo Code.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Kilo;

/// `agent`'s event from `payload`, sent by the plugin at `event_name`.
fn normalized(
    agent: Agent,
    (event_name, payload): (&str, &[u8]),
    locate: &dyn Fn(&Path) -> Option<Located>,
    stamp: Stamp,
) -> Result<Option<Event>, NormalizeError> {
    let Some(entry) = HOOKS.iter().find(|entry| entry.event_name == event_name) else {
        return Ok(None);
    };
    normalize::event((agent, &TOOLS, entry), payload, locate, stamp)
}

impl Adapter for Opencode {
    fn agent(&self) -> Agent {
        Agent::Opencode
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
        normalized(Agent::Opencode, (event_name, payload), locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        plain(text)
    }

    fn plugin_files(&self, program: &str) -> Vec<PluginFile> {
        let body = script(Agent::Opencode, "OpenCode", (GLUE, HOOKS), program);
        vec![PluginFile {
            path: "cortex.js",
            text: format!("{body}\nexport const CortexPlugin = server;\n"),
        }]
    }
}

impl Adapter for Kilo {
    fn agent(&self) -> Agent {
        Agent::Kilo
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
        normalized(Agent::Kilo, (event_name, payload), locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        plain(text)
    }

    fn plugin_files(&self, program: &str) -> Vec<PluginFile> {
        let body = script(Agent::Kilo, "Kilo Code", (GLUE, HOOKS), program);
        // Kilo Code takes a local plugin file's plugin from its default export, with an id.
        vec![PluginFile {
            path: "cortex.js",
            text: format!("{body}\nexport default {{ id: \"cortex\", server }};\n"),
        }]
    }
}
