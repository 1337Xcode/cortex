//! The payloads the history benchmarks send beyond the recorded ones as they are: a read of a
//! file whole, and an edit of one line, each as Claude Code reports it, moved to the benchmark's
//! repository and session.

use cortex_core::canonical_root;
use serde_json::Value;

use super::HistoryBench;
use crate::common::TestResult;

/// An edit as the agent reports it: the file, its text before, and the line replaced and its
/// replacement.
pub(super) struct Edit<'a> {
    /// The repository path.
    pub(super) path: &'a str,
    /// The whole file before the edit.
    pub(super) before: &'a str,
    /// The line replaced and the line put in its place.
    pub(super) lines: (&'a str, &'a str),
}

impl HistoryBench {
    /// The recorded read, of the file at `path` whole, as `session`'s.
    #[allow(
        clippy::indexing_slicing,
        reason = "the payload edited here is a JSON object, where indexing a missing key adds it"
    )]
    pub(super) fn read_of(&self, session: &str, path: &str) -> TestResult<Value> {
        let mut value = self.recorded("post-tool-use-read.json", session)?;
        let native = self.native(path)?;
        let text = std::fs::read_to_string(self.root().join(path))?;
        let lines = text.lines().count();
        value["tool_input"]["file_path"] = native.clone().into();
        let file = &mut value["tool_response"]["file"];
        file["filePath"] = native.into();
        file["content"] = text.into();
        file["numLines"] = lines.into();
        file["totalLines"] = lines.into();
        Ok(value)
    }

    /// The recorded edit, made `edit`, as `session`'s: one hunk of one line.
    #[allow(
        clippy::indexing_slicing,
        reason = "the payload edited here is a JSON object, where indexing a missing key adds it"
    )]
    pub(super) fn edit_of(&self, session: &str, edit: &Edit<'_>) -> TestResult<Value> {
        let (old, new) = edit.lines;
        let line = edit
            .before
            .lines()
            .position(|line| line == old)
            .ok_or("the line edited")?
            + 1;
        let native = self.native(edit.path)?;
        let mut value = self.recorded("post-tool-use-edit.json", session)?;
        let input = &mut value["tool_input"];
        input["file_path"] = native.clone().into();
        input["old_string"] = old.into();
        input["new_string"] = new.into();
        let response = &mut value["tool_response"];
        response["filePath"] = native.into();
        response["oldString"] = old.into();
        response["newString"] = new.into();
        response["originalFile"] = edit.before.into();
        response["structuredPatch"] = serde_json::json!([{
            "oldStart": line,
            "oldLines": 1,
            "newStart": line,
            "newLines": 1,
            "lines": [format!("-{old}"), format!("+{new}")],
        }]);
        Ok(value)
    }

    /// The file at `path` as an agent names it: an absolute path in this platform's form.
    fn native(&self, path: &str) -> TestResult<String> {
        let root = canonical_root(self.root())?;
        let full = path.split('/').fold(root, |folder, name| folder.join(name));
        Ok(full.to_string_lossy().into_owned())
    }
}
