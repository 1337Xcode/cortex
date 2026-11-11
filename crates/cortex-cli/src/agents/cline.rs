//! Cline's Visual Studio Code extension: a hook is a file named after its event, in a repository's
//! `.clinerules/hooks/` or `Cline/Hooks/` in the user's documents folder as the system names it
//! (see [`super::user_path`]), and Cline runs both. Cortex writes one file of its own per event of
//! its adapter, each running `cortex hook --agent cline --event <event>` with the payload Cline
//! gives on standard input: on Windows `<event>.ps1`, which Cline runs with PowerShell, written
//! with a UTF-8 byte order mark, without which Windows PowerShell 5.1 reads the script in the
//! system's ANSI code page and a path such as `C:\Users\José\...` would name another file;
//! elsewhere `<event>`, a POSIX shell script with its line naming the shell and the executable
//! bit, as Cline requires. A file of the user's of the same name is never written over. Cortex's
//! MCP server goes under `mcpServers` in `~/.cline/data/settings/cline_mcp_settings.json`, which
//! the extension and Cline's command line share.

use anyhow::Context as _;
use cortex_core::Agent;

use super::command::{Shell, hook_command};
use super::mcp::{McpFile, McpPlace, Shape};
use super::own::{OwnFile, OwnFiles, OwnKind, UsersFolder};
use super::user_path::UserPath;
use super::{FoundBy, Setup};

/// Cortex's hook script for each event of Cline's adapter, running `program`.
///
/// # Errors
///
/// This build has no Cline adapter, or the program's path cannot be written for PowerShell.
fn files(program: &str) -> anyhow::Result<Vec<OwnFile>> {
    let adapter = cortex_hook::adapter("cline").context("finding the Cline adapter")?;
    adapter
        .hooks()
        .iter()
        .map(|entry| script(program, entry.event_name))
        .collect()
}

/// The script that runs `program` at `event`, for this system.
///
/// # Errors
///
/// The program's path cannot be written for PowerShell.
fn script(program: &str, event: &str) -> anyhow::Result<OwnFile> {
    let first = format!(
        "# Cortex's Cline hook for {event}, written by `cortex init`; `cortex uninstall` removes it."
    );
    let (name, text) = if cfg!(windows) {
        let line = hook_command(program, "cline", event, Shell::Powershell)
            .with_context(|| format!("the path {program} cannot be written for PowerShell"))?;
        // The payload and the reply are UTF-8; PowerShell would read and pass them in the
        // console's code page otherwise. A process with no console cannot set it, and goes on.
        (
            format!("{event}.ps1"),
            format!(
                "\u{feff}{first}\ntry {{ [Console]::InputEncoding = [Console]::OutputEncoding = \
                 [Text.UTF8Encoding]::new($false) }} catch {{}}\n$OutputEncoding = \
                 [Text.UTF8Encoding]::new($false)\n[Console]::In.ReadToEnd() | {line}\n"
            ),
        )
    } else {
        let line = hook_command(program, "cline", event, Shell::Posix)
            .with_context(|| format!("the path {program} cannot be written for a shell"))?;
        (
            event.to_owned(),
            format!("#!/bin/sh\n{first}\nexec {line}\n"),
        )
    };
    Ok(OwnFile {
        name,
        text,
        is_executable: true,
    })
}

/// Cline: found by its folder, its folder in the user's documents, or the `cline` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Cline,
    found_by: FoundBy {
        folders: &[".cline", "Documents/Cline"],
        programs: &["cline"],
    },
    hooks: None,
    own: Some(OwnFiles {
        project: Some(".clinerules/hooks"),
        user: UsersFolder::At(UserPath::documents("Cline/Hooks")),
        loads_both: true,
        kind: OwnKind::Hook,
        hides: &[],
        files,
        after_removal: None,
    }),
    mcp: Some(McpFile {
        place: McpPlace::Files(&[UserPath::home(
            ".cline/data/settings/cline_mcp_settings.json",
        )]),
        servers: &["mcpServers"],
        shape: Shape::CommandArgs,
        off: Some(("disabled", true)),
    }),
    told_lines: None,
    told: &[
        "keep Cline's Hooks setting on (Settings > Feature Settings > Hooks, on unless you turned \
         it off); on macOS and Linux each hook also has a switch of its own in the Hooks tab of \
         Cline's Rules panel",
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    /// One script per event of the adapter, named for this system, each running Cortex at its
    /// event, typed here by hand.
    #[test]
    fn each_event_gets_a_script() -> anyhow::Result<()> {
        let written = files("/opt/cortex/bin/cortex")?;
        let names: Vec<&str> = written.iter().map(|file| file.name.as_str()).collect();
        let events = [
            "TaskStart",
            "TaskResume",
            "UserPromptSubmit",
            "PostToolUse",
            "PreCompact",
            "TaskComplete",
        ];
        let expected: Vec<String> = events
            .iter()
            .map(|event| {
                if cfg!(windows) {
                    format!("{event}.ps1")
                } else {
                    (*event).to_owned()
                }
            })
            .collect();
        assert_eq!(names, expected);
        assert!(written.iter().all(|file| file.is_executable));
        let first = written.first().map(|file| file.text.as_str());
        let expected = if cfg!(windows) {
            "\u{feff}# Cortex's Cline hook for TaskStart, written by `cortex init`; `cortex uninstall` \
             removes it.\ntry { [Console]::InputEncoding = [Console]::OutputEncoding = \
             [Text.UTF8Encoding]::new($false) } catch {}\n$OutputEncoding = \
             [Text.UTF8Encoding]::new($false)\n[Console]::In.ReadToEnd() | \
             /opt/cortex/bin/cortex hook --agent cline --event TaskStart\n"
        } else {
            "#!/bin/sh\n# Cortex's Cline hook for TaskStart, written by `cortex init`; `cortex \
             uninstall` removes it.\nexec /opt/cortex/bin/cortex hook --agent cline --event \
             TaskStart\n"
        };
        assert_eq!(first, Some(expected));
        Ok(())
    }

    /// A path with a space is quoted for the system's shell: in single quotes for a POSIX shell,
    /// behind PowerShell's call operator on Windows.
    #[test]
    fn a_path_with_a_space_is_quoted() -> anyhow::Result<()> {
        let written = script(r"C:\Users\Ann Lee\cortex.exe", "TaskStart")?;
        let last = written.text.lines().last();
        let expected = if cfg!(windows) {
            r"[Console]::In.ReadToEnd() | & 'C:\Users\Ann Lee\cortex.exe' hook --agent cline --event TaskStart"
        } else {
            "exec 'C:/Users/Ann Lee/cortex.exe' hook --agent cline --event TaskStart"
        };
        assert_eq!(last, Some(expected));
        Ok(())
    }
}
