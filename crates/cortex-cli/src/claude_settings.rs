//! Cortex's hooks in a Claude Code settings file: the entries the Claude Code adapter's event table
//! calls for, merged into what the file holds.
//!
//! Each entry is an exec-form command hook (the absolute path of this binary, and `hook --agent
//! claude --event <event>` as its arguments, so no shell is involved), with the event's matcher, a
//! timeout of the event's hard timeout in whole seconds, rounded up, and `onFailure: "continue"`,
//! so that a hook that fails never holds up the agent. Cortex's own entries are found wherever they
//! are, by arguments that begin `hook --agent claude` given to a command whose file name is
//! `cortex` or which is this binary, taken out, and the current ones put where the first of them
//! was; everything else, the user's hooks and keys and their order, stays as it was. Merging again
//! gives what merging once gave. `cortex doctor` reads the same files through [`cortex_commands`]
//! and [`flag`], and `cortex uninstall` takes Cortex's hooks out through
//! [`without_cortex_hooks`].

use anyhow::bail;
use cortex_hook::HookEntry;
use serde_json::{Map, Value};

/// The arguments every Cortex hook command begins with.
const ARGS_PREFIX: [&str; 3] = ["hook", "--agent", "claude"];

/// The settings in `existing` (a file's text, or `None` when there is no file) with Cortex's hook
/// for each of `entries`, run by `program`, as text: pretty JSON with a final line break.
///
/// # Errors
///
/// The text is not JSON, is not an object, or holds a `hooks` that is not an object, or an event
/// Cortex registers whose value is not a list: a file Claude Code itself would refuse, which is
/// left for the user to repair rather than overwritten.
pub(crate) fn merged(
    existing: Option<&str>,
    program: &str,
    entries: &[HookEntry],
) -> anyhow::Result<String> {
    let mut settings = parsed(existing)?;
    let hooks = settings
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(hooks) = hooks else {
        bail!("its `hooks` is not a JSON object");
    };
    let places = take_cortex_hooks(hooks, entries, program);
    for entry in entries {
        let group = group_of(entry, program);
        match hooks.get_mut(entry.event_name) {
            Some(Value::Array(groups)) => {
                let place = places
                    .iter()
                    .find(|(event, _)| event == entry.event_name)
                    .map_or(groups.len(), |(_, place)| (*place).min(groups.len()));
                groups.insert(place, group);
            }
            Some(_) => bail!("its `hooks.{}` is not a list", entry.event_name),
            None => {
                hooks.insert(entry.event_name.to_owned(), Value::Array(vec![group]));
            }
        }
    }
    let mut text = serde_json::to_string_pretty(&Value::Object(settings))?;
    text.push('\n');
    Ok(text)
}

/// The settings `existing` without Cortex's hooks, `program` being this binary, as text in the form
/// [`merged`] writes; `None` when it holds none. An event left with no group goes, and so does a
/// `hooks` left empty; everything else stays, in its order.
///
/// # Errors
///
/// As [`merged`]: the text is not a JSON object.
pub(crate) fn without_cortex_hooks(
    existing: &str,
    program: &str,
) -> anyhow::Result<Option<String>> {
    let mut settings = parsed(Some(existing))?;
    let Some(Value::Object(hooks)) = settings.get_mut("hooks") else {
        return Ok(None);
    };
    if take_cortex_hooks(hooks, &[], program).is_empty() {
        return Ok(None);
    }
    if hooks.is_empty() {
        settings.shift_remove("hooks");
    }
    let mut text = serde_json::to_string_pretty(&Value::Object(settings))?;
    text.push('\n');
    Ok(Some(text))
}

/// The settings object `existing` holds; an empty one for no file or a file of only white space.
/// A byte order mark at its start is passed over, and not written back.
///
/// # Errors
///
/// The text is not JSON, which is said with where, never with the text itself, or is not an
/// object.
fn parsed(existing: Option<&str>) -> anyhow::Result<Map<String, Value>> {
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

/// Takes every Cortex hook out of `hooks`, `program` being this binary, and returns, for each event
/// that held one, where its first group holding one was among the groups kept. An event Cortex no
/// longer registers whose every group was Cortex's goes; one of `entries` keeps its place among
/// the events, emptied or not.
fn take_cortex_hooks(
    hooks: &mut Map<String, Value>,
    entries: &[HookEntry],
    program: &str,
) -> Vec<(String, usize)> {
    let mut places = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        if let Value::Array(groups) = groups
            && let Some(place) = take_cortex(groups, program)
        {
            places.push((event.clone(), place));
        }
    }
    hooks.retain(|event, groups| {
        let was_emptied = places.iter().any(|(emptied, _)| emptied == event)
            && groups.as_array().is_some_and(Vec::is_empty);
        !was_emptied || entries.iter().any(|entry| entry.event_name == event)
    });
    places
}

/// Takes every Cortex hook out of the matcher groups of one event, `program` being this binary,
/// dropping each group left with no hook whose hooks were all Cortex's, and returns where the first
/// group holding one was, among the groups kept. A group or a list of hooks of another shape is left
/// alone.
fn take_cortex(groups: &mut Vec<Value>, program: &str) -> Option<usize> {
    let mut place = None;
    let mut kept = Vec::with_capacity(groups.len());
    for mut group in groups.drain(..) {
        let handlers = group
            .as_object_mut()
            .and_then(|group| group.get_mut("hooks"))
            .and_then(Value::as_array_mut);
        let Some(handlers) = handlers else {
            kept.push(group);
            continue;
        };
        let before = handlers.len();
        handlers.retain(|handler| !is_cortex(handler, program));
        if handlers.len() == before {
            kept.push(group);
            continue;
        }
        place.get_or_insert(kept.len());
        if !handlers.is_empty() {
            kept.push(group);
        }
    }
    *groups = kept;
    place
}

/// Whether `handler` is a Cortex hook: a command run with arguments beginning `hook --agent claude`
/// whose file name is `cortex` (`cortex.exe` on Windows), whatever folder it is in, or which is
/// `program`, this binary, whatever it is named: a binary installed under another name finds its
/// own hooks again.
fn is_cortex(handler: &Value, program: &str) -> bool {
    let Some(command) = handler.get("command").and_then(Value::as_str) else {
        return false;
    };
    let name = command.rsplit(['/', '\\']).next().unwrap_or(command);
    let stem = name
        .rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("exe"))
        .map_or(name, |(stem, _)| stem);
    let args = handler.get("args").and_then(Value::as_array);
    let begins_as_cortex = args.is_some_and(|args| {
        args.len() >= ARGS_PREFIX.len()
            && args
                .iter()
                .zip(ARGS_PREFIX)
                .all(|(arg, expected)| arg.as_str() == Some(expected))
    });
    (stem.eq_ignore_ascii_case("cortex") || command == program) && begins_as_cortex
}

/// The command each Cortex hook in the settings `existing` runs, `program` being this binary, in
/// the file's order: none when the file holds no Cortex hook.
///
/// # Errors
///
/// As [`merged`]: the text is not a JSON object.
pub(crate) fn cortex_commands(existing: &str, program: &str) -> anyhow::Result<Vec<String>> {
    let settings = parsed(Some(existing))?;
    let Some(Value::Object(hooks)) = settings.get("hooks") else {
        return Ok(Vec::new());
    };
    let handlers = hooks
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten();
    Ok(handlers
        .filter(|handler| is_cortex(handler, program))
        .filter_map(|handler| handler.get("command").and_then(Value::as_str))
        .map(str::to_owned)
        .collect())
}

/// What the top-level switch `key` of the settings `existing` says, when it is set to true or
/// false; `None` when it is not set, or set to something else, which Claude Code would not take
/// as a switch either.
///
/// # Errors
///
/// As [`merged`]: the text is not a JSON object.
pub(crate) fn flag(existing: &str, key: &str) -> anyhow::Result<Option<bool>> {
    Ok(parsed(Some(existing))?.get(key).and_then(Value::as_bool))
}

/// The matcher group holding Cortex's hook for `entry`, run by `program`.
fn group_of(entry: &HookEntry, program: &str) -> Value {
    let timeout_s = u64::try_from(entry.hard_timeout.as_millis().div_ceil(1_000))
        .unwrap_or(u64::MAX)
        .max(1);
    let args = ARGS_PREFIX
        .iter()
        .copied()
        .chain(["--event", entry.event_name])
        .map(Value::from)
        .collect();
    let mut handler = Map::new();
    handler.insert("type".to_owned(), "command".into());
    handler.insert("command".to_owned(), program.into());
    handler.insert("args".to_owned(), Value::Array(args));
    handler.insert("timeout".to_owned(), timeout_s.into());
    handler.insert("onFailure".to_owned(), "continue".into());
    let mut group = Map::new();
    if let Some(matcher) = entry.matcher {
        group.insert("matcher".to_owned(), matcher.into());
    }
    group.insert(
        "hooks".to_owned(),
        Value::Array(vec![Value::Object(handler)]),
    );
    Value::Object(group)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cortex_core::HookKind;

    use super::*;

    /// The binary the tests register.
    const PROGRAM: &str = "/opt/cortex/bin/cortex";

    /// A hook with a matcher and a hard timeout of 300 ms: 1 s once rounded up.
    const PRE_TOOL_USE: HookEntry = HookEntry {
        event_name: "PreToolUse",
        matcher: Some("Read"),
        hook: HookKind::PreTool,
        hard_timeout: Duration::from_millis(300),
    };

    /// A hook without a matcher and a hard timeout of 1,500 ms: 2 s once rounded up.
    const SESSION_START: HookEntry = HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
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
}
