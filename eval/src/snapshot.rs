//! The files of a run's copy as they were at one moment, kept by git without touching the copy:
//! every file git does not ignore is written to a tree through an index of the snapshot's own, so
//! the copy's index, branch and history stay as the agent left them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, bail};

use crate::process::{run_logged, tail_of};

/// How long one git command of a snapshot may run.
const GIT_WITHIN: Duration = Duration::from_secs(120);

/// The files of a copy at one moment, as a git tree.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    /// The copy.
    repo: PathBuf,
    /// The folder the snapshot's git output goes to.
    folder: PathBuf,
    /// The tree's id.
    tree: String,
}

impl Snapshot {
    /// The files of `repo` now, git's output written to files named for `name` in `folder`.
    ///
    /// # Errors
    ///
    /// Git cannot run or fails, or names no tree.
    pub(crate) fn take(repo: &Path, folder: &Path, name: &str) -> anyhow::Result<Self> {
        let index = folder.join(format!("{name}.index"));
        let log = folder.join(format!("{name}.log"));
        let mut add = git(repo);
        add.env("GIT_INDEX_FILE", &index).args(["add", "--all"]);
        if !run_logged(add, &log, GIT_WITHIN)?.success() {
            bail!("git add for the snapshot failed:\n{}", tail_of(&log));
        }
        let mut write = git(repo);
        write.env("GIT_INDEX_FILE", &index).arg("write-tree");
        if !run_logged(write, &log, GIT_WITHIN)?.success() {
            bail!("git write-tree for the snapshot failed:\n{}", tail_of(&log));
        }
        let tree = std::fs::read_to_string(&log)
            .with_context(|| format!("reading {}", log.display()))?
            .trim()
            .to_owned();
        std::fs::remove_file(&index).with_context(|| format!("removing {}", index.display()))?;
        if tree.is_empty() || !tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("git write-tree named no tree: {tree}");
        }
        Ok(Self {
            repo: repo.to_path_buf(),
            folder: folder.to_path_buf(),
            tree,
        })
    }

    /// The text of the file at `path` (relative to the copy, with `/`) in the snapshot; none when
    /// the snapshot holds no such file.
    ///
    /// # Errors
    ///
    /// Git cannot run, or its output cannot be read.
    pub(crate) fn file(&self, path: &str) -> anyhow::Result<Option<String>> {
        let out = self.folder.join("snapshot-file.out");
        let mut show = git(&self.repo);
        show.args(["cat-file", "blob", &format!("{}:{path}", self.tree)]);
        if !run_logged(show, &out, GIT_WITHIN)?.success() {
            return Ok(None);
        }
        let bytes = std::fs::read(&out).with_context(|| format!("reading {}", out.display()))?;
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

/// Git, run in `repo`.
fn git(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(repo);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot keeps the files as they were, an untracked one too, and leaves the copy's own
    /// index alone: after the snapshot `a.txt` is changed and `b.txt` removed, yet the snapshot
    /// gives their first texts, nothing for a file it never held, and git still sees `b.txt` as
    /// untracked rather than staged.
    #[test]
    fn a_snapshot_keeps_the_files_as_they_were() -> anyhow::Result<()> {
        let repo = tempfile::tempdir()?;
        let folder = tempfile::tempdir()?;
        let run = |args: &[&str]| -> anyhow::Result<String> {
            let output = git(repo.path()).args(args).output()?;
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        };
        run(&["init", "-q"])?;
        std::fs::write(repo.path().join("a.txt"), "one\n")?;
        run(&["add", "a.txt"])?;
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t.invalid",
            "commit",
            "-q",
            "-m",
            "a",
        ])?;
        std::fs::write(repo.path().join("b.txt"), "two\n")?;
        let snapshot = Snapshot::take(repo.path(), folder.path(), "before")?;
        std::fs::write(repo.path().join("a.txt"), "changed\n")?;
        std::fs::remove_file(repo.path().join("b.txt"))?;
        assert_eq!(snapshot.file("a.txt")?.as_deref(), Some("one\n"));
        assert_eq!(snapshot.file("b.txt")?.as_deref(), Some("two\n"));
        assert_eq!(snapshot.file("c.txt")?, None);
        std::fs::write(repo.path().join("b.txt"), "two\n")?;
        assert_eq!(run(&["status", "--porcelain"])?, " M a.txt\n?? b.txt\n");
        assert!(!folder.path().join("before.index").exists());
        Ok(())
    }
}
