//! Files of Cortex's own that init writes whole into an agent's folder: the plugin Cortex
//! generates for an agent whose hooks are code (OpenCode, Kilo Code, Amp, OpenClaw), or a hook
//! script per event for an agent that runs one file per event (Cline). Such a file is never merged
//! into a file of the user's. A file already at its path is Cortex's when it is what this build
//! writes, when one of its first lines says it was written by `cortex init` ([`MARKER`], which
//! every script Cortex generates carries), when a run of init recorded writing exactly it there,
//! or when it is in a folder of Cortex's own (OpenClaw's `cortex/`) whose script carries the
//! marker, as a file that cannot carry one (a JSON manifest) an older build wrote is; any other
//! file there is the user's, and init leaves it as it is.

use std::path::Path;

use anyhow::Context as _;
use cortex_core::{ContentHash, MAX_SETTINGS_BYTES, read_text_file};

use super::user_path::UserPath;

/// The words one of the first lines of each script Cortex generates holds.
pub(crate) const MARKER: &str = "written by `cortex init`";

/// How many of a file's first lines are looked at for [`MARKER`]: a script's line saying what
/// runs it comes before it.
const MARKER_LINES: usize = 3;

/// Where an agent reads the files Cortex writes whole, and what they are.
#[derive(Debug)]
pub(crate) struct OwnFiles {
    /// The folder in a repository, relative to its root with `/` between folders; none when the
    /// agent reads no such folder of a repository's, or one that is not the repository's own: the
    /// user's folder is written then, whatever the scope.
    pub(crate) project: Option<&'static str>,
    /// The user's folder; none when the agent documents none on this system.
    pub(crate) user: Option<UserPath>,
    /// Whether the agent loads the files of both folders, so that Cortex's in both would run each
    /// event twice: init then refuses one folder while the other holds Cortex's files.
    pub(crate) loads_both: bool,
    /// What the agent calls the files, said by init and doctor: `plugin`, `hook`.
    pub(crate) what: &'static str,
    /// The files, for this program.
    pub(crate) files: fn(&str) -> anyhow::Result<Vec<OwnFile>>,
    /// What uninstall says once it took the files out, if anything is left for the user to undo.
    pub(crate) after_removal: Option<&'static str>,
}

/// One file of Cortex's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnFile {
    /// Its path in the agent's folder, with `/` between folders.
    pub(crate) name: String,
    /// What it holds.
    pub(crate) text: String,
    /// Whether the agent runs it as a program, so that it must be executable where files carry
    /// that bit.
    pub(crate) is_executable: bool,
}

/// What tells that a file at one of Cortex's paths is Cortex's, besides what it holds.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Signs<'s> {
    /// The BLAKE3 hashes, in hexadecimal, of what runs of init recorded writing at its path.
    pub(crate) recorded: &'s [String],
    /// Whether it is in a folder of Cortex's own that holds a script carrying [`MARKER`].
    pub(crate) is_in_marked_folder: bool,
}

/// Whether `existing`, a file at the path of `written`, what this build writes there, is Cortex's,
/// given the `signs` around it.
pub(crate) fn is_cortex_s(existing: &str, written: &OwnFile, signs: Signs<'_>) -> bool {
    existing == written.text
        || has_marker(existing)
        || signs.is_in_marked_folder
        || signs
            .recorded
            .contains(&ContentHash::of(existing.as_bytes()).to_string())
}

/// Whether one of the first lines of `text` carries [`MARKER`].
fn has_marker(text: &str) -> bool {
    text.lines()
        .take(MARKER_LINES)
        .any(|line| line.contains(MARKER))
}

/// The path of `file` in the agent's folder `folder`.
pub(crate) fn path_in(folder: &Path, file: &OwnFile) -> std::path::PathBuf {
    file.name
        .split('/')
        .fold(folder.to_path_buf(), |path, part| path.join(part))
}

/// Whether `file` is in a folder of Cortex's own inside the agent's folder `folder` (OpenClaw's
/// `cortex/`) in which another of `written` carries [`MARKER`]; false for a file at the top. A
/// file that cannot be read carries no marker.
pub(crate) fn is_in_marked_folder(folder: &Path, file: &OwnFile, written: &[OwnFile]) -> bool {
    let Some((inner, _)) = file.name.rsplit_once('/') else {
        return false;
    };
    written
        .iter()
        .filter(|other| other.name != file.name)
        .filter(|other| other.name.rsplit_once('/').map(|(of, _)| of) == Some(inner))
        .any(|other| {
            read_text_file(&path_in(folder, other), MAX_SETTINGS_BYTES)
                .ok()
                .flatten()
                .is_some_and(|text| has_marker(&text))
        })
}

/// Whether the folder `folder` holds one of `written`, at its path there, as Cortex's, from this
/// build or an older one.
///
/// # Errors
///
/// A file that cannot be read as small text.
pub(crate) fn holds_cortex_files(folder: &Path, written: &[OwnFile]) -> anyhow::Result<bool> {
    for file in written {
        let signs = Signs {
            recorded: &[],
            is_in_marked_folder: is_in_marked_folder(folder, file, written),
        };
        let text = read_text_file(&path_in(folder, file), MAX_SETTINGS_BYTES)?;
        if text.is_some_and(|text| is_cortex_s(&text, file, signs)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The files of the plugin `agent`'s adapter generates, running `program`.
///
/// # Errors
///
/// This build has no adapter for `agent`, or its adapter generates no plugin.
pub(crate) fn plugin(agent: &str, program: &str) -> anyhow::Result<Vec<OwnFile>> {
    let adapter =
        cortex_hook::adapter(agent).with_context(|| format!("finding the {agent} adapter"))?;
    let files: Vec<OwnFile> = adapter
        .plugin_files(program)
        .into_iter()
        .map(|file| OwnFile {
            name: file.path.to_owned(),
            text: file.text,
            is_executable: false,
        })
        .collect();
    anyhow::ensure!(!files.is_empty(), "the {agent} adapter writes no plugin");
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file is Cortex's when it is what is written, or when one of its first three lines says
    /// init wrote it, as an older build's plugin does; a file of the user's, or one whose fourth
    /// line says so, is not.
    #[test]
    fn a_file_is_cortex_s_by_its_text_or_its_marker() {
        let written = OwnFile {
            name: "cortex.js".to_owned(),
            text: "// Cortex's plugin for Amp, written by `cortex init`; ...\nnew\n".to_owned(),
            is_executable: false,
        };
        let none = Signs::default();
        assert!(is_cortex_s(&written.text, &written, none));
        assert!(is_cortex_s(
            "// Cortex's plugin for Amp, written by `cortex init`; ...\nold\n",
            &written,
            none
        ));
        assert!(is_cortex_s(
            "#!/bin/sh\n# Cortex's Cline hook, written by `cortex init`\nexec x\n",
            &written,
            none
        ));
        assert!(!is_cortex_s(
            "export default function () {}\n",
            &written,
            none
        ));
        assert!(!is_cortex_s(
            "a\nb\nc\n// written by `cortex init`\n",
            &written,
            none
        ));
    }

    /// A file that cannot carry the marker is Cortex's when a run of init recorded writing exactly
    /// it there, or when it is in Cortex's folder beside a script carrying the marker; neither
    /// sign makes any other text Cortex's.
    #[test]
    fn a_manifest_s_record_or_a_marked_folder_makes_a_file_cortex_s() -> anyhow::Result<()> {
        let older = "{\"id\": \"cortex\", \"version\": 1}\n";
        let written = OwnFile {
            name: "cortex/openclaw.plugin.json".to_owned(),
            text: "{\"id\": \"cortex\", \"version\": 2}\n".to_owned(),
            is_executable: false,
        };
        assert!(!is_cortex_s(older, &written, Signs::default()));
        let recorded = [ContentHash::of(older.as_bytes()).to_string()];
        let signs = Signs {
            recorded: &recorded,
            is_in_marked_folder: false,
        };
        assert!(is_cortex_s(older, &written, signs));
        assert!(!is_cortex_s("{\"id\": \"mine\"}\n", &written, signs));
        let folder = tempfile::tempdir()?;
        let script = OwnFile {
            name: "cortex/index.js".to_owned(),
            text: "// Cortex's plugin for OpenClaw, written by `cortex init`; ...\n".to_owned(),
            is_executable: false,
        };
        let both = [script.clone(), written.clone()];
        assert!(!is_in_marked_folder(folder.path(), &written, &both));
        std::fs::create_dir(folder.path().join("cortex"))?;
        std::fs::write(folder.path().join("cortex").join("index.js"), "// mine\n")?;
        assert!(!is_in_marked_folder(folder.path(), &written, &both));
        std::fs::write(folder.path().join("cortex").join("index.js"), &script.text)?;
        assert!(is_in_marked_folder(folder.path(), &written, &both));
        let marked = Signs {
            recorded: &[],
            is_in_marked_folder: true,
        };
        assert!(is_cortex_s(older, &written, marked));
        assert!(!is_in_marked_folder(
            folder.path(),
            &script,
            std::slice::from_ref(&script)
        ));
        Ok(())
    }

    /// Each plugin agent's files are its adapter's, running the program given.
    #[test]
    fn a_plugin_is_its_adapter_s() -> anyhow::Result<()> {
        let files = plugin("amp", "/opt/cortex")?;
        assert_eq!(files.len(), 1);
        assert_eq!(
            files.first().map(|file| file.name.as_str()),
            Some("cortex.js")
        );
        assert!(
            files
                .iter()
                .all(|file| file.text.contains("\"/opt/cortex\""))
        );
        assert!(plugin("claude", "/opt/cortex").is_err());
        Ok(())
    }
}
