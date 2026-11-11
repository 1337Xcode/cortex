//! The text Cortex gives the agents it reaches through MCP alone, with no hooks it can use: the
//! model is told to call `cortex_where`, Cortex's MCP tool, first when a task needs code it has
//! not read yet, and Aider's model, which has no MCP either, to suggest running `cortex where`.
//! Where an agent reads every file of a rules folder, the text is a rules file of Cortex's own
//! there ([`rules_file`]). Where it reads only a file the user owns, or one other agents read too
//! (`AGENTS.md`, `CLAUDE.md`, `.rules`), init says the lines to add instead: Codex and Claude Code
//! read those files, get Cortex through hooks, and have no `cortex_where` tool to call.

use super::own::OwnFile;

/// The name of Cortex's rules file in an agent's rules folder.
const NAME: &str = "cortex.md";

/// The rules for an agent with Cortex's MCP server, as a string literal, so that an agent's told
/// text can be put together with it at compile time.
macro_rules! mcp_rules {
    () => {
        "# Cortex\n\nCortex keeps a map of the code of the repository you work in. When a task \
         needs code you\nhave not read yet and the `cortex_where` tool (Cortex's MCP server) is \
         available, call it first\nwith the task in one sentence: it names the files and line \
         spans the task most likely touches,\nand why.\n"
    };
}
pub(crate) use mcp_rules;

/// The rules for Aider, which has no MCP: its model suggests the command line, which Aider runs
/// once the user agrees.
macro_rules! aider_rules {
    () => {
        "# Cortex\n\nCortex keeps a map of the code of the repository you work in. When a task \
         needs files that\nare not in the chat yet, first suggest the shell command `cortex where \
         \"<the task in one sentence>\"`:\nit names the files and line spans the task most likely \
         touches, and why.\n"
    };
}
pub(crate) use aider_rules;

/// How an agent's rules file opens, before the rules.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Opening {
    /// YAML front matter holding these lines (the agent's keys that make the rules always apply),
    /// after a `description` saying the file is Cortex's.
    FrontMatter(&'static str),
    /// A Markdown comment saying the file is Cortex's.
    Comment,
}

/// Cortex's rules file for the agent whose name people write as `title`, opening as `opening`
/// says, its second line or its first carrying [`super::own::MARKER`], by which a later build
/// knows a file an older one wrote.
pub(crate) fn rules_file(title: &str, opening: Opening) -> OwnFile {
    let said = format!(
        "Cortex's rules for {title}, {}; `cortex uninstall` removes them",
        super::own::MARKER
    );
    let opening = match opening {
        Opening::FrontMatter(keys) => format!("---\ndescription: \"{said}\"\n{keys}\n---\n"),
        Opening::Comment => format!("<!-- {said}. -->\n"),
    };
    OwnFile {
        name: NAME.to_owned(),
        text: format!("{opening}\n{}", mcp_rules!()),
        is_executable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Continue's file, typed here by hand: front matter whose second line says whose the file
    /// is, its keys, then the rules.
    #[test]
    fn a_front_matter_file_is_typed_by_hand() {
        let expected = "---\n\
description: \"Cortex's rules for Continue, written by `cortex init`; `cortex uninstall` removes them\"\n\
name: Cortex\n\
alwaysApply: true\n\
---\n\
\n\
# Cortex\n\
\n\
Cortex keeps a map of the code of the repository you work in. When a task needs code you\n\
have not read yet and the `cortex_where` tool (Cortex's MCP server) is available, call it first\n\
with the task in one sentence: it names the files and line spans the task most likely touches,\n\
and why.\n";
        let file = rules_file(
            "Continue",
            Opening::FrontMatter("name: Cortex\nalwaysApply: true"),
        );
        assert_eq!(file.name, "cortex.md");
        assert_eq!(file.text, expected);
        assert!(!file.is_executable);
    }

    /// Roo Code's file opens with a comment saying whose it is, and an older build's file, with
    /// other rules after the same first line, is Cortex's by it.
    #[test]
    fn a_comment_file_carries_the_marker() {
        let file = rules_file("Roo Code", Opening::Comment);
        let first = "<!-- Cortex's rules for Roo Code, written by `cortex init`; `cortex uninstall` \
                     removes them. -->";
        assert_eq!(file.text.lines().next(), Some(first));
        let older = format!("{first}\n\nolder rules\n");
        assert!(super::super::own::is_cortex_s(
            &older,
            &file,
            super::super::own::Signs::default()
        ));
    }

    /// Aider's rules name the command line and no tool.
    #[test]
    fn aider_is_told_the_command_line() {
        let text = aider_rules!();
        assert!(text.contains("`cortex where \"<the task in one sentence>\"`"));
        assert!(!text.contains("cortex_where"));
    }
}
