//! One handler of Cortex's in an agent's hooks file: the object that runs this program at one
//! event, made from the event's [`HookEntry`], and how a handler already in a file is known as
//! Cortex's. Run as an argument list (Claude Code's exec form) it is the absolute path of this
//! binary with `hook --agent <agent> --event <event>` as its arguments and `onFailure: "continue"`;
//! run as a command line it is the line [`hook_command`] writes for the agent's shell. Its timeout
//! is the event's hard timeout rounded up to the agent's unit. [`Keys`] says what the agent calls
//! each part.

use anyhow::Context as _;
use cortex_hook::HookEntry;
use serde_json::{Map, Value};

use super::command::hook_command;
use super::settings::{Events, Handler, Style, TimeoutUnit};

/// What an agent calls the parts of a handler.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Keys {
    /// The key of the program or command line: `command`, or the shell's own (`bash`,
    /// `powershell`) for an agent that takes one line per shell.
    pub(crate) command: &'static str,
    /// The key of the timeout: `timeout`, or `timeoutSec`.
    pub(crate) timeout: &'static str,
    /// Whether the handler says `"type": "command"`: an agent that documents no `type` gets none.
    pub(crate) is_typed: bool,
}

impl Keys {
    /// Claude Code's keys, which most agents took: `type`, `command` and `timeout`.
    pub(crate) const CLAUDE: Self = Self {
        command: "command",
        timeout: "timeout",
        is_typed: true,
    };
}

/// The handler running `program` at `entry`'s event, written in `style`, its matcher on it when
/// `matcher` (an agent whose handlers sit straight in an event's list).
///
/// # Errors
///
/// The program's path needs quoting for a shell Cortex does not know.
pub(super) fn handler_of(
    entry: &HookEntry,
    program: &str,
    style: &Style,
    matcher: Option<&str>,
) -> anyhow::Result<Map<String, Value>> {
    let keys = style.keys;
    let mut handler = Map::new();
    if keys.is_typed {
        handler.insert("type".to_owned(), "command".into());
    }
    if let Some(matcher) = matcher {
        handler.insert("matcher".to_owned(), matcher.into());
    }
    match style.handler {
        Handler::Arguments => {
            let args = ["hook", "--agent", style.agent, "--event", entry.event_name]
                .into_iter()
                .map(Value::from)
                .collect();
            handler.insert(keys.command.to_owned(), program.into());
            handler.insert("args".to_owned(), Value::Array(args));
        }
        Handler::CommandLine(shell) => {
            let line =
                hook_command(program, style.agent, entry.event_name, shell).with_context(|| {
                    format!(
                        "the path {program} needs quoting, and how {} quotes it on this system \
                         is not documented",
                        style.agent
                    )
                })?;
            handler.insert(keys.command.to_owned(), line.into());
        }
    }
    handler.insert(keys.timeout.to_owned(), timeout_of(entry, style).into());
    if let Handler::Arguments = style.handler {
        handler.insert("onFailure".to_owned(), "continue".into());
    }
    Ok(handler)
}

/// `entry`'s hard timeout in `style`'s unit, rounded up, at least 1.
pub(super) fn timeout_of(entry: &HookEntry, style: &Style) -> u64 {
    let millis = u64::try_from(entry.hard_timeout.as_millis()).unwrap_or(u64::MAX);
    match style.timeout {
        TimeoutUnit::Seconds => millis.div_ceil(1_000).max(1),
        TimeoutUnit::Milliseconds => millis.max(1),
    }
}

/// The program or command line `item` runs, as `style` places it: in the handler's command key,
/// or, for an agent whose hooks each name their trigger, in the hook's `action`.
pub(super) fn command_in<'v>(item: &'v Value, style: &Style) -> Option<&'v str> {
    let holder = match style.events {
        Events::Triggers => item.get("action")?,
        Events::Under(_) | Events::WholeFile | Events::WholeFileOrHooks => item,
    };
    holder.get(style.keys.command).and_then(Value::as_str)
}

/// Whether `item` is a Cortex hook of `style`'s agent. Run as an argument list: a command whose
/// arguments begin `hook --agent <agent>` and whose file name is `cortex` (`cortex.exe` on
/// Windows), whatever folder it is in, or which is `program`, this binary, whatever it is named,
/// so that a binary installed under another name finds its own hooks again. Run as a command line:
/// one that runs ` hook --agent <agent> --event `.
pub(super) fn is_cortex(item: &Value, program: &str, style: &Style) -> bool {
    let Some(command) = command_in(item, style) else {
        return false;
    };
    if let Handler::CommandLine(_) = style.handler {
        return command.contains(&format!(" hook --agent {} --event ", style.agent));
    }
    let name = command.rsplit(['/', '\\']).next().unwrap_or(command);
    let stem = name
        .rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("exe"))
        .map_or(name, |(stem, _)| stem);
    let prefix = ["hook", "--agent", style.agent];
    let args = item.get("args").and_then(Value::as_array);
    let begins_as_cortex = args.is_some_and(|args| {
        args.len() >= prefix.len()
            && args
                .iter()
                .zip(prefix)
                .all(|(arg, expected)| arg.as_str() == Some(expected))
    });
    (stem.eq_ignore_ascii_case("cortex") || command == program) && begins_as_cortex
}
