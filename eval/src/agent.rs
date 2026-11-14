//! Claude Code, run headless for one session: the command, and what its stream of JSON lines says
//! the session took.
//!
//! The command loads the folder's own settings alone (`--setting-sources project,local`), where
//! `cortex init` puts its hooks, and no MCP server but the folder's (`--strict-mcp-config`); it
//! accepts edits and allows the task's tools, nothing more. `ANTHROPIC_API_KEY` is left out of its
//! environment, so it signs in as the user's own login does; the stream says which source it used.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::texts::Texts;

/// The environment variable left out of every agent's environment.
const API_KEY_VARIABLE: &str = "ANTHROPIC_API_KEY";

/// The command that runs `prompt` as one headless session of `claude` in `folder`, with `model`
/// at `effort`, allowed `tools`, and `home` as its home folder for Cortex while Claude Code keeps
/// its own configuration in `claude_config`.
pub(crate) fn command(
    prompt: &str,
    folder: &Path,
    (model, effort, tools): (&str, &str, &[String]),
    (home, claude_config): (&Path, &Path),
) -> Command {
    let mut command = Command::new("claude");
    command
        .args(["-p", prompt, "--model", model, "--effort", effort])
        .args(["--output-format", "stream-json", "--verbose"])
        .args(["--setting-sources", "project,local", "--strict-mcp-config"])
        .args(["--permission-mode", "acceptEdits"])
        .arg("--allowedTools")
        .arg(tools.join(","))
        .current_dir(folder)
        .env_remove(API_KEY_VARIABLE)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CLAUDE_CONFIG_DIR", claude_config);
    command
}

/// What a session's stream says it took.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Session {
    /// Its id, which reads the store and stays out of the results: an id names a real session.
    #[serde(skip)]
    pub(crate) session_id: Option<String>,
    /// The agent's version.
    pub(crate) agent_version: Option<String>,
    /// The model it ran.
    pub(crate) model: Option<String>,
    /// Where its credentials came from, as Claude Code names it (`none` for the user's login).
    pub(crate) api_key_source: Option<String>,
    /// Whether it ended in an error.
    pub(crate) is_error: bool,
    /// Its turns.
    pub(crate) turns: u64,
    /// Its tool calls.
    pub(crate) tool_calls: u64,
    /// Every input token: fresh, written to the cache and read from it.
    pub(crate) cumulative_input: u64,
    /// The fresh input.
    pub(crate) uncached_input: u64,
    /// Its output tokens.
    pub(crate) output: u64,
    /// What it cost, as Claude Code reports it, in US dollars.
    pub(crate) cost_usd: f64,
    /// How long it took, in milliseconds.
    pub(crate) duration_ms: u64,
    /// The texts Cortex gave it, as the store holds them; none in results from before the runner
    /// recorded them.
    #[serde(default)]
    pub(crate) texts: Option<Texts>,
}

impl Session {
    /// What the stream `text`, one JSON value a line, says the session took; lines that are not
    /// JSON are passed over, and a stream with no result line leaves the counts at zero and the
    /// session marked as an error.
    pub(crate) fn of_stream(text: &str) -> Self {
        let mut session = Self {
            is_error: true,
            ..Self::default()
        };
        for line in text.lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match (at(&value, "type").as_str(), at(&value, "subtype").as_str()) {
                (Some("system"), Some("init")) => session.read_init(&value),
                (Some("assistant"), _) => session.tool_calls += tool_uses(&value),
                (Some("result"), _) => session.read_result(&value),
                _ => {}
            }
        }
        session
    }

    /// Takes the session's id, version, model and credential source from its first line.
    fn read_init(&mut self, init: &Value) {
        let text = |key: &str| at(init, key).as_str().map(str::to_owned);
        self.session_id = text("session_id");
        self.agent_version = text("claude_code_version");
        self.model = text("model");
        self.api_key_source = text("apiKeySource");
    }

    /// Takes the session's counts from its result line.
    fn read_result(&mut self, result: &Value) {
        let count = |value: &Value| value.as_u64().unwrap_or(0);
        let usage = at(result, "usage");
        let fresh = count(at(usage, "input_tokens"));
        self.uncached_input = fresh;
        self.cumulative_input = fresh
            .saturating_add(count(at(usage, "cache_creation_input_tokens")))
            .saturating_add(count(at(usage, "cache_read_input_tokens")));
        self.output = count(at(usage, "output_tokens"));
        self.turns = count(at(result, "num_turns"));
        self.duration_ms = count(at(result, "duration_ms"));
        self.cost_usd = at(result, "total_cost_usd").as_f64().unwrap_or(0.0);
        self.is_error = at(result, "is_error").as_bool().unwrap_or(true);
    }
}

/// The value at `key` of `value`, null when there is none.
fn at<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// How many tool calls an assistant line holds.
fn tool_uses(line: &Value) -> u64 {
    at(at(line, "message"), "content")
        .as_array()
        .map_or(0, |blocks| {
            let calls = blocks
                .iter()
                .filter(|block| at(block, "type").as_str() == Some("tool_use"))
                .count();
            u64::try_from(calls).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream as Claude Code writes it, cut to the keys read: its init line, an assistant line
    /// with two tool calls and one with text, a line that is not JSON, and its result.
    const STREAM: &str = concat!(
        "{\"type\":\"system\",\"subtype\":\"hook_started\",\"session_id\":\"s1\"}\n",
        "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\",\"model\":\"m\",",
        "\"apiKeySource\":\"none\",\"claude_code_version\":\"2.1.318\"}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\"},",
        "{\"type\":\"text\"},{\"type\":\"tool_use\"}]}}\n",
        "not json\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\"}]}}\n",
        "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"num_turns\":9,",
        "\"duration_ms\":21944,\"total_cost_usd\":0.25,\"usage\":{\"input_tokens\":12,",
        "\"cache_creation_input_tokens\":3000,\"cache_read_input_tokens\":40000,",
        "\"output_tokens\":700}}\n"
    );

    /// The stream's counts, added by hand: cumulative input 12 + 3,000 + 40,000 = 43,012, of it
    /// 12 fresh; two tool calls; its init line's id, model, version and credential source.
    #[test]
    fn a_stream_says_what_its_session_took() {
        let session = Session::of_stream(STREAM);
        assert_eq!(
            session,
            Session {
                session_id: Some("s1".to_owned()),
                agent_version: Some("2.1.318".to_owned()),
                model: Some("m".to_owned()),
                api_key_source: Some("none".to_owned()),
                is_error: false,
                turns: 9,
                tool_calls: 2,
                cumulative_input: 43_012,
                uncached_input: 12,
                output: 700,
                cost_usd: 0.25,
                duration_ms: 21_944,
                texts: None,
            }
        );
    }

    /// What a session took is written to the results without its id, which names a real session.
    #[test]
    fn a_session_is_written_without_its_id() -> Result<(), serde_json::Error> {
        let written = serde_json::to_value(Session::of_stream(STREAM))?;
        assert_eq!(written.get("session_id"), None);
        assert_eq!(written.get("turns"), Some(&serde_json::json!(9)));
        Ok(())
    }

    /// A stream cut short before its result is an error with nothing counted.
    #[test]
    fn a_stream_without_its_result_is_an_error() {
        let cut = STREAM.lines().take(3).collect::<Vec<_>>().join("\n");
        let session = Session::of_stream(&cut);
        assert!(session.is_error);
        assert_eq!((session.turns, session.cumulative_input), (0, 0));
    }

    /// The command runs headless with the folder's settings only, accepts edits, allows the tools
    /// given, and leaves the API key out of the agent's environment.
    #[test]
    fn the_agent_runs_without_the_api_key() {
        let tools = ["Read".to_owned(), "Edit".to_owned()];
        let command = command(
            "fix it",
            Path::new("work"),
            ("sonnet", "medium", &tools),
            (Path::new("home"), Path::new("claude")),
        );
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-p",
                "fix it",
                "--model",
                "sonnet",
                "--effort",
                "medium",
                "--output-format",
                "stream-json",
                "--verbose",
                "--setting-sources",
                "project,local",
                "--strict-mcp-config",
                "--permission-mode",
                "acceptEdits",
                "--allowedTools",
                "Read,Edit"
            ]
        );
        let removed = command
            .get_envs()
            .any(|(key, value)| key == API_KEY_VARIABLE && value.is_none());
        assert!(removed, "the API key is left out");
    }
}
