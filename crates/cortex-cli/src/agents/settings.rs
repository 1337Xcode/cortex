//! Cortex's hooks in an agent's JSON hooks file, in the shape its agent reads. A [`Style`] says
//! where the events are (under a top-level key, at the top of the file, or, for an agent whose
//! hooks each name the event that triggers them, in one list), how their names are spelled, how an
//! event's list holds Cortex's hook (in a matcher group with its `hooks` list of handlers, as
//! Claude Code's hooks defined, or as a handler of its own), how the handler runs Cortex and what
//! it calls each part (see [`super::handler`]), and the `version` the file declares, if any.
//!
//! Cortex's own entries are found wherever they are (see [`super::handler::is_cortex`] and
//! [`super::entries`]), taken out, and the current ones put where the first of them was, keeping
//! the keys the user added to them (such as a hook's `enabled` switch); everything else, the
//! user's hooks and keys and their order, stays as it was. Merging again gives what merging once
//! gave. `cortex doctor` reads the same files through [`cortex_commands`], [`is_current`],
//! [`is_turned_off`] and [`flag`], and `cortex uninstall` takes Cortex's hooks out through
//! [`without_cortex_hooks`].

use anyhow::bail;
use cortex_hook::HookEntry;
use serde_json::{Map, Value};

use super::command::Shell;
use super::entries::{
    event_keys, item_of, items_in, take_cortex, take_cortex_hooks, with_users_keys,
};
use super::handler::{Keys, command_in, is_cortex};

/// How one agent's hooks file holds Cortex's hooks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Style {
    /// The agent's name on Cortex's command line.
    pub(crate) agent: &'static str,
    /// How the agent's handler runs Cortex.
    pub(crate) handler: Handler,
    /// The unit of the handler's timeout.
    pub(crate) timeout: TimeoutUnit,
    /// Where the events are.
    pub(crate) events: Events,
    /// How the events' names are spelled.
    pub(crate) spelling: Spelling,
    /// How an event's list holds a hook.
    pub(crate) layout: Layout,
    /// What the agent calls each part of a handler.
    pub(crate) keys: Keys,
    /// The `version` the file declares at its top, written when it declares none.
    pub(crate) version: Option<Version>,
}

/// How an agent's hook handler runs a program.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Handler {
    /// The program and an argument list, with no shell (Claude Code's exec form).
    Arguments,
    /// A command line, given to the shell the agent uses on this system.
    CommandLine(Shell),
}

/// The unit an agent's hook timeout is written in.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TimeoutUnit {
    /// Whole seconds.
    Seconds,
    /// Milliseconds.
    Milliseconds,
}

/// Where an agent's hooks file holds the events.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Events {
    /// In an object mapping each event to its list, under this top-level key: `hooks`, or the name
    /// of a hook of the agent's own (Antigravity's files map each hook's name to its events).
    Under(&'static str),
    /// In an object mapping each event to its list, which is the whole file.
    WholeFile,
    /// As [`Events::WholeFile`], or under `hooks` when the file already keeps its events there: an
    /// agent that reads both, and then ignores every other key beside `hooks` (OpenHands).
    WholeFileOrHooks,
    /// In one list under the top-level key `hooks`, each hook named and naming the event that
    /// triggers it (Kiro's).
    Triggers,
}

/// How the events' names are spelled in an agent's hooks file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spelling {
    /// As the agent's adapter names them (`SessionStart`).
    AsAdapter,
    /// As the user's own events in the file are spelled: in Pascal case (`SessionStart`) when one
    /// of them is, in snake case (`session_start`) otherwise. For an agent that reads both and
    /// refuses a file that names one event both ways (OpenHands).
    AsFile,
}

/// How an event's list holds a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    /// In a matcher group: an object with the matcher, if any, and a `hooks` list of handlers.
    Grouped,
    /// As a handler of its own, with its matcher on it.
    Flat,
    /// In a matcher group when it has a matcher (a tool's events), as a handler of its own
    /// otherwise (Antigravity's).
    GroupedWhenMatched,
}

/// The `version` a hooks file declares.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Version {
    /// A number, such as `1`.
    Number(u64),
    /// A text, such as `"v1"`.
    Text(&'static str),
}

impl Version {
    /// The version as JSON.
    fn value(self) -> Value {
        match self {
            Self::Number(number) => number.into(),
            Self::Text(text) => text.into(),
        }
    }
}

/// Where one hooks file, as it is, holds the events of a [`Style`].
#[derive(Debug, Clone, Copy)]
enum Place {
    /// In the object under this top-level key.
    Key(&'static str),
    /// At the top of the file.
    Whole,
    /// In the one list of named hooks under `hooks`.
    Triggers,
}

/// Where `settings` holds the events of `style`.
fn place_in(settings: &Map<String, Value>, style: &Style) -> Place {
    match style.events {
        Events::Under(key) => Place::Key(key),
        Events::WholeFile => Place::Whole,
        Events::WholeFileOrHooks if settings.get("hooks").is_some_and(Value::is_object) => {
            Place::Key("hooks")
        }
        Events::WholeFileOrHooks => Place::Whole,
        Events::Triggers => Place::Triggers,
    }
}

/// The hooks file `existing` (a file's text, or `None` when there is no file) with Cortex's hook
/// for each of `entries`, run by `program`, written in `style`, as text: pretty JSON with a final
/// line break.
///
/// # Errors
///
/// The text is not JSON, is not an object, or holds where the events go something of another
/// shape, or an event Cortex registers whose value is not a list: a file the agent itself would
/// refuse, which is left for the user to repair rather than overwritten. Or the program's path
/// needs quoting for a shell Cortex does not know.
pub(crate) fn merged(
    existing: Option<&str>,
    program: &str,
    entries: &[HookEntry],
    style: &Style,
) -> anyhow::Result<String> {
    let mut settings = parsed(existing)?;
    if let Some(version) = style.version {
        settings.entry("version").or_insert_with(|| version.value());
    }
    match place_in(&settings, style) {
        Place::Triggers => merge_triggers(&mut settings, entries, program, style)?,
        place => {
            let prefix = match place {
                Place::Key(key) => format!("{key}."),
                Place::Whole | Place::Triggers => String::new(),
            };
            let hooks = events_in(&mut settings, place)?;
            merge_events((hooks, &prefix), entries, program, style)?;
        }
    }
    text_of(settings)
}

/// Puts Cortex's hook for each of `entries` into the events `hooks`, as [`merged`] says; an error
/// names an event after `prefix`, where the events are (`hooks.`).
///
/// # Errors
///
/// As [`merged`].
fn merge_events(
    (hooks, prefix): (&mut Map<String, Value>, &str),
    entries: &[HookEntry],
    program: &str,
    style: &Style,
) -> anyhow::Result<()> {
    let keys = event_keys(hooks, entries, program, style);
    let taken = take_cortex_hooks(hooks, &keys, program, style);
    for (entry, key) in entries.iter().zip(&keys) {
        let found = taken.iter().find(|(event, _)| event == key);
        let item = with_users_keys(
            item_of(entry, program, style)?,
            found.and_then(|(_, taken)| taken.removed.first()),
        );
        match hooks.get_mut(key) {
            Some(Value::Array(items)) => {
                let place = found.map_or(items.len(), |(_, taken)| taken.place.min(items.len()));
                items.insert(place, item);
            }
            Some(_) => bail!("its `{prefix}{key}` is not a list"),
            None => {
                hooks.insert(key.clone(), Value::Array(vec![item]));
            }
        }
    }
    Ok(())
}

/// Puts Cortex's hook for each of `entries`, in their order, into the one list of hooks of
/// `settings`, where the first of Cortex's was, or at its end, each keeping the keys the user
/// added to Cortex's hook of the same trigger.
///
/// # Errors
///
/// As [`merged`]: a `hooks` that is not a list.
fn merge_triggers(
    settings: &mut Map<String, Value>,
    entries: &[HookEntry],
    program: &str,
    style: &Style,
) -> anyhow::Result<()> {
    let Value::Array(hooks) = settings
        .entry("hooks")
        .or_insert_with(|| Value::Array(Vec::new()))
    else {
        bail!("its `hooks` is not a list");
    };
    let taken = take_cortex(hooks, program, style);
    let place = taken.as_ref().map_or(hooks.len(), |taken| taken.place);
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let old = taken
            .iter()
            .flat_map(|taken| &taken.removed)
            .find(|old| old.get("trigger").and_then(Value::as_str) == Some(entry.event_name));
        items.push(with_users_keys(item_of(entry, program, style)?, old));
    }
    hooks.splice(place..place, items);
    Ok(())
}

/// The hooks file `existing` without Cortex's hooks, `program` being this binary, written in
/// `style`, as text in the form [`merged`] writes; `None` when it holds none. An event left with
/// no hook goes, and so does the object or list of events left empty, but for a file that is the
/// events itself; everything else stays, in its order, the `version` too.
///
/// # Errors
///
/// As [`merged`]: the text is not a JSON object.
pub(crate) fn without_cortex_hooks(
    existing: &str,
    program: &str,
    style: &Style,
) -> anyhow::Result<Option<String>> {
    let mut settings = parsed(Some(existing))?;
    let (key, is_empty) = match place_in(&settings, style) {
        Place::Key(key) => match settings.get_mut(key) {
            Some(Value::Object(hooks)) => {
                if take_cortex_hooks(hooks, &[], program, style).is_empty() {
                    return Ok(None);
                }
                (key, hooks.is_empty())
            }
            _ => return Ok(None),
        },
        Place::Whole => {
            if take_cortex_hooks(&mut settings, &[], program, style).is_empty() {
                return Ok(None);
            }
            return text_of(settings).map(Some);
        }
        Place::Triggers => match settings.get_mut("hooks") {
            Some(Value::Array(hooks)) => {
                if take_cortex(hooks, program, style).is_none() {
                    return Ok(None);
                }
                ("hooks", hooks.is_empty())
            }
            _ => return Ok(None),
        },
    };
    if is_empty {
        settings.shift_remove(key);
    }
    text_of(settings).map(Some)
}

/// `settings` as text: pretty JSON with a final line break.
///
/// # Errors
///
/// The JSON cannot be written, which a map of JSON values never fails at.
pub(super) fn text_of(settings: Map<String, Value>) -> anyhow::Result<String> {
    let mut text = serde_json::to_string_pretty(&Value::Object(settings))?;
    text.push('\n');
    Ok(text)
}

/// The map of events of `settings` at `place`, made under its key when missing.
///
/// # Errors
///
/// What is under the key is not a JSON object.
fn events_in(
    settings: &mut Map<String, Value>,
    place: Place,
) -> anyhow::Result<&mut Map<String, Value>> {
    let Place::Key(key) = place else {
        return Ok(settings);
    };
    match settings
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
    {
        Value::Object(hooks) => Ok(hooks),
        _ => bail!("its `{key}` is not a JSON object"),
    }
}

/// The lists of `settings` that hold its hooks, as `style` places them.
fn lists_in<'s>(settings: &'s Map<String, Value>, style: &Style) -> Vec<&'s Vec<Value>> {
    match place_in(settings, style) {
        Place::Key(key) => settings
            .get(key)
            .and_then(Value::as_object)
            .map(|hooks| hooks.values().filter_map(Value::as_array).collect())
            .unwrap_or_default(),
        Place::Whole => settings.values().filter_map(Value::as_array).collect(),
        Place::Triggers => settings
            .get("hooks")
            .and_then(Value::as_array)
            .into_iter()
            .collect(),
    }
}

/// The settings object `existing` holds; an empty one for no file or a file of only white space.
/// A byte order mark at its start is passed over, and not written back.
///
/// # Errors
///
/// The text is not JSON, which is said with where, never with the text itself, or is not an
/// object.
pub(super) fn parsed(existing: Option<&str>) -> anyhow::Result<Map<String, Value>> {
    let text = existing.map(|text| text.strip_prefix('\u{feff}').unwrap_or(text));
    let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
        return Ok(Map::new());
    };
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(settings)) => Ok(settings),
        Ok(_) => bail!("it is not a JSON object"),
        Err(error) => bail!(
            "it is not JSON (line {}, column {})",
            error.line(),
            error.column()
        ),
    }
}

/// The command each Cortex hook in the hooks file `existing` runs, `program` being this binary, in
/// the file's order: none when the file holds no Cortex hook.
///
/// # Errors
///
/// As [`merged`]: the text is not a JSON object.
pub(crate) fn cortex_commands(
    existing: &str,
    program: &str,
    style: &Style,
) -> anyhow::Result<Vec<String>> {
    let settings = parsed(Some(existing))?;
    Ok(items_in(lists_in(&settings, style), style)
        .into_iter()
        .filter(|item| is_cortex(item, program, style))
        .filter_map(|item| command_in(item, style))
        .map(str::to_owned)
        .collect())
}

/// Whether the user turned Cortex's hooks off in the hooks file `existing`, `program` being this
/// binary: `"enabled": false` on the object of events Cortex owns (Antigravity's hook name
/// `cortex`) or on one of Cortex's hooks. Init leaves such a switch as it is.
///
/// # Errors
///
/// As [`merged`]: the text is not a JSON object.
pub(crate) fn is_turned_off(existing: &str, program: &str, style: &Style) -> anyhow::Result<bool> {
    let settings = parsed(Some(existing))?;
    let is_off = |value: &Value| value.get("enabled").and_then(Value::as_bool) == Some(false);
    let owns_events = matches!(style.events, Events::Under(key) if key != "hooks");
    let events_off = match place_in(&settings, style) {
        Place::Key(key) if owns_events => settings.get(key).is_some_and(is_off),
        Place::Key(_) | Place::Whole | Place::Triggers => false,
    };
    Ok(events_off
        || items_in(lists_in(&settings, style), style)
            .into_iter()
            .any(|item| is_cortex(item, program, style) && is_off(item)))
}

/// Whether the hooks file `existing` holds Cortex's current hook for each of `entries`, run by
/// `program`, written in `style`: merging them in would change nothing but the file's layout of
/// white space and keys.
///
/// # Errors
///
/// As [`merged`]: the text is not JSON of the agent's shape, or the program's path cannot be
/// written for the agent's shell.
pub(crate) fn is_current(
    existing: &str,
    program: &str,
    entries: &[HookEntry],
    style: &Style,
) -> anyhow::Result<bool> {
    let expected = merged(Some(existing), program, entries, style)?;
    let expected: Value = serde_json::from_str(&expected)?;
    Ok(Value::Object(parsed(Some(existing))?) == expected)
}

/// What the top-level switch `key` of the settings `existing` says, when it is set to true or
/// false; `None` when it is not set, or set to something else, which the agent would not take as a
/// switch either.
///
/// # Errors
///
/// As [`merged`]: the text is not a JSON object.
pub(crate) fn flag(existing: &str, key: &str) -> anyhow::Result<Option<bool>> {
    Ok(parsed(Some(existing))?.get(key).and_then(Value::as_bool))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cortex_core::HookKind;

    use super::*;
    use crate::agents::claude::STYLE;
    use crate::agents::handler;

    /// [`super::merged`] in Claude Code's style.
    fn merged(
        existing: Option<&str>,
        program: &str,
        entries: &[HookEntry],
    ) -> anyhow::Result<String> {
        super::merged(existing, program, entries, &STYLE)
    }

    /// [`super::without_cortex_hooks`] in Claude Code's style.
    fn without_cortex_hooks(existing: &str, program: &str) -> anyhow::Result<Option<String>> {
        super::without_cortex_hooks(existing, program, &STYLE)
    }

    /// [`handler::is_cortex`] in Claude Code's style.
    fn is_cortex(item: &Value, program: &str) -> bool {
        handler::is_cortex(item, program, &STYLE)
    }

    /// [`super::cortex_commands`] in Claude Code's style.
    fn cortex_commands(existing: &str, program: &str) -> anyhow::Result<Vec<String>> {
        super::cortex_commands(existing, program, &STYLE)
    }

    /// The binary the tests register.
    const PROGRAM: &str = "/opt/cortex/bin/cortex";

    /// A hook with a matcher and a hard timeout of 300 ms: 1 s once rounded up.
    const PRE_TOOL_USE: HookEntry = HookEntry {
        event_name: "PreToolUse",
        matcher: Some("Read"),
        hook: HookKind::PreTool,
        hard_timeout: Duration::from_millis(300),
        gives_context: false,
    };

    /// A hook without a matcher and a hard timeout of 1,500 ms: 2 s once rounded up.
    const SESSION_START: HookEntry = HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    };

    /// No file: a settings object holding only the two hooks, in the table's order.
    #[test]
    fn no_file_gets_cortex_s_hooks() -> anyhow::Result<()> {
        let expected = r#"{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Read",
        "hooks": [
          {
            "type": "command",
            "command": "/opt/cortex/bin/cortex",
            "args": [
              "hook",
              "--agent",
              "claude",
              "--event",
              "PreToolUse"
            ],
            "timeout": 1,
            "onFailure": "continue"
          }
        ]
      }
    ],
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/opt/cortex/bin/cortex",
            "args": [
              "hook",
              "--agent",
              "claude",
              "--event",
              "SessionStart"
            ],
            "timeout": 2,
            "onFailure": "continue"
          }
        ]
      }
    ]
  }
}
"#;
        let entries = [PRE_TOOL_USE, SESSION_START];
        assert_eq!(merged(None, PROGRAM, &entries)?, expected);
        assert_eq!(merged(Some(" \n"), PROGRAM, &entries)?, expected);
        Ok(())
    }

    /// The user's keys before and after `hooks` and their own hooks, on Cortex's event and on
    /// another, stay as they were and in their order; Cortex's group comes after the user's.
    #[test]
    fn the_user_s_settings_and_hooks_stay() -> anyhow::Result<()> {
        let existing = r#"{
  "model": "opus",
  "hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": "echo started"}]}],
    "PostToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "check.sh"}]}]
  },
  "permissions": {"allow": ["Read"]}
}"#;
        let expected = r#"{
  "model": "opus",
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo started"
          }
        ]
      },
      {
        "hooks": [
          {
            "type": "command",
            "command": "/opt/cortex/bin/cortex",
            "args": [
              "hook",
              "--agent",
              "claude",
              "--event",
              "SessionStart"
            ],
            "timeout": 2,
            "onFailure": "continue"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "check.sh"
          }
        ]
      }
    ]
  },
  "permissions": {
    "allow": [
      "Read"
    ]
  }
}
"#;
        assert_eq!(merged(Some(existing), PROGRAM, &[SESSION_START])?, expected);
        Ok(())
    }

    /// An older Cortex hook (another folder, another timeout, a capitalised `.EXE`) ahead of the
    /// user's hook is replaced where it was; one sharing a group with a user's hook is taken out of
    /// the group, which stays, Cortex's own group going where it was; one for an event Cortex no
    /// longer registers goes, with the event it alone held.
    #[test]
    fn older_cortex_hooks_are_replaced_in_place() -> anyhow::Result<()> {
        let existing = r#"{"hooks": {
  "Notification": [{"hooks": [{"type": "command", "command": "C:\\old\\cortex.exe",
    "args": ["hook", "--agent", "claude", "--event", "Notification"]}]}],
  "SessionStart": [
    {"hooks": [{"type": "command", "command": "C:\\old\\Cortex.EXE",
      "args": ["hook", "--agent", "claude", "--event", "SessionStart"], "timeout": 9}]},
    {"hooks": [{"type": "command", "command": "echo started"}]}
  ],
  "PreToolUse": [
    {"matcher": "Read", "hooks": [{"type": "command", "command": "mine.sh"},
      {"type": "command", "command": "/usr/bin/cortex",
        "args": ["hook", "--agent", "claude", "--event", "PreToolUse"]}]}
  ]
}}"#;
        let expected = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/opt/cortex/bin/cortex",
            "args": [
              "hook",
              "--agent",
              "claude",
              "--event",
              "SessionStart"
            ],
            "timeout": 2,
            "onFailure": "continue"
          }
        ]
      },
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo started"
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Read",
        "hooks": [
          {
            "type": "command",
            "command": "/opt/cortex/bin/cortex",
            "args": [
              "hook",
              "--agent",
              "claude",
              "--event",
              "PreToolUse"
            ],
            "timeout": 1,
            "onFailure": "continue"
          }
        ]
      },
      {
        "matcher": "Read",
        "hooks": [
          {
            "type": "command",
            "command": "mine.sh"
          }
        ]
      }
    ]
  }
}
"#;
        let entries = [SESSION_START, PRE_TOOL_USE];
        assert_eq!(merged(Some(existing), PROGRAM, &entries)?, expected);
        Ok(())
    }

    /// A registered event that held only Cortex's older hook keeps its place among the events: the
    /// user's event after it stays after it.
    #[test]
    fn an_event_keeps_its_place_when_its_hook_is_replaced() -> anyhow::Result<()> {
        let existing = r#"{"hooks": {
  "SessionStart": [{"hooks": [{"type": "command", "command": "/old/cortex",
    "args": ["hook", "--agent", "claude", "--event", "SessionStart"]}]}],
  "Notification": [{"hooks": [{"type": "command", "command": "bell.sh"}]}]
}}"#;
        let value: Value =
            serde_json::from_str(&merged(Some(existing), PROGRAM, &[SESSION_START])?)?;
        let events: Vec<&str> = value
            .get("hooks")
            .and_then(Value::as_object)
            .map(|hooks| hooks.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(events, ["SessionStart", "Notification"]);
        Ok(())
    }

    /// Merging what a merge wrote gives it back unchanged: no hook is written twice. With one of
    /// Cortex's hooks missing (as if taken out by hand), a merge adds it back.
    #[test]
    fn merging_again_changes_nothing_and_repairs_drift() -> anyhow::Result<()> {
        let entries = [PRE_TOOL_USE, SESSION_START];
        let once = merged(Some(r#"{"model": "opus"}"#), PROGRAM, &entries)?;
        assert_eq!(merged(Some(&once), PROGRAM, &entries)?, once);
        let one_missing = merged(Some(r#"{"model": "opus"}"#), PROGRAM, &[PRE_TOOL_USE])?;
        assert_eq!(merged(Some(&one_missing), PROGRAM, &entries)?, once);
        Ok(())
    }

    /// A hook that only looks like Cortex's is the user's: a `cortex` command with other arguments,
    /// one in shell form, a program merely named like it, and Cortex for another agent. Cortex's in
    /// any folder and any case of `.exe` is Cortex's.
    #[test]
    fn only_cortex_s_own_hooks_are_taken() -> anyhow::Result<()> {
        let others = [
            r#"{"command": "/bin/cortex", "args": ["status"]}"#,
            r#"{"command": "cortex hook --agent claude --event Stop"}"#,
            r#"{"command": "/bin/cortex-helper", "args": ["hook", "--agent", "claude"]}"#,
            r#"{"command": "/bin/cortex", "args": ["hook", "--agent", "codex"]}"#,
        ];
        for hook in others {
            assert!(!is_cortex(&serde_json::from_str(hook)?, PROGRAM), "{hook}");
        }
        let ours = r#"{"command": "D:/tools/CORTEX.exe",
            "args": ["hook", "--agent", "claude", "--event", "Stop"]}"#;
        assert!(is_cortex(&serde_json::from_str(ours)?, PROGRAM));
        let renamed = r#"{"command": "/opt/bin/cortex-x86_64-linux",
            "args": ["hook", "--agent", "claude", "--event", "Stop"]}"#;
        assert!(!is_cortex(&serde_json::from_str(renamed)?, PROGRAM));
        assert!(is_cortex(
            &serde_json::from_str(renamed)?,
            "/opt/bin/cortex-x86_64-linux"
        ));
        Ok(())
    }

    /// A binary installed under another name finds the hooks it wrote: merging again with it
    /// changes nothing. A byte order mark before the JSON is passed over and not written back.
    #[test]
    fn a_renamed_binary_and_a_byte_order_mark() -> anyhow::Result<()> {
        let renamed = "/opt/bin/cortex-x86_64-linux";
        let once = merged(None, renamed, &[SESSION_START])?;
        assert_eq!(merged(Some(&once), renamed, &[SESSION_START])?, once);
        let with_mark = format!("\u{feff}{once}");
        assert_eq!(merged(Some(&with_mark), renamed, &[SESSION_START])?, once);
        Ok(())
    }

    /// The commands of Cortex's hooks are read back in the file's order, the user's left out; a file
    /// with no `hooks` has none. A switch reads as set only when it is true or false.
    #[test]
    fn doctor_reads_commands_and_switches() -> anyhow::Result<()> {
        let written = merged(
            Some(
                r#"{"disableAllHooks": true, "hooks": {"Stop": [{"hooks": [{"command": "x.sh"}]}]}}"#,
            ),
            PROGRAM,
            &[PRE_TOOL_USE, SESSION_START],
        )?;
        assert_eq!(cortex_commands(&written, PROGRAM)?, [PROGRAM, PROGRAM]);
        assert_eq!(cortex_commands("{}", PROGRAM)?, Vec::<String>::new());
        assert_eq!(flag(&written, "disableAllHooks")?, Some(true));
        assert_eq!(
            flag(r#"{"disableAllHooks": "yes"}"#, "disableAllHooks")?,
            None
        );
        assert_eq!(flag("{}", "disableAllHooks")?, None);
        assert!(flag("[]", "disableAllHooks").is_err());
        Ok(())
    }

    /// Taking Cortex's hooks out of settings they were merged into gives back the user's: keys
    /// and hooks in their order, the event that held only Cortex's group gone, the user's event
    /// keeping the user's group. Settings that held only Cortex's hooks lose `hooks` itself;
    /// settings that hold none give nothing to write, and a file that is not JSON is refused.
    #[test]
    fn cortex_s_hooks_come_out() -> anyhow::Result<()> {
        let users = r#"{"model": "opus", "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo started"}]}]}, "permissions": {"allow": ["Read"]}}"#;
        let written = merged(Some(users), PROGRAM, &[PRE_TOOL_USE, SESSION_START])?;
        let expected = r#"{
  "model": "opus",
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo started"
          }
        ]
      }
    ]
  },
  "permissions": {
    "allow": [
      "Read"
    ]
  }
}
"#;
        assert_eq!(
            without_cortex_hooks(&written, PROGRAM)?.as_deref(),
            Some(expected)
        );
        let only_cortex = merged(None, PROGRAM, &[PRE_TOOL_USE])?;
        assert_eq!(
            without_cortex_hooks(&only_cortex, PROGRAM)?.as_deref(),
            Some("{}\n")
        );
        let between = r#"{"model": "opus", "hooks": {}, "theme": "dark", "verbose": true}"#;
        let written = merged(Some(between), PROGRAM, &[PRE_TOOL_USE])?;
        assert_eq!(
            without_cortex_hooks(&written, PROGRAM)?.as_deref(),
            Some("{\n  \"model\": \"opus\",\n  \"theme\": \"dark\",\n  \"verbose\": true\n}\n")
        );
        assert_eq!(without_cortex_hooks(users, PROGRAM)?, None);
        assert!(without_cortex_hooks("{\"hooks\": oops", PROGRAM).is_err());
        Ok(())
    }

    /// A file that is not JSON, or not an object, or whose `hooks` or a registered event's value has
    /// another shape, is refused, and the error says where, never what the file holds.
    #[test]
    fn a_file_of_another_shape_is_refused() {
        let cases = [
            ("{\"secret\": tru", "it is not JSON (line 1, column 14)"),
            ("[1, 2]", "it is not a JSON object"),
            ("{\"hooks\": []}", "its `hooks` is not a JSON object"),
            (
                "{\"hooks\": {\"SessionStart\": {}}}",
                "its `hooks.SessionStart` is not a list",
            ),
        ];
        for (existing, expected) in cases {
            let error = merged(Some(existing), PROGRAM, &[SESSION_START])
                .err()
                .map(|error| error.to_string());
            assert_eq!(error.as_deref(), Some(expected), "{existing}");
        }
    }

    /// A command-line style, as Codex's on a POSIX system, by hand: the events under `hooks`, each
    /// handler a command line and a timeout in seconds; merging again changes nothing; a user's
    /// hook stays; taking Cortex's hooks out gives the user's file back, and an older Cortex line
    /// (another program path) is found as Cortex's. In milliseconds at the top of the file, as
    /// Droid's and Gemini's units are, the same hook is the whole object with 300 for its timeout.
    #[test]
    fn a_command_line_style_merges_and_takes_out() -> anyhow::Result<()> {
        let style = Style {
            agent: "codex",
            handler: Handler::CommandLine(Shell::Posix),
            timeout: TimeoutUnit::Seconds,
            events: Events::Under("hooks"),
            spelling: Spelling::AsAdapter,
            layout: Layout::Grouped,
            keys: Keys::CLAUDE,
            version: None,
        };
        let user = r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "mine.sh"}]}]}}"#;
        let merged = super::merged(Some(user), PROGRAM, &[PRE_TOOL_USE], &style)?;
        let value: Value = serde_json::from_str(&merged)?;
        assert_eq!(
            value,
            serde_json::json!({"hooks": {"PreToolUse": [
                {"hooks": [{"type": "command", "command": "mine.sh"}]},
                {"matcher": "Read", "hooks": [{"type": "command",
                    "command": "/opt/cortex/bin/cortex hook --agent codex --event PreToolUse",
                    "timeout": 1}]}
            ]}})
        );
        assert_eq!(
            super::merged(Some(&merged), PROGRAM, &[PRE_TOOL_USE], &style)?,
            merged
        );
        let older = merged.replace("/opt/cortex/bin/cortex", "/usr/old/cortex");
        let without = super::without_cortex_hooks(&older, PROGRAM, &style)?;
        let without: Value = serde_json::from_str(without.as_deref().unwrap_or("null"))?;
        assert_eq!(without, serde_json::from_str::<Value>(user)?);
        let whole = Style {
            timeout: TimeoutUnit::Milliseconds,
            events: Events::WholeFile,
            ..style
        };
        let value: Value =
            serde_json::from_str(&super::merged(None, PROGRAM, &[PRE_TOOL_USE], &whole)?)?;
        assert_eq!(
            value,
            serde_json::json!({"PreToolUse": [{"matcher": "Read", "hooks": [{"type": "command",
                "command": "/opt/cortex/bin/cortex hook --agent codex --event PreToolUse",
                "timeout": 300}]}]})
        );
        Ok(())
    }
}
