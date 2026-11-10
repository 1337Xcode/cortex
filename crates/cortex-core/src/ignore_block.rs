//! The block of lines `cortex init` adds at the end of a repository's `.gitignore`, so that git
//! leaves out what Cortex writes there: a comment naming Cortex, then some of the lines below. Init
//! writes it, uninstall takes it out again, and a look at the work tree knows a `.gitignore` whose
//! only change is this block as Cortex's own work, not the user's.

/// The comment that opens the block.
pub const IGNORE_BLOCK_HEADER: &str = "# Cortex";

/// The line that leaves out Cortex's folder.
pub const IGNORE_CORTEX_FOLDER: &str = ".cortex/";

/// The lines that leave out Cortex's folder but for the folder shared with the team, which is
/// committed.
pub const IGNORE_ALL_BUT_SHARED: [&str; 2] = [".cortex/*", "!.cortex/shared/"];

/// The line that leaves out Claude Code's local settings, where a project's Cortex hooks are.
pub const IGNORE_LOCAL_SETTINGS: &str = ".claude/settings.local.json";

/// The lines that leave out each agent's file of a repository's that init writes hooks to: the
/// agent reads it and git does not share it, since a hook names this machine's program.
pub const IGNORE_LOCAL_FILES: [&str; 5] = [
    IGNORE_LOCAL_SETTINGS,
    ".devin/config.local.json",
    ".augment/settings.local.json",
    ".github/hooks/cortex.json",
    ".kiro/hooks/cortex.json",
];

/// `line` without its line ending, `\n` or `\r\n`.
fn bare(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

/// Whether `line`, without its line ending, is one the block holds after its comment.
fn is_block_line(line: &str) -> bool {
    let line = bare(line);
    line == IGNORE_CORTEX_FOLDER
        || IGNORE_ALL_BUT_SHARED.contains(&line)
        || IGNORE_LOCAL_FILES.contains(&line)
}

/// `text`, a `.gitignore`, without the block init adds at its end: the text before the last line
/// that is the block's comment, when one or more lines follow it and each is one the block holds;
/// `None` when its end is no such block, as when the user wrote after it.
///
/// ```
/// use cortex_core::without_ignore_block;
/// assert_eq!(without_ignore_block("target/\n# Cortex\n.cortex/\n"), Some("target/\n"));
/// assert_eq!(without_ignore_block("target/\n# Cortex\n.cortex/\n*.log\n"), None);
/// ```
#[must_use]
pub fn without_ignore_block(text: &str) -> Option<&str> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let header = lines
        .iter()
        .rposition(|line| bare(line) == IGNORE_BLOCK_HEADER)?;
    let after = lines.get(header + 1..)?;
    if after.is_empty() || !after.iter().all(|line| is_block_line(line)) {
        return None;
    }
    let before: usize = lines.iter().take(header).map(|line| line.len()).sum();
    text.get(..before)
}

#[cfg(test)]
mod tests {
    use super::without_ignore_block;

    /// The block taken off whatever came before it, in either line ending, with any of its lines
    /// in any order; a `.gitignore` that is the block alone leaves nothing. Not a block: the user's
    /// line after it, the comment alone, a line it never holds, no comment at all.
    #[test]
    fn the_block_is_taken_off_its_end() {
        assert_eq!(
            without_ignore_block("target/\n# Cortex\n.cortex/\n.claude/settings.local.json\n"),
            Some("target/\n")
        );
        assert_eq!(
            without_ignore_block("target/\r\n# Cortex\r\n.cortex/*\r\n!.cortex/shared/\r\n"),
            Some("target/\r\n")
        );
        assert_eq!(without_ignore_block("# Cortex\n.cortex/"), Some(""));
        assert_eq!(
            without_ignore_block("# Cortex\nold\n# Cortex\n.cortex/\n"),
            Some("# Cortex\nold\n")
        );
        assert_eq!(
            without_ignore_block("target/\n# Cortex\n.cortex/\n*.log\n"),
            None
        );
        assert_eq!(without_ignore_block("target/\n# Cortex\n"), None);
        assert_eq!(without_ignore_block("# Cortex\nnode_modules/\n"), None);
        assert_eq!(without_ignore_block("target/\n.cortex/\n"), None);
        assert_eq!(without_ignore_block(""), None);
    }
}
