//! The OpenClaw adapter, built from OpenClaw's plugin hooks documentation (read 2026-11-06;
//! v2026.11.1 the latest release) and not yet run live. OpenClaw's hooks are a native plugin's
//! handlers, so Cortex writes a plugin (`plugins/openclaw.js`, with its package and manifest) that
//! sends each event in Claude Code's hook JSON, under the hook's name, and gives OpenClaw the text
//! back with a prompt: a session's primer at the end of the system prompt on its first prompt, the
//! work order after the prompt. Its hooks around a tool only observe. The working folder is the
//! agent workspace named by `OPENCLAW_WORKSPACE_DIR`; with it unset, no event is made. Its tools'
//! inputs are not documented; a call's result is its `error`, else its `result`.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::plugin::{PluginFile, plain, script};
use crate::shaped::normalize;
use crate::shaped::table::{Outcome, PromptField, ToolTable};

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// The glue wiring the plugin's core to OpenClaw's plugin API.
const GLUE: &str = include_str!("../plugins/openclaw.js");

/// The plugin's package, naming its entry file.
const PACKAGE: &str = r#"{
  "name": "cortex",
  "version": "1.0.0",
  "type": "module",
  "openclaw": { "extensions": ["./index.js"] }
}
"#;

/// The plugin's manifest: loaded at the Gateway's start, with no settings of its own.
const MANIFEST: &str = r#"{
  "id": "cortex",
  "name": "Cortex",
  "activation": { "onStartup": true },
  "configSchema": { "type": "object", "additionalProperties": false }
}
"#;

/// OpenClaw's tools Cortex reads, by their ids.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read", ToolKind::Read),
        ("apply_patch", ToolKind::ApplyPatch),
        ("exec", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::ResponseSuccess,
    prompt: PromptField::Prompt,
    folder_variable: Some("OPENCLAW_WORKSPACE_DIR"),
};

/// The events the plugin sends, by OpenClaw's hooks' names.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "session_start",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "before_prompt_build",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: true,
    },
    HookEntry {
        event_name: "after_tool_call",
        matcher: None,
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: false,
    },
    HookEntry {
        event_name: "before_compaction",
        matcher: None,
        hook: HookKind::Compact,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "agent_end",
        matcher: None,
        hook: HookKind::Stop,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "session_end",
        matcher: None,
        hook: HookKind::SessionEnd,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
];

/// OpenClaw.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenClaw;

impl Adapter for OpenClaw {
    fn agent(&self) -> Agent {
        Agent::OpenClaw
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
        normalize::event((Agent::OpenClaw, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        plain(text)
    }

    fn plugin_files(&self, program: &str) -> Vec<PluginFile> {
        vec![
            PluginFile {
                path: "cortex/index.js",
                text: script(Agent::OpenClaw, "OpenClaw", (GLUE, HOOKS), program),
            },
            PluginFile {
                path: "cortex/package.json",
                text: PACKAGE.to_owned(),
            },
            PluginFile {
                path: "cortex/openclaw.plugin.json",
                text: MANIFEST.to_owned(),
            },
        ]
    }
}
