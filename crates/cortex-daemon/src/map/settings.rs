//! What the map shows of the repository's configuration: the limits of the history Cortex reads,
//! said beside every number of commits so that a file whose commits all fell outside them does not
//! read as never committed; and the address that opens a file in the person's editor.

use std::fmt::Write as _;
use std::path::Path;

use cortex_core::Config;
use serde::Serialize;

/// The line an editor link opens a file at: the panel names a file, not a line in it.
const FIRST_LINE: &str = "1";

/// The history the co-change miner reads, as the configuration limits it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
// The names are the configuration keys' and the API's, where the shared `max` says what they are.
#[allow(clippy::struct_field_names)]
pub(crate) struct HistoryLimits {
    /// `git.max_commits`: the most commits it reads, newest first.
    pub(crate) max_commits: u64,
    /// `git.max_age_days`: the oldest commit it reads, in days.
    pub(crate) max_age_days: u64,
    /// `git.max_files_per_commit`: a commit that changed more files than this is left out.
    pub(crate) max_files_per_commit: u64,
}

/// The editor address of `ui.editor_url` and the repository's root it fills `{path}` from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorLink {
    /// The address, `{path}` and `{line}` to be filled in.
    template: String,
    /// The repository's root as a URL path: `/` separators, starting with `/`.
    root: String,
}

/// The configuration the map's answers carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapSettings {
    /// The history Cortex reads.
    pub(crate) history: HistoryLimits,
    /// How to open a file in the editor; none when `ui.editor_url` is empty or the root is not
    /// text.
    editor: Option<EditorLink>,
}

impl MapSettings {
    /// The map's settings from `config`, for the repository whose root is `root`.
    pub(crate) fn of(config: &Config, root: &Path) -> Self {
        let editor = root
            .to_str()
            .filter(|_| !config.ui.editor_url.is_empty())
            .map(|root| EditorLink {
                template: config.ui.editor_url.clone(),
                root: root_as_url_path(root),
            });
        Self {
            history: HistoryLimits {
                max_commits: config.git_max_commits,
                max_age_days: config.git_max_age_days,
                max_files_per_commit: config.git_max_files_per_commit,
            },
            editor,
        }
    }

    /// The address that opens the file at repository path `path` in the editor, at its first
    /// line; none when no editor address is set.
    pub(crate) fn editor_url(&self, path: &str) -> Option<String> {
        let editor = self.editor.as_ref()?;
        let full = format!("{}/{}", editor.root.trim_end_matches('/'), path);
        Some(
            editor
                .template
                .replace("{path}", &percent_encoded(&full))
                .replace("{line}", FIRST_LINE),
        )
    }
}

/// `root` as a URL path: Windows' verbatim prefix dropped, `\` turned to `/`, and a `/` before a
/// drive letter (`C:\repo` is `/C:/repo`, `/home/me/repo` stays as it is).
fn root_as_url_path(root: &str) -> String {
    let root = root
        .strip_prefix(r"\\?\")
        .unwrap_or(root)
        .replace('\\', "/");
    if root.starts_with('/') {
        root
    } else {
        format!("/{root}")
    }
}

/// `path` with every byte but letters, digits, `-`, `.`, `_`, `~`, `/` and `:` written as `%XX`,
/// so a space, `#`, `?` or `%` in a name cannot end or change the address.
fn percent_encoded(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            encoded.push(char::from(byte));
        } else {
            // Writing to a String cannot fail.
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The settings of a configuration whose editor address is `template`, for the root `root`.
    fn settings(template: &str, root: &str) -> MapSettings {
        let mut config = Config::default();
        config.ui.editor_url = template.to_owned();
        MapSettings::of(&config, Path::new(root))
    }

    /// The default address opens a file at its first line, its path a URL path: on Windows the
    /// drive letter after a `/` and backslashes turned, the verbatim prefix dropped; on Unix as it
    /// is. A space is `%20`, `#` `%23`, and `é` its two UTF-8 bytes `%C3%A9`.
    #[test]
    fn an_editor_link_is_the_address_filled_with_the_url_path() {
        let windows = settings("vscode://file{path}:{line}", r"C:\Users\me\repo");
        assert_eq!(
            windows.editor_url("src/a b#.rs").as_deref(),
            Some("vscode://file/C:/Users/me/repo/src/a%20b%23.rs:1")
        );
        let verbatim = settings("vscode://file{path}:{line}", r"\\?\C:\repo\");
        assert_eq!(
            verbatim.editor_url("main.rs").as_deref(),
            Some("vscode://file/C:/repo/main.rs:1")
        );
        let unix = settings("idea://open?file={path}&line={line}", "/home/me/repo");
        assert_eq!(
            unix.editor_url("src/é.rs").as_deref(),
            Some("idea://open?file=/home/me/repo/src/%C3%A9.rs&line=1")
        );
    }

    /// An empty `ui.editor_url` shows no editor link.
    #[test]
    fn an_empty_address_gives_no_link() {
        assert_eq!(settings("", "/home/me/repo").editor_url("main.rs"), None);
    }
}
