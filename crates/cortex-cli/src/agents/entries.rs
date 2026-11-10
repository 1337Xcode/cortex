//! Cortex's entries in the lists of an agent's hooks file, as a [`Style`] lays them out: the item
//! that holds Cortex's hook for one event (a matcher group, a handler of its own, or a named hook
//! naming its trigger), under the event's name as the file spells it; and Cortex's items found
//! again and taken out of a list, wherever they are, with where the first of them was and what
//! they held, so that the current ones go back there with the keys the user added to them.

use cortex_hook::HookEntry;
use serde_json::{Map, Value};

use super::handler::{handler_of, is_cortex, timeout_of};
use super::settings::{Events, Layout, Spelling, Style};

/// The keys Cortex writes on some of its items and not on others, which an older item's keys never
/// bring back: a matcher an event no longer has, an argument list or failure policy a command line
/// does not take.
const CORTEX_ONLY_KEYS: &[&str] = &["matcher", "args", "onFailure"];

/// What taking Cortex's hooks out of one list found.
#[derive(Debug)]
pub(super) struct Taken {
    /// Where the first item holding one of them was, among the items kept.
    pub(super) place: usize,
    /// What was taken out, in the list's order: each item taken whole, and each handler taken out
    /// of a group that stays, as a group of its own (`{"hooks": [handler]}`).
    pub(super) removed: Vec<Value>,
}

/// Takes every Cortex hook out of `hooks`, `program` being this binary, and returns, for each event
/// that held one, what [`take_cortex`] found. An event whose every hook was Cortex's goes, unless
/// its name is one of `keep`, where Cortex's current hooks go: it then keeps its place among the
/// events, emptied.
pub(super) fn take_cortex_hooks(
    hooks: &mut Map<String, Value>,
    keep: &[String],
    program: &str,
    style: &Style,
) -> Vec<(String, Taken)> {
    let mut taken = Vec::new();
    for (event, items) in hooks.iter_mut() {
        if let Value::Array(items) = items
            && let Some(found) = take_cortex(items, program, style)
        {
            taken.push((event.clone(), found));
        }
    }
    hooks.retain(|event, items| {
        let was_emptied = taken.iter().any(|(emptied, _)| emptied == event)
            && items.as_array().is_some_and(Vec::is_empty);
        !was_emptied || keep.contains(event)
    });
    taken
}

/// Takes every Cortex hook out of one list, `program` being this binary, and returns what it took
/// and where the first item holding one was, among the items kept; `None` when it held none. A
/// matcher group (an item with a `hooks` list, where `style` has groups) loses Cortex's handlers,
/// and goes when they were all it held; a handler of its own (where `style` has those) goes when
/// it is Cortex's. An item of another shape is left alone.
pub(super) fn take_cortex(items: &mut Vec<Value>, program: &str, style: &Style) -> Option<Taken> {
    let (has_groups, has_handlers) = match style.layout {
        Layout::Grouped => (true, false),
        Layout::Flat => (false, true),
        Layout::GroupedWhenMatched => (true, true),
    };
    let mut place = None;
    let mut removed = Vec::new();
    let mut kept = Vec::with_capacity(items.len());
    for mut item in items.drain(..) {
        let handlers = item
            .as_object_mut()
            .and_then(|group| group.get_mut("hooks"))
            .and_then(Value::as_array_mut)
            .filter(|_| has_groups);
        let Some(handlers) = handlers else {
            if has_handlers && is_cortex(&item, program, style) {
                place.get_or_insert(kept.len());
                removed.push(item);
            } else {
                kept.push(item);
            }
            continue;
        };
        let (cortex, users): (Vec<Value>, Vec<Value>) = handlers
            .drain(..)
            .partition(|handler| is_cortex(handler, program, style));
        *handlers = users;
        if cortex.is_empty() {
            kept.push(item);
            continue;
        }
        place.get_or_insert(kept.len());
        if handlers.is_empty() {
            if let Some(group) = item.as_object_mut() {
                group.insert("hooks".to_owned(), Value::Array(cortex));
            }
            removed.push(item);
        } else {
            let alone = cortex.into_iter().map(|handler| {
                let mut group = Map::new();
                group.insert("hooks".to_owned(), Value::Array(vec![handler]));
                Value::Object(group)
            });
            removed.extend(alone);
            kept.push(item);
        }
    }
    *items = kept;
    place.map(|place| Taken { place, removed })
}

/// `new`, Cortex's current item, with the keys the user added to `old`, the item of Cortex's it
/// replaces (an `enabled` switch, a description, a group's loop limit), after its own: every key
/// Cortex writes is the current one's, and a key Cortex writes only on some items
/// ([`CORTEX_ONLY_KEYS`]) is never brought back. A group's first handler keeps the user's keys of
/// the old group's first handler the same way.
pub(super) fn with_users_keys(new: Value, old: Option<&Value>) -> Value {
    let Some(old) = old.and_then(Value::as_object) else {
        return new;
    };
    let Value::Object(mut new) = new else {
        return new;
    };
    let old_handler = old
        .get("hooks")
        .and_then(Value::as_array)
        .and_then(|handlers| handlers.first());
    if let Some(Value::Array(handlers)) = new.get_mut("hooks")
        && let Some(first) = handlers.first_mut()
    {
        *first = with_users_keys(first.take(), old_handler);
    }
    for (key, value) in old {
        if !new.contains_key(key) && !CORTEX_ONLY_KEYS.contains(&key.as_str()) {
            new.insert(key.clone(), value.clone());
        }
    }
    Value::Object(new)
}

/// The name each of `entries` has in the events `hooks`, as `style` spells them; for a style that
/// spells them as the file does, as the user's own events there are spelled (those that still
/// hold a hook once Cortex's, run by `program`, are taken out).
pub(super) fn event_keys(
    hooks: &Map<String, Value>,
    entries: &[HookEntry],
    program: &str,
    style: &Style,
) -> Vec<String> {
    let is_snake = match style.spelling {
        Spelling::AsAdapter => false,
        Spelling::AsFile => {
            let mut users = hooks.clone();
            take_cortex_hooks(&mut users, &[], program, style);
            !users
                .keys()
                .any(|key| key.chars().any(|char| char.is_ascii_uppercase()))
        }
    };
    entries
        .iter()
        .map(|entry| {
            if is_snake {
                snake_case(entry.event_name)
            } else {
                entry.event_name.to_owned()
            }
        })
        .collect()
}

/// `name`, in Pascal case, in snake case: `SessionStart` is `session_start`.
fn snake_case(name: &str) -> String {
    let mut snake = String::with_capacity(name.len() + 4);
    for (at, char) in name.chars().enumerate() {
        if char.is_ascii_uppercase() && at > 0 {
            snake.push('_');
        }
        snake.push(char.to_ascii_lowercase());
    }
    snake
}

/// The item of an event's list, or of the one list of hooks, that holds Cortex's hook for `entry`,
/// run by `program`, written in `style`.
///
/// # Errors
///
/// The program's path needs quoting for a shell Cortex does not know.
pub(super) fn item_of(entry: &HookEntry, program: &str, style: &Style) -> anyhow::Result<Value> {
    if let Events::Triggers = style.events {
        return trigger_of(entry, program, style);
    }
    let is_grouped = match style.layout {
        Layout::Grouped => true,
        Layout::Flat => false,
        Layout::GroupedWhenMatched => entry.matcher.is_some(),
    };
    if !is_grouped {
        return Ok(Value::Object(handler_of(
            entry,
            program,
            style,
            entry.matcher,
        )?));
    }
    let mut group = Map::new();
    if let Some(matcher) = entry.matcher {
        group.insert("matcher".to_owned(), matcher.into());
    }
    let handler = handler_of(entry, program, style, None)?;
    group.insert(
        "hooks".to_owned(),
        Value::Array(vec![Value::Object(handler)]),
    );
    Ok(Value::Object(group))
}

/// The named hook, triggered by `entry`'s event, that runs `program` as its action, written in
/// `style`: `cortex <event>` for its name, then the event, the matcher if any, the action, and
/// the timeout, which sits on the hook rather than its action.
///
/// # Errors
///
/// The program's path needs quoting for a shell Cortex does not know.
fn trigger_of(entry: &HookEntry, program: &str, style: &Style) -> anyhow::Result<Value> {
    let mut action = handler_of(entry, program, style, None)?;
    action.shift_remove(style.keys.timeout);
    let mut hook = Map::new();
    hook.insert(
        "name".to_owned(),
        format!("cortex {}", entry.event_name).into(),
    );
    hook.insert("trigger".to_owned(), entry.event_name.into());
    if let Some(matcher) = entry.matcher {
        hook.insert("matcher".to_owned(), matcher.into());
    }
    hook.insert("action".to_owned(), Value::Object(action));
    hook.insert(
        style.keys.timeout.to_owned(),
        timeout_of(entry, style).into(),
    );
    Ok(Value::Object(hook))
}

/// Every handler of `lists`, the lists of a hooks file, in the file's order, as [`take_cortex`]
/// finds them: the matcher groups' handlers and the handlers of their own, or the named hooks.
pub(super) fn items_in<'s>(lists: Vec<&'s Vec<Value>>, style: &Style) -> Vec<&'s Value> {
    lists
        .into_iter()
        .flatten()
        .flat_map(|item| match item.get("hooks").and_then(Value::as_array) {
            Some(handlers) if style.layout != Layout::Flat => handlers.iter().collect(),
            _ if style.layout == Layout::Grouped => Vec::new(),
            _ => vec![item],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cortex_core::HookKind;
    use serde_json::json;

    use super::*;
    use crate::agents::command::Shell;
    use crate::agents::handler::Keys;
    use crate::agents::settings::{
        Handler, TimeoutUnit, Version, cortex_commands, is_current, is_turned_off, merged,
        without_cortex_hooks,
    };

    /// The binary the tests register.
    const PROGRAM: &str = "/opt/cortex/bin/cortex";

    /// A tool's hook with a matcher and a hard timeout of 500 ms: 1 s once rounded up.
    const POST_TOOL_USE: HookEntry = HookEntry {
        event_name: "PostToolUse",
        matcher: Some("^(Read|Write)$"),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    };

    /// A hook without a matcher and a hard timeout of 1,500 ms: 2 s once rounded up.
    const SESSION_START: HookEntry = HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    };

    /// A style of `layout` with its events in `events`, run as a POSIX command line, in seconds,
    /// with Claude Code's keys and no version.
    fn style(events: Events, layout: Layout) -> Style {
        Style {
            agent: "cursor",
            handler: Handler::CommandLine(Shell::Posix),
            timeout: TimeoutUnit::Seconds,
            events,
            spelling: Spelling::AsAdapter,
            layout,
            keys: Keys::CLAUDE,
            version: None,
        }
    }

    /// The command line of Cortex's hook at `event` in the tests' styles.
    fn line(event: &str) -> String {
        format!("{PROGRAM} hook --agent cursor --event {event}")
    }

    /// Handlers of their own, as Cursor's: a new file declares version 1, each handler carries its
    /// matcher, typed handlers; a user's handler on Cortex's event stays first; merging again changes
    /// nothing; the commands read back in the file's order; taking Cortex's out leaves the user's
    /// handler and the version.
    #[test]
    fn handlers_of_their_own_carry_their_matcher() -> anyhow::Result<()> {
        let flat = Style {
            version: Some(Version::Number(1)),
            ..style(Events::Under("hooks"), Layout::Flat)
        };
        let user = r#"{"hooks": {"PostToolUse": [{"command": "lint.sh", "matcher": "Write"}]}}"#;
        let written = merged(Some(user), PROGRAM, &[SESSION_START, POST_TOOL_USE], &flat)?;
        let expected = json!({
            "hooks": {
                "PostToolUse": [
                    {"command": "lint.sh", "matcher": "Write"},
                    {"type": "command", "matcher": "^(Read|Write)$",
                     "command": line("PostToolUse"), "timeout": 1}
                ],
                "SessionStart": [
                    {"type": "command", "command": line("SessionStart"), "timeout": 2}
                ]
            },
            "version": 1
        });
        assert_eq!(serde_json::from_str::<Value>(&written)?, expected);
        assert_eq!(
            merged(
                Some(&written),
                PROGRAM,
                &[SESSION_START, POST_TOOL_USE],
                &flat
            )?,
            written
        );
        assert_eq!(
            cortex_commands(&written, PROGRAM, &flat)?,
            [line("PostToolUse"), line("SessionStart")]
        );
        let new = merged(None, PROGRAM, &[SESSION_START], &flat)?;
        assert!(
            new.starts_with("{\n  \"version\": 1,\n  \"hooks\": {"),
            "{new}"
        );
        let without = without_cortex_hooks(&written, PROGRAM, &flat)?.unwrap_or_default();
        assert_eq!(
            serde_json::from_str::<Value>(&without)?,
            json!({"hooks": {"PostToolUse": [{"command": "lint.sh", "matcher": "Write"}]},
                   "version": 1})
        );
        Ok(())
    }

    /// Antigravity's layout: the events under Cortex's own hook name, a tool's event in a matcher
    /// group and another event's handler straight in its list; the user's hook names stay, and
    /// taking Cortex's out takes its name away with them.
    #[test]
    fn a_tool_s_event_alone_is_grouped() -> anyhow::Result<()> {
        let mixed = style(Events::Under("cortex"), Layout::GroupedWhenMatched);
        let user = r#"{"lint": {"PostToolUse": [{"matcher": "run_command", "hooks": [{"type": "command", "command": "lint.sh"}]}]}}"#;
        let written = merged(Some(user), PROGRAM, &[SESSION_START, POST_TOOL_USE], &mixed)?;
        let expected = json!({
            "lint": {"PostToolUse": [{"matcher": "run_command",
                "hooks": [{"type": "command", "command": "lint.sh"}]}]},
            "cortex": {
                "SessionStart": [
                    {"type": "command", "command": line("SessionStart"), "timeout": 2}
                ],
                "PostToolUse": [{"matcher": "^(Read|Write)$", "hooks": [
                    {"type": "command", "command": line("PostToolUse"), "timeout": 1}
                ]}]
            }
        });
        assert_eq!(serde_json::from_str::<Value>(&written)?, expected);
        assert_eq!(
            merged(
                Some(&written),
                PROGRAM,
                &[SESSION_START, POST_TOOL_USE],
                &mixed
            )?,
            written
        );
        let without = without_cortex_hooks(&written, PROGRAM, &mixed)?.unwrap_or_default();
        assert_eq!(
            serde_json::from_str::<Value>(&without)?,
            serde_json::from_str::<Value>(user)?
        );
        Ok(())
    }

    /// Kiro's layout: one list of named hooks, each naming its trigger, its matcher on it, the
    /// command line as its action and the timeout on the hook; a new file declares version `v1`.
    /// An older Cortex hook ahead of the user's is replaced where it was, Cortex's hooks in their
    /// table's order; taking them out leaves the user's.
    #[test]
    fn named_hooks_name_their_trigger() -> anyhow::Result<()> {
        let triggers = Style {
            version: Some(Version::Text("v1")),
            ..style(Events::Triggers, Layout::Flat)
        };
        let user_hook = json!({"name": "lint", "trigger": "PostFileSave",
            "action": {"type": "command", "command": "lint.sh"}});
        let existing = json!({"version": "v1", "hooks": [
            {"name": "cortex Stop", "trigger": "Stop",
             "action": {"type": "command", "command": "/old/cortex hook --agent cursor --event Stop"}},
            user_hook
        ]});
        let written = merged(
            Some(&existing.to_string()),
            PROGRAM,
            &[SESSION_START, POST_TOOL_USE],
            &triggers,
        )?;
        let expected = json!({"version": "v1", "hooks": [
            {"name": "cortex SessionStart", "trigger": "SessionStart",
             "action": {"type": "command", "command": line("SessionStart")}, "timeout": 2},
            {"name": "cortex PostToolUse", "trigger": "PostToolUse", "matcher": "^(Read|Write)$",
             "action": {"type": "command", "command": line("PostToolUse")}, "timeout": 1},
            user_hook
        ]});
        assert_eq!(serde_json::from_str::<Value>(&written)?, expected);
        assert_eq!(
            merged(
                Some(&written),
                PROGRAM,
                &[SESSION_START, POST_TOOL_USE],
                &triggers
            )?,
            written
        );
        assert_eq!(
            cortex_commands(&written, PROGRAM, &triggers)?,
            [line("SessionStart"), line("PostToolUse")]
        );
        let without = without_cortex_hooks(&written, PROGRAM, &triggers)?.unwrap_or_default();
        assert_eq!(
            serde_json::from_str::<Value>(&without)?,
            json!({"version": "v1", "hooks": [user_hook]})
        );
        let error = merged(
            Some(r#"{"hooks": {}}"#),
            PROGRAM,
            &[SESSION_START],
            &triggers,
        )
        .err()
        .map(|error| error.to_string());
        assert_eq!(error.as_deref(), Some("its `hooks` is not a list"));
        Ok(())
    }

    /// An agent's own keys, as Copilot's and Crush's: the command line under the shell's key, the
    /// timeout as `timeoutSec`; no `type` where the agent documents none.
    #[test]
    fn an_agent_s_own_keys_name_the_parts() -> anyhow::Result<()> {
        let copilot = Style {
            keys: Keys {
                command: "bash",
                timeout: "timeoutSec",
                is_typed: true,
            },
            ..style(Events::Under("hooks"), Layout::Flat)
        };
        let written = merged(None, PROGRAM, &[SESSION_START], &copilot)?;
        assert_eq!(
            serde_json::from_str::<Value>(&written)?,
            json!({"hooks": {"SessionStart": [
                {"type": "command", "bash": line("SessionStart"), "timeoutSec": 2}
            ]}})
        );
        assert_eq!(
            cortex_commands(&written, PROGRAM, &copilot)?,
            [line("SessionStart")]
        );
        let crush = Style {
            keys: Keys {
                is_typed: false,
                ..Keys::CLAUDE
            },
            ..style(Events::Under("hooks"), Layout::Flat)
        };
        let written = merged(None, PROGRAM, &[POST_TOOL_USE], &crush)?;
        assert_eq!(
            serde_json::from_str::<Value>(&written)?,
            json!({"hooks": {"PostToolUse": [
                {"matcher": "^(Read|Write)$", "command": line("PostToolUse"), "timeout": 1}
            ]}})
        );
        Ok(())
    }

    /// A file is current when merging in Cortex's hooks would change nothing but its white space
    /// and the order of its keys; one hook missing, or a hook running another binary, is not.
    #[test]
    fn current_hooks_are_told_from_drifted_ones() -> anyhow::Result<()> {
        let flat = style(Events::Under("hooks"), Layout::Flat);
        let entries = [SESSION_START, POST_TOOL_USE];
        let written = merged(None, PROGRAM, &entries, &flat)?;
        assert!(is_current(&written, PROGRAM, &entries, &flat)?);
        let compact = serde_json::to_string(&serde_json::from_str::<Value>(&written)?)?;
        assert!(is_current(&compact, PROGRAM, &entries, &flat)?);
        let one = merged(None, PROGRAM, &[SESSION_START], &flat)?;
        assert!(!is_current(&one, PROGRAM, &entries, &flat)?);
        let other = written.replace(PROGRAM, "/usr/old/cortex");
        assert!(!is_current(&other, PROGRAM, &entries, &flat)?);
        assert!(is_current("[]", PROGRAM, &entries, &flat).is_err());
        Ok(())
    }

    /// The keys a user adds to Cortex's hooks stay when init writes them again, and so does a switch
    /// turning them off, which doctor reads: Kiro's `enabled`, `description` and `confirm` on a named
    /// hook, Cursor's `failClosed` on a handler, a group's `loop_limit` and its handler's `enabled`;
    /// Cortex's command, matcher and timeout are the current ones. Antigravity's switch on the hook
    /// name `cortex` stays too. A file with no switch turned off is not turned off.
    #[test]
    fn the_user_s_keys_on_cortex_s_hooks_stay() -> anyhow::Result<()> {
        let triggers = Style {
            version: Some(Version::Text("v1")),
            ..style(Events::Triggers, Layout::Flat)
        };
        let old = json!({"version": "v1", "hooks": [
            {"name": "cortex PostToolUse", "trigger": "PostToolUse", "matcher": "^old$",
             "description": "mine", "enabled": false, "confirm": true,
             "action": {"type": "command", "command": "/old/cortex hook --agent cursor --event PostToolUse"},
             "timeout": 9}
        ]});
        let written = merged(Some(&old.to_string()), PROGRAM, &[POST_TOOL_USE], &triggers)?;
        assert_eq!(
            serde_json::from_str::<Value>(&written)?,
            json!({"version": "v1", "hooks": [
                {"name": "cortex PostToolUse", "trigger": "PostToolUse", "matcher": "^(Read|Write)$",
                 "action": {"type": "command", "command": line("PostToolUse")}, "timeout": 1,
                 "description": "mine", "enabled": false, "confirm": true}
            ]})
        );
        assert_eq!(
            merged(Some(&written), PROGRAM, &[POST_TOOL_USE], &triggers)?,
            written
        );
        assert!(is_turned_off(&written, PROGRAM, &triggers)?);
        let on = merged(None, PROGRAM, &[POST_TOOL_USE], &triggers)?;
        assert!(!is_turned_off(&on, PROGRAM, &triggers)?);

        let flat = style(Events::Under("hooks"), Layout::Flat);
        let old = json!({"hooks": {"SessionStart": [
            {"command": "/old/cortex hook --agent cursor --event SessionStart", "matcher": "x",
             "failClosed": false}
        ]}});
        let written = merged(Some(&old.to_string()), PROGRAM, &[SESSION_START], &flat)?;
        assert_eq!(
            serde_json::from_str::<Value>(&written)?,
            json!({"hooks": {"SessionStart": [
                {"type": "command", "command": line("SessionStart"), "timeout": 2,
                 "failClosed": false}
            ]}})
        );

        let grouped = style(Events::Under("cortex"), Layout::GroupedWhenMatched);
        let old = json!({"cortex": {"enabled": false, "PostToolUse": [{"matcher": "^old$",
            "loop_limit": 3, "hooks": [{"type": "command", "enabled": false,
            "command": "/old/cortex hook --agent cursor --event PostToolUse"}]}]}});
        let written = merged(Some(&old.to_string()), PROGRAM, &[POST_TOOL_USE], &grouped)?;
        assert_eq!(
            serde_json::from_str::<Value>(&written)?,
            json!({"cortex": {"enabled": false, "PostToolUse": [{"matcher": "^(Read|Write)$",
                "loop_limit": 3, "hooks": [{"type": "command", "command": line("PostToolUse"),
                "timeout": 1, "enabled": false}]}]}})
        );
        assert_eq!(
            merged(Some(&written), PROGRAM, &[POST_TOOL_USE], &grouped)?,
            written
        );
        assert!(is_turned_off(&written, PROGRAM, &grouped)?);
        let switch_only = json!({"cortex": {"enabled": false}}).to_string();
        let written = merged(Some(&switch_only), PROGRAM, &[POST_TOOL_USE], &grouped)?;
        assert!(is_turned_off(&written, PROGRAM, &grouped)?);
        assert!(is_current(&written, PROGRAM, &[POST_TOOL_USE], &grouped)?);
        Ok(())
    }

    /// OpenHands' file: a fresh one names Cortex's events in snake case at its top; a user's
    /// `stop` in snake case gets Cortex's `Stop` hook in the same list, never a second `Stop`; a
    /// user's `Stop` in Pascal case gets Pascal case; a file keeping its events under `hooks`
    /// gets Cortex's there, its other keys left alone. Merging again changes nothing.
    #[test]
    fn openhands_events_follow_the_file_s_spelling_and_wrapper() -> anyhow::Result<()> {
        let openhands = Style {
            spelling: Spelling::AsFile,
            ..style(Events::WholeFileOrHooks, Layout::Grouped)
        };
        const STOP: HookEntry = HookEntry {
            event_name: "Stop",
            matcher: None,
            hook: HookKind::Stop,
            hard_timeout: Duration::from_millis(300),
            gives_context: false,
        };
        let entries = [SESSION_START, STOP];
        let group = |event: &str, seconds: u64| json!({"hooks": [{"type": "command", "command": line(event), "timeout": seconds}]});
        let users = json!({"hooks": [{"command": "notify.sh"}]});
        let cases = [
            (
                None,
                json!({"session_start": [group("SessionStart", 2)], "stop": [group("Stop", 1)]}),
            ),
            (
                Some(json!({"stop": [users]})),
                json!({"stop": [users, group("Stop", 1)],
                       "session_start": [group("SessionStart", 2)]}),
            ),
            (
                Some(json!({"Stop": [users]})),
                json!({"Stop": [users, group("Stop", 1)],
                       "SessionStart": [group("SessionStart", 2)]}),
            ),
            (
                Some(json!({"hooks": {"stop": [users]}, "note": "kept"})),
                json!({"hooks": {"stop": [users, group("Stop", 1)],
                                 "session_start": [group("SessionStart", 2)]},
                       "note": "kept"}),
            ),
        ];
        for (before, expected) in cases {
            let before = before.map(|before| before.to_string());
            let written = merged(before.as_deref(), PROGRAM, &entries, &openhands)?;
            assert_eq!(
                serde_json::from_str::<Value>(&written)?,
                expected,
                "{before:?}"
            );
            assert_eq!(
                merged(Some(&written), PROGRAM, &entries, &openhands)?,
                written
            );
        }
        Ok(())
    }
}
