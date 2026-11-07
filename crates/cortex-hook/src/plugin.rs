//! The plugins Cortex writes for the agents whose hooks are code a plugin registers, not commands
//! (OpenCode, Kilo Code, Amp, OpenClaw): one JavaScript core, which runs `cortex hook` with an event
//! in the JSON Claude Code's hooks use and keeps text for a later event, and each agent's glue,
//! which wires the core to the agent's plugin API. A plugin never throws, blocks or changes a call:
//! its failures give nothing, as the hook's do.

use std::time::Duration;

use cortex_core::Agent;
use serde_json::Value;

use crate::adapter::HookEntry;

/// The core every plugin starts with.
const CORE: &str = include_str!("../plugins/core.js");

/// How much longer than an event's hard timeout a plugin waits for `cortex hook` before stopping
/// it: the hook ends itself by its hard timeout, so the plugin stops only one that hung, after
/// the time a process takes to start.
const KILL_MARGIN: Duration = Duration::from_millis(100);

/// The most characters a plugin agent is given at once. None of their documentation names a
/// limit; Claude Code's is taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// A file of a plugin: where it goes in the agent's plugin folder, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginFile {
    /// Its path in the agent's plugin folder, with `/` between folders.
    pub path: &'static str,
    /// Its text.
    pub text: String,
}

/// A plugin's script for `agent`, titled `title`: the core, then the agent's `glue`, running
/// `program` and stopping a run of an event of `hooks` that outlasts its hard timeout by
/// [`KILL_MARGIN`].
pub(crate) fn script(
    agent: Agent,
    title: &str,
    (glue, hooks): (&str, &[HookEntry]),
    program: &str,
) -> String {
    // Written by hand, not from a JSON map, so that the events keep the hooks' order in every
    // build.
    let kill_after: Vec<String> = hooks
        .iter()
        .map(|entry| {
            let after = (entry.hard_timeout + KILL_MARGIN).as_millis();
            format!("{}: {after}", Value::from(entry.event_name))
        })
        .collect();
    // The program goes in last: a path holding a placeholder's name is not replaced again.
    let core = CORE
        .replace("__TITLE__", title)
        .replace("__AGENT__", &Value::from(agent.name()).to_string())
        .replace(
            "__KILL_AFTER_MS__",
            &format!("{{ {} }}", kill_after.join(", ")),
        )
        .replace("__PROGRAM__", &Value::from(program).to_string());
    format!("{core}{glue}")
}

/// `text` as a plugin takes it: plain, without its last line break, or nothing past the agents'
/// limit.
pub(crate) fn plain(text: &str) -> Option<String> {
    // INVARIANT: budgeted. A text past the agent's limit is not given in part.
    if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
        return None;
    }
    Some(text.trim_end().to_owned())
}
