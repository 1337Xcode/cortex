//! goose: its MCP servers ("extensions") are in its YAML configuration,
//! `~/.config/goose/config.yaml` (`%APPDATA%\Block\goose\config\config.yaml` on Windows), which
//! Cortex does not write, so init says the lines to add. Cortex writes no hooks for goose: it has
//! no adapter for goose's hook payloads yet, and a hook would run Cortex for an agent it cannot
//! read. Doctor looks for the program in the file.

use cortex_core::Agent;

use super::yaml::quoted;
use super::{FoundBy, Setup, ToldLines};

/// The lines to add under goose's `extensions:` for `program`, in the shape its documentation
/// gives a command-line extension.
///
/// # Errors
///
/// None: every path can be written in YAML.
fn lines(program: &str) -> anyhow::Result<String> {
    Ok(format!(
        "extensions:\n  cortex:\n    name: cortex\n    type: stdio\n    cmd: {}\n    args: ['mcp']\n    \
         enabled: true\n    env_keys: []\n    envs: {{}}\n    timeout: 300\n",
        quoted(program)
    ))
}

/// The program as [`lines`] writes it, quoted for YAML, which doctor looks for in the file.
///
/// # Errors
///
/// None.
fn signs(program: &str) -> anyhow::Result<Vec<String>> {
    Ok(vec![quoted(program)])
}

/// goose: found by its configuration folder or the `goose` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Goose,
    found_by: FoundBy {
        folders: &[if cfg!(windows) {
            "AppData/Roaming/Block/goose"
        } else {
            ".config/goose"
        }],
        programs: &["goose"],
    },
    hooks: None,
    own: None,
    mcp: None,
    told_lines: Some(ToldLines {
        file: if cfg!(windows) {
            "AppData/Roaming/Block/goose/config/config.yaml"
        } else {
            ".config/goose/config.yaml"
        },
        why: "its configuration is YAML, which Cortex does not write; and it gets no hooks, since \
              Cortex has no adapter for goose's hook payloads yet",
        lines,
        signs,
    }),
    told: &[],
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines for a Windows program, typed here by hand from goose's documentation.
    #[test]
    fn the_lines_are_goose_yaml() -> anyhow::Result<()> {
        let expected = r"extensions:
  cortex:
    name: cortex
    type: stdio
    cmd: 'C:\tools\cortex.exe'
    args: ['mcp']
    enabled: true
    env_keys: []
    envs: {}
    timeout: 300
";
        assert_eq!(lines(r"C:\tools\cortex.exe")?, expected);
        Ok(())
    }

    /// The sign doctor looks for is text the lines hold, for a path with a space and a quote.
    #[test]
    fn the_lines_hold_the_sign() -> anyhow::Result<()> {
        let program = r"C:\Users\Ann O'Neil\my bin\cortex.exe";
        let text = lines(program)?;
        for sign in signs(program)? {
            assert!(text.contains(&sign), "{sign}");
        }
        Ok(())
    }
}
