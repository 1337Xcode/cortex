//! Cortex's MCP server in an agent's JSON settings: one entry named `cortex`, under the object of
//! servers the agent reads (`mcp`, `mcpServers`, Amp's `amp.mcpServers`), that starts this
//! program with `mcp`, in the shape the agent takes ([`Shape`]). The entry goes where it was, or
//! last; every other key of the file stays as it was and in its order, and so do the keys the
//! user added to Cortex's entry (a switch that turns it off, a list of tools to allow). An entry
//! named `cortex` that runs another program is the user's, and is left as it is: init then says
//! what to add instead. A file that is not plain JSON, as one holding comments, is never
//! rewritten, since writing it back would drop them.

use anyhow::{Context as _, bail};
use serde_json::{Map, Value, json};

use super::settings::{parsed, text_of};
use super::user_path::UserPath;

/// The name of Cortex's entry among the agent's servers.
pub(crate) const NAME: &str = "cortex";

/// The name init tells the user to give Cortex's entry when a server of another program already
/// has [`NAME`].
pub(crate) const OTHER_NAME: &str = "cortex-index";

/// The settings file an agent reads its MCP servers from, and how it holds them.
#[derive(Debug)]
pub(crate) struct McpFile {
    /// The user's files the agent reads: the first that exists is edited, and the first is made
    /// when none does.
    pub(crate) user: &'static [UserPath],
    /// The keys, from the file's top, of the object holding the servers.
    pub(crate) servers: &'static [&'static str],
    /// How an entry runs its server.
    pub(crate) shape: Shape,
    /// The key and value with which the user turns an entry off (`"enabled": false`), if the
    /// agent documents one.
    pub(crate) off: Option<(&'static str, bool)>,
}

/// How an agent's MCP entry runs a local server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// `{"type": "local", "command": [program, args...]}` (OpenCode's and Kilo Code's).
    LocalList,
    /// `{"command": program, "args": [args...]}`.
    CommandArgs,
}

/// Cortex's entry in `shape`, running `program` with `mcp`.
pub(crate) fn entry(program: &str, shape: Shape) -> Value {
    match shape {
        Shape::LocalList => json!({"type": "local", "command": [program, "mcp"]}),
        Shape::CommandArgs => json!({"command": program, "args": ["mcp"]}),
    }
}

/// The keys of `file`'s servers as people write them: `mcp.servers`.
pub(crate) fn servers_path(file: &McpFile) -> String {
    file.servers.join(".")
}

/// The settings `existing` (a file's text, or `None` for no file) with Cortex's current entry, run
/// by `program`, as text: pretty JSON with a final line break.
///
/// # Errors
///
/// The text is not plain JSON or not an object, something on the way to the servers is not an
/// object, or the servers hold an entry named `cortex` that runs another program.
pub(crate) fn merged(
    existing: Option<&str>,
    program: &str,
    file: &McpFile,
) -> anyhow::Result<String> {
    let mut settings = parsed(existing)?;
    let servers = servers_in(&mut settings, file)?;
    let old = servers.get(NAME);
    if old.is_some_and(|old| !is_cortex_entry(old, program)) {
        bail!(
            "its `{}.{NAME}` is an MCP server of another program",
            servers_path(file)
        );
    }
    let mut new = entry(program, file.shape);
    if let (Some(Value::Object(old)), Value::Object(new)) = (old, &mut new) {
        for (key, value) in old {
            if !new.contains_key(key) && !matches!(key.as_str(), "type" | "command" | "args") {
                new.insert(key.clone(), value.clone());
            }
        }
    }
    // An entry already there keeps its place among the servers.
    servers.insert(NAME.to_owned(), new);
    text_of(settings)
}

/// The settings `existing` without Cortex's entry, run by `program` or an older Cortex, as text in
/// the form [`merged`] writes; `None` when it holds none. An object on the way to the servers that
/// is left empty goes too.
///
/// # Errors
///
/// The text is not plain JSON, or not an object.
pub(crate) fn without(
    existing: &str,
    program: &str,
    file: &McpFile,
) -> anyhow::Result<Option<String>> {
    let mut settings = parsed(Some(existing))?;
    if !take_out(&mut settings, file.servers, program) {
        return Ok(None);
    }
    text_of(settings).map(Some)
}

/// Takes Cortex's entry out of the object at `keys` in `object`, and each object on the way left
/// empty; whether there was one.
fn take_out(object: &mut Map<String, Value>, keys: &[&str], program: &str) -> bool {
    let Some((first, rest)) = keys.split_first() else {
        let is_cortex = object
            .get(NAME)
            .is_some_and(|entry| is_cortex_entry(entry, program));
        if is_cortex {
            object.shift_remove(NAME);
        }
        return is_cortex;
    };
    let Some(Value::Object(inner)) = object.get_mut(*first) else {
        return false;
    };
    let was_taken = take_out(inner, rest, program);
    if was_taken && inner.is_empty() {
        object.shift_remove(*first);
    }
    was_taken
}

/// Whether the settings `existing` hold Cortex's current entry, run by `program`: merging it in
/// would change nothing but the file's layout of white space and keys.
///
/// # Errors
///
/// As [`merged`].
pub(crate) fn is_current(existing: &str, program: &str, file: &McpFile) -> anyhow::Result<bool> {
    let expected: Value = serde_json::from_str(&merged(Some(existing), program, file)?)?;
    Ok(Value::Object(parsed(Some(existing))?) == expected)
}

/// Whether the settings `existing` hold an entry of Cortex's, run by `program` or an older Cortex;
/// a file that is not plain JSON holds none Cortex can tell.
pub(crate) fn holds_cortex(existing: &str, program: &str, file: &McpFile) -> bool {
    entry_in(existing, file).is_some_and(|entry| is_cortex_entry(&entry, program))
}

/// Whether the settings `existing` hold an entry named [`NAME`] that runs another program than
/// Cortex.
pub(crate) fn is_name_taken(existing: &str, program: &str, file: &McpFile) -> bool {
    entry_in(existing, file).is_some_and(|entry| !is_cortex_entry(&entry, program))
}

/// Whether the user turned Cortex's entry, run by `program` or an older Cortex, off in the
/// settings `existing`, with the switch the agent documents. Init leaves such a switch as it is.
pub(crate) fn is_turned_off(existing: &str, program: &str, file: &McpFile) -> bool {
    let Some((key, off)) = file.off else {
        return false;
    };
    entry_in(existing, file).is_some_and(|entry| {
        is_cortex_entry(&entry, program) && entry.get(key).and_then(Value::as_bool) == Some(off)
    })
}

/// The entry named [`NAME`] among the servers of the settings `existing`, if they are plain JSON
/// and hold one.
fn entry_in(existing: &str, file: &McpFile) -> Option<Value> {
    let settings = parsed(Some(existing)).ok()?;
    let mut object = &settings;
    for key in file.servers {
        object = object.get(*key)?.as_object()?;
    }
    object.get(NAME).cloned()
}

/// The object of servers in `settings`, each object on the way made when missing.
///
/// # Errors
///
/// Something on the way is not an object.
fn servers_in<'s>(
    settings: &'s mut Map<String, Value>,
    file: &McpFile,
) -> anyhow::Result<&'s mut Map<String, Value>> {
    let mut object = settings;
    for key in file.servers {
        let inner = object
            .entry(*key)
            .or_insert_with(|| Value::Object(Map::new()));
        object = inner
            .as_object_mut()
            .with_context(|| format!("its `{key}` is not a JSON object"))?;
    }
    Ok(object)
}

/// Whether `entry` is Cortex's MCP server: its program is `program`, this binary, whatever it is
/// named, or a file named `cortex` (`cortex.exe` on Windows) in any folder, and its only argument
/// is `mcp`. The program and its arguments are read in either [`Shape`].
fn is_cortex_entry(entry: &Value, program: &str) -> bool {
    let (command, args): (Option<&str>, Vec<&Value>) = match entry.get("command") {
        Some(Value::Array(parts)) => (
            parts.first().and_then(Value::as_str),
            parts.iter().skip(1).collect(),
        ),
        Some(Value::String(command)) => (
            Some(command.as_str()),
            entry
                .get("args")
                .and_then(Value::as_array)
                .map(|args| args.iter().collect())
                .unwrap_or_default(),
        ),
        _ => (None, Vec::new()),
    };
    let Some(command) = command else {
        return false;
    };
    let name = command.rsplit(['/', '\\']).next().unwrap_or(command);
    let stem = name
        .rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("exe"))
        .map_or(name, |(stem, _)| stem);
    let is_cortex = stem.eq_ignore_ascii_case("cortex") || command == program;
    is_cortex && args.len() == 1 && args.first().and_then(|arg| arg.as_str()) == Some("mcp")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The binary the tests register.
    const PROGRAM: &str = "/opt/cortex/bin/cortex";

    /// OpenCode's shape: servers under `mcp`.
    const OPENCODE: McpFile = McpFile {
        user: &[UserPath::config("opencode/opencode.json")],
        servers: &["mcp"],
        shape: Shape::LocalList,
        off: Some(("enabled", false)),
    };

    /// OpenClaw's shape: servers two keys down, `mcp.servers`.
    const NESTED: McpFile = McpFile {
        user: &[UserPath::home(".openclaw/openclaw.json")],
        servers: &["mcp", "servers"],
        shape: Shape::CommandArgs,
        off: None,
    };

    /// No file: an object holding only Cortex's entry, typed here by hand.
    #[test]
    fn no_file_gets_cortex_s_entry() -> anyhow::Result<()> {
        let expected = r#"{
  "mcp": {
    "cortex": {
      "type": "local",
      "command": [
        "/opt/cortex/bin/cortex",
        "mcp"
      ]
    }
  }
}
"#;
        assert_eq!(merged(None, PROGRAM, &OPENCODE)?, expected);
        Ok(())
    }

    /// The user's keys and servers stay in their order; Cortex's entry already there keeps its
    /// place and the key the user added to it, and takes the current program; taking it out again
    /// gives back the user's settings.
    #[test]
    fn the_user_s_keys_and_servers_stay() -> anyhow::Result<()> {
        let existing = r#"{"model": "x", "mcp": {"cortex": {"type": "local", "command": ["/old/cortex", "mcp"], "enabled": false}, "github": {"type": "remote", "url": "https://example.test"}}, "theme": "dark"}"#;
        let expected = json!({"model": "x", "mcp": {
            "cortex": {"type": "local", "command": [PROGRAM, "mcp"], "enabled": false},
            "github": {"type": "remote", "url": "https://example.test"}
        }, "theme": "dark"});
        let written = merged(Some(existing), PROGRAM, &OPENCODE)?;
        assert_eq!(serde_json::from_str::<Value>(&written)?, expected);
        let keys: Vec<&str> = written
            .lines()
            .filter_map(|line| line.strip_prefix("  \""))
            .filter_map(|line| line.split('"').next())
            .collect();
        assert_eq!(keys, ["model", "mcp", "theme"]);
        assert!(written.find("\"cortex\"") < written.find("\"github\""));
        assert!(is_current(&written, PROGRAM, &OPENCODE)?);
        assert!(!is_current(existing, PROGRAM, &OPENCODE)?);
        let back = without(&written, PROGRAM, &OPENCODE)?
            .ok_or("no entry")
            .map_err(anyhow::Error::msg)?;
        assert_eq!(
            serde_json::from_str::<Value>(&back)?,
            json!({"model": "x", "mcp": {"github": {"type": "remote", "url": "https://example.test"}}, "theme": "dark"})
        );
        Ok(())
    }

    /// Servers two keys down are made on the way, and both objects go again once Cortex's entry
    /// is taken out, leaving the user's key.
    #[test]
    fn nested_servers_come_and_go() -> anyhow::Result<()> {
        let written = merged(Some(r#"{"gateway": {"port": 1}}"#), PROGRAM, &NESTED)?;
        assert_eq!(
            serde_json::from_str::<Value>(&written)?,
            json!({"gateway": {"port": 1}, "mcp": {"servers": {"cortex": {"command": PROGRAM, "args": ["mcp"]}}}})
        );
        let back = without(&written, PROGRAM, &NESTED)?
            .ok_or("no entry")
            .map_err(anyhow::Error::msg)?;
        assert_eq!(
            serde_json::from_str::<Value>(&back)?,
            json!({"gateway": {"port": 1}})
        );
        assert_eq!(without(&back, PROGRAM, &NESTED)?, None);
        Ok(())
    }

    /// An entry named `cortex` that runs another program, or Cortex with other arguments, is the
    /// user's: it is refused, and never taken out.
    #[test]
    fn another_program_s_cortex_entry_is_the_user_s() -> anyhow::Result<()> {
        for existing in [
            r#"{"mcp": {"cortex": {"type": "local", "command": ["npx", "cortex-mcp"]}}}"#,
            r#"{"mcp": {"cortex": {"type": "local", "command": ["/opt/cortex", "serve"]}}}"#,
        ] {
            let refused = merged(Some(existing), PROGRAM, &OPENCODE)
                .err()
                .map(|error| error.to_string());
            assert_eq!(
                refused.as_deref(),
                Some("its `mcp.cortex` is an MCP server of another program")
            );
            assert_eq!(without(existing, PROGRAM, &OPENCODE)?, None);
            assert!(!holds_cortex(existing, PROGRAM, &OPENCODE));
            assert!(is_name_taken(existing, PROGRAM, &OPENCODE));
        }
        let written = merged(None, PROGRAM, &OPENCODE)?;
        assert!(!is_name_taken(&written, PROGRAM, &OPENCODE));
        Ok(())
    }

    /// The switch the agent documents, set to off on Cortex's entry, is the user's turning it off;
    /// the same switch on, on another server, or on an agent that documents none, is not.
    #[test]
    fn the_user_s_switch_turns_cortex_s_entry_off() {
        let off = r#"{"mcp": {"cortex": {"type": "local", "command": ["/opt/cortex/bin/cortex", "mcp"], "enabled": false}}}"#;
        assert!(is_turned_off(off, PROGRAM, &OPENCODE));
        assert!(!is_turned_off(off, PROGRAM, &NESTED));
        let on = off.replace("false", "true");
        assert!(!is_turned_off(&on, PROGRAM, &OPENCODE));
        let other =
            r#"{"mcp": {"cortex": {"type": "local", "command": ["npx", "x"], "enabled": false}}}"#;
        assert!(!is_turned_off(other, PROGRAM, &OPENCODE));
    }

    /// A file with comments is not plain JSON: it is refused, never rewritten.
    #[test]
    fn a_file_with_comments_is_refused() {
        let existing = "{\n  // mine\n  \"mcp\": {}\n}\n";
        assert!(merged(Some(existing), PROGRAM, &OPENCODE).is_err());
        assert!(without(existing, PROGRAM, &OPENCODE).is_err());
    }
}
