//! The testbeds tasks run in, as `eval/testbeds.toml` lists them: each a git repository at a pinned
//! commit, and how a run gets its own copy, cloned from the repository or copied from a folder
//! holding that commit, its `.git` left behind. A copy with no history is made a repository of one
//! commit, since Cortex reads the git history of what it works in. A task's regression, a diff that
//! breaks the copy, is applied and committed on top of that.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use serde::Deserialize;

/// The testbeds file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Testbeds {
    /// Each testbed.
    testbed: Vec<Testbed>,
}

/// One testbed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Testbed {
    /// Its name, as tasks give it.
    pub(crate) name: String,
    /// The repository it is cloned from.
    pub(crate) url: String,
    /// The commit it is pinned at.
    pub(crate) commit: String,
    /// What it is, for the report.
    pub(crate) description: String,
}

/// The testbeds `text` lists.
///
/// # Errors
///
/// Text that is not a testbeds file, or a name listed twice.
pub(crate) fn parse(text: &str) -> anyhow::Result<Vec<Testbed>> {
    let file: Testbeds = toml::from_str(text).context("reading the testbeds")?;
    let mut names: Vec<&str> = file.testbed.iter().map(|bed| bed.name.as_str()).collect();
    names.sort_unstable();
    if let Some(pair) = names.windows(2).find(|pair| pair.first() == pair.get(1)) {
        bail!(
            "the testbed {} is listed twice",
            pair.first().copied().unwrap_or_default()
        );
    }
    Ok(file.testbed)
}

impl Testbed {
    /// Makes `to` a copy of the testbed: copied from `from`, a folder holding its commit, when
    /// given, else cloned and checked out at its commit.
    ///
    /// # Errors
    ///
    /// A copy or a git command that fails.
    pub(crate) fn prepare(&self, from: Option<&Path>, to: &Path) -> anyhow::Result<()> {
        match from {
            Some(folder) => {
                copy_folder(folder, to)
                    .with_context(|| format!("copying {} to {}", folder.display(), to.display()))?;
                git(to, &["init", "-q"])?;
                git(to, &["add", "--all"])?;
                let message = format!("{} at {}", self.name, self.commit);
                git(
                    to,
                    &["-c", "commit.gpgsign=false", "commit", "-q", "-m", &message],
                )
            }
            None => {
                let target = to.to_string_lossy();
                git(
                    Path::new("."),
                    &["clone", "-q", "--filter=blob:none", &self.url, &target],
                )?;
                git(to, &["checkout", "-q", "--detach", &self.commit])
            }
        }
    }
}

/// The message of the commit a regression becomes: history that was there before the run began.
const REGRESSION_MESSAGE: &str = "an earlier change";

/// Applies the unified diff in `patch` to the copy in `repo` and commits it, as a fixed author, so
/// that the broken state is history and not work the agent left uncommitted.
///
/// # Errors
///
/// The diff does not apply to the copy, or git cannot stage or commit it.
pub(crate) fn apply_regression(repo: &Path, patch: &Path) -> anyhow::Result<()> {
    let patch_arg = patch.to_string_lossy();
    git(repo, &["apply", &patch_arg]).with_context(|| {
        format!(
            "applying the regression {} to {}",
            patch.display(),
            repo.display()
        )
    })?;
    git(repo, &["add", "--all"])?;
    git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            REGRESSION_MESSAGE,
        ],
    )
}

/// Runs git with `args` in `folder`, as a fixed author.
///
/// # Errors
///
/// Git cannot run or exits with an error.
fn git(folder: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(folder)
        .env("GIT_AUTHOR_NAME", "Cortex Evaluation")
        .env("GIT_AUTHOR_EMAIL", "eval@cortex.invalid")
        .env("GIT_COMMITTER_NAME", "Cortex Evaluation")
        .env("GIT_COMMITTER_EMAIL", "eval@cortex.invalid")
        .status()
        .with_context(|| format!("running git {args:?}"))?;
    if !status.success() {
        bail!("git {args:?} in {} failed: {status}", folder.display());
    }
    Ok(())
}

/// The folders a copy leaves behind: git's own, and Cortex's, whose index and history would make
/// the copy anything but fresh.
const LEFT_BEHIND: [&str; 2] = [".git", ".cortex"];

/// Copies the files under `from` to `to`, folders made as needed, the folders in [`LEFT_BEHIND`]
/// left behind and symbolic links not followed.
///
/// # Errors
///
/// A file or folder that cannot be read or written.
fn copy_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        let is_left = LEFT_BEHIND.iter().any(|name| entry.file_name() == *name);
        if is_left || kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_folder(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A testbeds file as written, and one listing a name twice.
    #[test]
    fn testbeds_are_read_and_names_kept_apart() -> anyhow::Result<()> {
        let one = "[[testbed]]\nname = \"serena\"\nurl = \"https://example.invalid/serena\"\n\
                   commit = \"ede329a\"\ndescription = \"Python tools\"\n";
        let beds = parse(one)?;
        assert_eq!(beds.first().map(|bed| bed.commit.as_str()), Some("ede329a"));
        assert!(parse(&format!("{one}{one}")).is_err());
        Ok(())
    }

    /// A copy takes every file and folder but `.git` and `.cortex`, and becomes a repository of one
    /// commit.
    #[test]
    fn a_copy_leaves_git_behind_and_is_committed() -> anyhow::Result<()> {
        let from = tempfile::tempdir()?;
        std::fs::create_dir_all(from.path().join("src/deep"))?;
        std::fs::create_dir_all(from.path().join(".git"))?;
        std::fs::write(from.path().join("src/deep/a.py"), "a = 1\n")?;
        std::fs::write(from.path().join(".git/HEAD"), "not ours\n")?;
        std::fs::create_dir_all(from.path().join(".cortex"))?;
        std::fs::write(from.path().join(".cortex/cortex.db"), "an old index\n")?;
        let to = tempfile::tempdir()?;
        let bed = Testbed {
            name: "t".to_owned(),
            url: "https://example.invalid/t".to_owned(),
            commit: "abc".to_owned(),
            description: String::new(),
        };
        bed.prepare(Some(from.path()), to.path())?;
        assert_eq!(
            std::fs::read_to_string(to.path().join("src/deep/a.py"))?,
            "a = 1\n"
        );
        assert!(!to.path().join(".cortex").exists());
        let head = std::fs::read_to_string(to.path().join(".git/HEAD"))?;
        assert_ne!(head, "not ours\n");
        let log = Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(to.path())
            .output()?;
        assert_eq!(String::from_utf8(log.stdout)?.lines().count(), 1);
        Ok(())
    }

    /// A repository of one commit holding `a.txt` as `one`, its line endings left as written.
    fn tiny_repository() -> anyhow::Result<tempfile::TempDir> {
        let repo = tempfile::tempdir()?;
        git(repo.path(), &["init", "-q"])?;
        git(repo.path(), &["config", "core.autocrlf", "false"])?;
        std::fs::write(repo.path().join("a.txt"), "one\n")?;
        git(repo.path(), &["add", "--all"])?;
        git(
            repo.path(),
            &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "first"],
        )?;
        Ok(repo)
    }

    /// What git prints for `args` in `repo`.
    fn git_output(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
        let output = Command::new("git").args(args).current_dir(repo).output()?;
        Ok(String::from_utf8(output.stdout)?)
    }

    /// The diff changing `a.txt`'s one line from `one` to `two`, written by hand.
    const ONE_TO_TWO: &str = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n\
                              @@ -1 +1 @@\n-one\n+two\n";

    /// A regression changes the file and is committed under the fixed author with its fixed
    /// message, so the copy has two commits and a clean work tree.
    #[test]
    fn a_regression_is_applied_and_committed() -> anyhow::Result<()> {
        let repo = tiny_repository()?;
        let patches = tempfile::tempdir()?;
        let patch = patches.path().join("one-to-two.patch");
        std::fs::write(&patch, ONE_TO_TWO)?;
        apply_regression(repo.path(), &patch)?;
        let text = std::fs::read_to_string(repo.path().join("a.txt"))?;
        assert_eq!(text, "two\n");
        assert_eq!(git_output(repo.path(), &["status", "--porcelain"])?, "");
        let log = git_output(repo.path(), &["log", "--format=%an <%ae> %s"])?;
        assert_eq!(
            log,
            "Cortex Evaluation <eval@cortex.invalid> an earlier change\n\
             Cortex Evaluation <eval@cortex.invalid> first\n"
        );
        Ok(())
    }

    /// A diff that does not apply to the copy fails the run and leaves the copy as it was.
    #[test]
    fn a_regression_that_does_not_apply_is_refused() -> anyhow::Result<()> {
        let repo = tiny_repository()?;
        let patches = tempfile::tempdir()?;
        let patch = patches.path().join("wrong.patch");
        std::fs::write(&patch, ONE_TO_TWO.replace("-one", "-zero"))?;
        let error = apply_regression(repo.path(), &patch)
            .err()
            .map(|error| format!("{error:#}"))
            .unwrap_or_default();
        assert!(error.contains("applying the regression"), "{error}");
        let text = std::fs::read_to_string(repo.path().join("a.txt"))?;
        assert_eq!(text, "one\n");
        assert_eq!(git_output(repo.path(), &["status", "--porcelain"])?, "");
        let log = git_output(repo.path(), &["log", "--oneline"])?;
        assert_eq!(log.lines().count(), 1);
        Ok(())
    }
}
