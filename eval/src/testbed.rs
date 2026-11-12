//! The testbeds tasks run in, as `eval/testbeds.toml` lists them: each a git repository at a pinned
//! commit, and how a run gets its own copy, cloned from the repository or copied from a folder
//! holding that commit, its `.git` left behind. A copy with no history is made a repository of one
//! commit, since Cortex reads the git history of what it works in. A task that needs history gets
//! a clone of the whole repository instead, cut so that the pinned commit is the newest thing in
//! it. A task's regression, a diff that breaks the copy, is applied and committed on top of that.

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
    /// A copy that `has_history` is a clone with the repository's whole history up to the pinned
    /// commit and nothing past it; `from` must then be a git repository holding the commit, not
    /// only its files. Otherwise the copy has no history: a copy of `from`'s files is made a
    /// repository of one commit, and a clone is a blobless one.
    ///
    /// # Errors
    ///
    /// A copy or a git command that fails, or, for a copy with history, a `from` that is not a git
    /// repository holding the commit.
    pub(crate) fn prepare(
        &self,
        from: Option<&Path>,
        to: &Path,
        has_history: bool,
    ) -> anyhow::Result<()> {
        match (from, has_history) {
            (Some(folder), false) => self.copy_files(folder, to),
            (Some(folder), true) => {
                self.check_history_source(folder)?;
                self.clone_with_history(&folder.to_string_lossy(), to)
            }
            (None, true) => self.clone_with_history(&self.url, to),
            (None, false) => {
                let target = to.to_string_lossy();
                git(
                    Path::new("."),
                    &["clone", "-q", "--filter=blob:none", &self.url, &target],
                )?;
                git(to, &["checkout", "-q", "--detach", &self.commit])
            }
        }
    }

    /// Refuses `folder` as the source of a copy with history unless it is a git repository holding
    /// the testbed's commit: a folder of files only has no history to keep.
    ///
    /// # Errors
    ///
    /// The folder has no `.git`, or its repository does not hold the commit.
    pub(crate) fn check_history_source(&self, folder: &Path) -> anyhow::Result<()> {
        if !folder.join(".git").exists() {
            bail!(
                "{} has no .git, so it cannot give the testbed {} its history; a task that needs \
                 history takes a full clone",
                folder.display(),
                self.name
            );
        }
        let wanted = format!("{}^{{commit}}", self.commit);
        git(folder, &["cat-file", "-e", &wanted]).with_context(|| {
            format!(
                "{} does not hold the commit {} of the testbed {}",
                folder.display(),
                self.commit,
                self.name
            )
        })
    }

    /// Makes `to` a repository of the files in `folder`, as one commit.
    fn copy_files(&self, folder: &Path, to: &Path) -> anyhow::Result<()> {
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

    /// Clones `source`, a repository's folder or url, in full into `to`, checks out the testbed's
    /// commit and cuts everything past it away. A blobless clone is not used: a blame would fetch
    /// the blobs it needs from the network.
    fn clone_with_history(&self, source: &str, to: &Path) -> anyhow::Result<()> {
        let target = to.to_string_lossy();
        git(
            Path::new("."),
            &["clone", "-q", "--no-checkout", source, &target],
        )
        .with_context(|| format!("cloning {source} to {}", to.display()))?;
        git(to, &["checkout", "-q", "--detach", &self.commit])?;
        cut_history_after(to)
    }
}

/// Leaves the clone in `repo`, checked out at the commit to keep, with that commit and its
/// ancestors as the only history, and nothing past it reachable or stored: every ref goes (the
/// branches and tags, which point past the commit), then the remote, every reflog entry, and the
/// objects that no ref or HEAD reaches. The result needs no network: a blame, a log and a
/// `cat-file` work on what is left and find nothing newer.
///
/// # Errors
///
/// A git command that fails, naming it and the repository.
fn cut_history_after(repo: &Path) -> anyhow::Result<()> {
    for name in git_text(repo, &["for-each-ref", "--format=%(refname)"])?.lines() {
        git(repo, &["update-ref", "-d", "--no-deref", name])?;
    }
    git(repo, &["remote", "remove", "origin"])?;
    git(repo, &["reflog", "expire", "--expire=now", "--all"])?;
    git(repo, &["gc", "-q", "--prune=now"])
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

/// The git command for `args` in `folder`, as a fixed author.
fn git_command(folder: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(folder)
        .env("GIT_AUTHOR_NAME", "Cortex Evaluation")
        .env("GIT_AUTHOR_EMAIL", "eval@cortex.invalid")
        .env("GIT_COMMITTER_NAME", "Cortex Evaluation")
        .env("GIT_COMMITTER_EMAIL", "eval@cortex.invalid");
    command
}

/// Runs git with `args` in `folder`, as a fixed author.
///
/// # Errors
///
/// Git cannot run or exits with an error.
fn git(folder: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = git_command(folder, args)
        .status()
        .with_context(|| format!("running git {args:?}"))?;
    if !status.success() {
        bail!("git {args:?} in {} failed: {status}", folder.display());
    }
    Ok(())
}

/// What git prints to stdout for `args` in `folder`, as a fixed author.
///
/// # Errors
///
/// Git cannot run, exits with an error, or prints what is not UTF-8.
fn git_text(folder: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = git_command(folder, args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    if !output.status.success() {
        bail!(
            "git {args:?} in {} failed: {}",
            folder.display(),
            output.status
        );
    }
    String::from_utf8(output.stdout)
        .with_context(|| format!("reading what git {args:?} printed in {}", folder.display()))
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
        bed.prepare(Some(from.path()), to.path(), false)?;
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
        assert_eq!(git_text(repo.path(), &["status", "--porcelain"])?, "");
        let log = git_text(repo.path(), &["log", "--format=%an <%ae> %s"])?;
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
        assert_eq!(git_text(repo.path(), &["status", "--porcelain"])?, "");
        let log = git_text(repo.path(), &["log", "--oneline"])?;
        assert_eq!(log.lines().count(), 1);
        Ok(())
    }

    /// A scripted repository of four commits, `a.txt` gaining a line in each: `first` (one line),
    /// `pin` (two), `later` (three) and `latest` (four), with a branch at `later` and a tag at
    /// `latest`, so that refs point past the pin. Holds the folder and the commits' ids.
    struct Scripted {
        /// The repository.
        folder: tempfile::TempDir,
        /// The commit `first`.
        first: String,
        /// The commit `pin`, the one a testbed is pinned at.
        pin: String,
        /// The commit `later`.
        later: String,
        /// The commit `latest`, the tip of `main`.
        latest: String,
    }

    /// Makes the scripted repository.
    fn scripted_repository() -> anyhow::Result<Scripted> {
        let folder = tempfile::tempdir()?;
        git(folder.path(), &["init", "-q", "-b", "main"])?;
        git(folder.path(), &["config", "core.autocrlf", "false"])?;
        let mut ids = Vec::new();
        for (subject, text) in [
            ("first", "one\n"),
            ("pin", "one\ntwo\n"),
            ("later", "one\ntwo\nthree\n"),
            ("latest", "one\ntwo\nthree\nfour\n"),
        ] {
            std::fs::write(folder.path().join("a.txt"), text)?;
            git(folder.path(), &["add", "--all"])?;
            let commit = ["-c", "commit.gpgsign=false", "commit", "-q", "-m", subject];
            git(folder.path(), &commit)?;
            ids.push(
                git_text(folder.path(), &["rev-parse", "HEAD"])?
                    .trim()
                    .to_owned(),
            );
        }
        let Ok([first, pin, later, latest]) = <[String; 4]>::try_from(ids) else {
            anyhow::bail!("four commits were made");
        };
        git(folder.path(), &["branch", "side", &later])?;
        git(folder.path(), &["tag", "v-latest"])?;
        Ok(Scripted {
            folder,
            first,
            pin,
            later,
            latest,
        })
    }

    /// A testbed pinned at `commit`, to be cloned from `url`.
    fn bed(commit: &str, url: &str) -> Testbed {
        Testbed {
            name: "t".to_owned(),
            url: url.to_owned(),
            commit: commit.to_owned(),
            description: String::new(),
        }
    }

    /// A copy that has history, made from a folder or from a url, holds the commits up to the pin
    /// and nothing else: `git log --all` lists `pin` and `first` and no more, HEAD is the pin, no
    /// ref, remote or file change is left, and the work tree is the pin's.
    #[test]
    fn a_copy_with_history_holds_the_commits_up_to_the_pin() -> anyhow::Result<()> {
        let source = scripted_repository()?;
        let url = source.folder.path().to_string_lossy().into_owned();
        let testbed = bed(&source.pin, &url);
        for from in [Some(source.folder.path()), None] {
            let to = tempfile::tempdir()?;
            testbed.prepare(from, to.path(), true)?;
            let log = git_text(to.path(), &["log", "--all", "--format=%s"])?;
            assert_eq!(log, "pin\nfirst\n");
            assert_eq!(
                git_text(to.path(), &["rev-parse", "HEAD"])?.trim(),
                source.pin
            );
            assert_eq!(git_text(to.path(), &["for-each-ref"])?, "");
            assert_eq!(git_text(to.path(), &["remote"])?, "");
            assert_eq!(git_text(to.path(), &["status", "--porcelain"])?, "");
            let text = std::fs::read_to_string(to.path().join("a.txt"))?;
            assert_eq!(text.replace("\r\n", "\n"), "one\ntwo\n");
        }
        Ok(())
    }

    /// Whether the repository in `repo` holds the object `id`.
    fn holds(repo: &Path, id: &str) -> anyhow::Result<bool> {
        Ok(git_command(repo, &["cat-file", "-e", id])
            .output()?
            .status
            .success())
    }

    /// A commit past the pin cannot be found in a copy that has history, though its branch and tag
    /// were in the repository it was cloned from; the pin and the commit before it can.
    #[test]
    fn a_commit_past_the_pin_cannot_be_found() -> anyhow::Result<()> {
        let source = scripted_repository()?;
        assert!(holds(source.folder.path(), &source.latest)?);
        let to = tempfile::tempdir()?;
        bed(&source.pin, "unused").prepare(Some(source.folder.path()), to.path(), true)?;
        assert!(holds(to.path(), &source.pin)?);
        assert!(holds(to.path(), &source.first)?);
        assert!(!holds(to.path(), &source.later)?);
        assert!(!holds(to.path(), &source.latest)?);
        Ok(())
    }

    /// `git blame` in a copy that has history names, for each line of the file, the commit that
    /// wrote it, with no remote to fetch from: line one is `first`'s, line two `pin`'s.
    #[test]
    fn a_blame_works_with_no_remote() -> anyhow::Result<()> {
        let source = scripted_repository()?;
        let to = tempfile::tempdir()?;
        bed(&source.pin, "unused").prepare(Some(source.folder.path()), to.path(), true)?;
        let blame = git_text(to.path(), &["blame", "--line-porcelain", "a.txt"])?;
        let writers: Vec<&str> = blame
            .lines()
            .filter_map(|line| line.split(' ').next())
            .filter(|word| word.len() == 40 && word.chars().all(|c| c.is_ascii_hexdigit()))
            .collect();
        assert_eq!(writers, [source.first.as_str(), source.pin.as_str()]);
        Ok(())
    }

    /// A copy that has no history is as it was: the files of the folder, its `.git` left behind,
    /// are one commit, whatever the folder's own history holds.
    #[test]
    fn a_copy_without_history_is_one_commit() -> anyhow::Result<()> {
        let source = scripted_repository()?;
        let to = tempfile::tempdir()?;
        let testbed = bed(&source.pin, "unused");
        testbed.prepare(Some(source.folder.path()), to.path(), false)?;
        let log = git_text(to.path(), &["log", "--all", "--format=%s"])?;
        assert_eq!(log, format!("t at {}\n", source.pin));
        let text = std::fs::read_to_string(to.path().join("a.txt"))?;
        assert_eq!(text, "one\ntwo\nthree\nfour\n");
        Ok(())
    }

    /// A folder of files, with no `.git`, cannot give a testbed its history, and a repository that
    /// lacks the pinned commit cannot either; each is refused naming the folder and why, before
    /// anything is cloned.
    #[test]
    fn a_source_without_the_history_is_refused() -> anyhow::Result<()> {
        let source = scripted_repository()?;
        let files = tempfile::tempdir()?;
        std::fs::write(files.path().join("a.txt"), "one\n")?;
        let to = tempfile::tempdir()?;
        let no_git = bed(&source.pin, "unused")
            .prepare(Some(files.path()), to.path(), true)
            .err()
            .map(|error| format!("{error:#}"))
            .unwrap_or_default();
        assert!(no_git.contains("has no .git"), "{no_git}");
        assert!(
            no_git.contains(&files.path().display().to_string()),
            "{no_git}"
        );
        let missing = "0123456789abcdef0123456789abcdef01234567";
        let no_commit = bed(missing, "unused")
            .prepare(Some(source.folder.path()), to.path(), true)
            .err()
            .map(|error| format!("{error:#}"))
            .unwrap_or_default();
        assert!(
            no_commit.contains("does not hold the commit"),
            "{no_commit}"
        );
        assert!(no_commit.contains(missing), "{no_commit}");
        assert_eq!(std::fs::read_dir(to.path())?.count(), 0);
        Ok(())
    }
}
