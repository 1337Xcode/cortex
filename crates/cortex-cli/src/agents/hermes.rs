//! Hermes Agent: its shell hooks and MCP servers are in its YAML configuration,
//! `~/.hermes/config.yaml` (`%LOCALAPPDATA%\hermes\config.yaml` on Windows), and it runs a hook
//! only once the user approves that event's command. Cortex writes no YAML and never approves a
//! hook, so init writes nothing for Hermes: it says the lines to add, one hook per event of the
//! adapter, each command run without a shell as Hermes splits it, and the MCP server, and the
//! approval that is the user's. Doctor looks for the hooks' lines in the file.

use anyhow::Context as _;
use cortex_core::Agent;

use super::command::{Shell, hook_command};
use super::yaml::quoted;
use super::{FoundBy, Setup, ToldLines};

/// One of Cortex's hooks for Hermes.
struct Hook {
    /// The event, as Hermes names it.
    event: &'static str,
    /// The tools it runs for, if not every one.
    matcher: Option<&'static str>,
    /// Its timeout in whole seconds.
    seconds: u128,
    /// The command line it runs.
    line: String,
}

/// Each of Cortex's hooks for Hermes, running `program`.
///
/// # Errors
///
/// This build has no Hermes adapter.
fn hooks(program: &str) -> anyhow::Result<Vec<Hook>> {
    let adapter = cortex_hook::adapter("hermes").context("finding the Hermes adapter")?;
    adapter
        .hooks()
        .iter()
        .map(|entry| {
            let line = hook_command(program, "hermes", entry.event_name, Shell::Posix)
                .with_context(|| format!("the path {program} cannot be written for Hermes"))?;
            let seconds = entry.hard_timeout.as_millis().div_ceil(1_000).max(1);
            Ok(Hook {
                event: entry.event_name,
                matcher: entry.matcher,
                seconds,
                line,
            })
        })
        .collect()
}

/// The lines to add to Hermes' configuration for `program`: Cortex's hooks under `hooks:` and its
/// MCP server under `mcp_servers:`.
///
/// # Errors
///
/// As [`hooks`].
fn lines(program: &str) -> anyhow::Result<String> {
    let mut text = String::from("hooks:\n");
    for Hook {
        event,
        matcher,
        seconds,
        line,
    } in hooks(program)?
    {
        text.push_str(&format!("  {event}:\n"));
        let mut first = "-";
        if let Some(matcher) = matcher {
            text.push_str(&format!("    - matcher: {}\n", quoted(matcher)));
            first = " ";
        }
        text.push_str(&format!("    {first} command: {}\n", quoted(&line)));
        text.push_str(&format!("      timeout: {seconds}\n"));
    }
    text.push_str(&format!(
        "mcp_servers:\n  cortex:\n    command: {}\n    args: ['mcp']\n",
        quoted(program)
    ));
    Ok(text)
}

/// The command lines of Cortex's hooks as [`lines`] writes them, quoted for YAML, which doctor
/// looks for in the file.
///
/// # Errors
///
/// As [`hooks`].
fn signs(program: &str) -> anyhow::Result<Vec<String>> {
    Ok(hooks(program)?
        .into_iter()
        .map(|hook| quoted(&hook.line))
        .collect())
}

/// Hermes Agent: found by its folder or the `hermes` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Hermes,
    found_by: FoundBy {
        folders: &[if cfg!(windows) {
            "AppData/Local/hermes"
        } else {
            ".hermes"
        }],
        programs: &["hermes"],
    },
    hooks: None,
    own: None,
    mcp: None,
    told_lines: Some(ToldLines {
        file: if cfg!(windows) {
            "AppData/Local/hermes/config.yaml"
        } else {
            ".hermes/config.yaml"
        },
        why: "its configuration is YAML, which Cortex does not write, and Hermes runs a hook only \
              once you approve it",
        lines,
        signs,
    }),
    told: &[
        "approve each of Cortex's hooks when Hermes first asks, in `hermes chat` in the \
         foreground (`hermes hooks list` shows what you approved); Cortex never approves them",
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines for a plain program, typed here by hand from Hermes' documentation and its
    /// adapter's table: the session's start 2 s, every other hook 1 s.
    #[test]
    fn the_lines_are_hermes_yaml() -> anyhow::Result<()> {
        let expected = "hooks:
  on_session_start:
    - command: '/opt/cortex hook --agent hermes --event on_session_start'
      timeout: 2
  pre_llm_call:
    - command: '/opt/cortex hook --agent hermes --event pre_llm_call'
      timeout: 1
  post_tool_call:
    - matcher: '^(read_file|write_file|patch|search_files|terminal)$'
      command: '/opt/cortex hook --agent hermes --event post_tool_call'
      timeout: 1
  on_session_end:
    - command: '/opt/cortex hook --agent hermes --event on_session_end'
      timeout: 1
mcp_servers:
  cortex:
    command: '/opt/cortex'
    args: ['mcp']
";
        assert_eq!(lines("/opt/cortex")?, expected);
        assert_eq!(
            signs("/opt/cortex")?.first().map(String::as_str),
            Some("'/opt/cortex hook --agent hermes --event on_session_start'")
        );
        Ok(())
    }

    /// Every sign doctor looks for is text the lines hold, for a plain path and for one with a
    /// space and a quote.
    #[test]
    fn the_lines_hold_every_sign() -> anyhow::Result<()> {
        for program in ["/opt/cortex", "/home/o'neil/my bin/cortex"] {
            let text = lines(program)?;
            let signs = signs(program)?;
            assert_eq!(signs.len(), 4);
            for sign in signs {
                assert!(text.contains(&sign), "{sign}");
            }
        }
        Ok(())
    }

    /// A path with a space and a quote is quoted for Hermes' split, then for YAML.
    #[test]
    fn a_path_is_quoted_twice() -> anyhow::Result<()> {
        let text = lines("/home/o'neil/my bin/cortex")?;
        assert!(
            text.contains(
                r"    - command: '''/home/o''\''''neil/my bin/cortex'' hook --agent hermes --event on_session_start'"
            ),
            "{text}"
        );
        Ok(())
    }
}
