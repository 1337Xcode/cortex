//! The history read by running the `git` program: the fallback when `gix` cannot read a
//! repository, and in the tests the reference `gix`'s reading is checked against. Git is the
//! program the `PATH` names outside the repository; it runs in the repository's top, reading no
//! configuration but the repository's own, looking for no repository above it and fetching
//! nothing a partial clone left out, with every option that shapes its answer on its command line,
//! for a bounded time and answer.
//!
//! Git's own rename search decides renames here, so a file over 1 MiB moved with an edit is a
//! rename, which `gix` takes as a deletion and an addition; elsewhere the two read the same.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use cortex_core::{Captured, GIT, canonical_root, capture, find_program};

use crate::git_log::{self, FORMAT};
use crate::history::{GitError, HistorySource, Mined, MinedCommit, PathChange, Window};

/// How long git has to answer a question about HEAD, the configuration or a file in the git
/// folder.
const ASK_WITHIN: Duration = Duration::from_secs(10);

/// How long git has to list the history a window covers: 20,000 commits of a large repository
/// take it tens of seconds.
const LOG_WITHIN: Duration = Duration::from_secs(300);

/// The most of a short answer read: an id, a path, a few settings.
const MAX_ANSWER_BYTES: u64 = 64 * 1024;

/// The most of the history's listing read; a listing cut short is not read at all.
const MAX_LOG_BYTES: u64 = 256 * 1024 * 1024;

/// The most of the shallow clone's list of cut commits read: 25,000 ids.
const MAX_SHALLOW_BYTES: u64 = 1024 * 1024;

/// The name of the git folder at the top of a work tree.
const DOT_GIT: &str = ".git";

/// The exit code of `git config --get` and `git rev-parse --verify --quiet` for a value or a
/// revision that is not there.
const ABSENT: i32 = 1;

/// A repository's history read by running git.
pub struct CliHistory {
    /// The git program.
    git: PathBuf,
    /// The repository's top, in its canonical form.
    root: PathBuf,
}

impl CliHistory {
    /// The history of the git repository whose work tree's top is `root`, read with the git the
    /// `PATH` names outside it.
    ///
    /// # Errors
    ///
    /// [`GitError::NoRepository`] when `root` has no `.git`; [`GitError::Open`] when git is not on
    /// the `PATH`, does not answer, or names another folder as the work tree's top.
    pub fn open(root: &Path) -> Result<Self, GitError> {
        if !root.join(DOT_GIT).exists() {
            return Err(GitError::NoRepository(root.to_path_buf()));
        }
        let root = canonical_root(root).map_err(|error| GitError::Open(error.to_string()))?;
        let git = find_program(GIT, &root)
            .ok_or_else(|| GitError::Open("git is not on the PATH".to_owned()))?;
        let history = Self { git, root };
        let top = history
            .ask(&["rev-parse", "--show-toplevel"])
            .map_err(GitError::Open)?
            .ok_or_else(|| GitError::Open("git names no work tree".to_owned()))?;
        let is_root =
            canonical_root(Path::new(top.trim_end())).is_ok_and(|top| top == history.root);
        if !is_root {
            return Err(GitError::Open(format!(
                "its work tree is not {}",
                history.root.display()
            )));
        }
        Ok(history)
    }

    /// Git, run in the repository's top with `args`, as the module says.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.git);
        command
            .arg("--no-pager")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
            // A partial clone would otherwise fetch what it left out from its remote.
            .env("GIT_NO_LAZY_FETCH", "1");
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        ] {
            command.env_remove(name);
        }
        if let Some(parent) = self.root.parent() {
            command.env("GIT_CEILING_DIRECTORIES", parent);
        }
        command
    }

    /// Git's short answer to `args` as text; `None` when git says the thing asked for is not
    /// there.
    ///
    /// # Errors
    ///
    /// Git does not answer in time, fails, or answers more than a short answer or other than text.
    fn ask(&self, args: &[&str]) -> Result<Option<String>, String> {
        let answer = run(&mut self.command(args), ASK_WITHIN, MAX_ANSWER_BYTES)?;
        if answer.status.code() == Some(ABSENT) && answer.stdout.is_empty() {
            return Ok(None);
        }
        if !answer.status.success() {
            return Err(format!("git ended with {}", answer.status));
        }
        String::from_utf8(answer.stdout)
            .map(Some)
            .map_err(|_| "git's answer is not UTF-8 text".to_owned())
    }
}

impl CliHistory {
    /// Whether the repository is a partial clone, as git tells one: `extensions.partialClone`
    /// set, or `promisor` set on a remote. A `promisor` value that is not a false one counts as
    /// set, as the `gix` source counts it.
    ///
    /// # Errors
    ///
    /// Git does not answer.
    fn is_partial(&self) -> Result<bool, GitError> {
        let ask = |args: &[&str]| self.ask(args).map_err(GitError::Open);
        if ask(&["config", "--get", "extensions.partialClone"])?.is_some() {
            return Ok(true);
        }
        let promisors = ask(&["config", "--get-regexp", r"^remote\..*\.promisor$"])?;
        Ok(promisors.is_some_and(|lines| {
            lines.lines().any(|line| {
                let value = line.split_once(' ').map_or("", |(_, value)| value);
                !["false", "no", "off", "0"]
                    .iter()
                    .any(|word| value.eq_ignore_ascii_case(word))
            })
        }))
    }

    /// The commits a shallow clone cut, whose parents it does not hold; none in a full clone.
    ///
    /// # Errors
    ///
    /// Git does not say where the list is, or it cannot be read.
    fn shallow_cut(&self) -> Result<Vec<String>, GitError> {
        let walk =
            |message: String| GitError::Walk(format!("reading the shallow commits: {message}"));
        let path = self
            .ask(&["rev-parse", "--git-path", "shallow"])
            .map_err(walk)?
            .ok_or_else(|| walk("git names no file".to_owned()))?;
        let file = self.root.join(path.trim_end());
        let text = match std::fs::File::open(&file) {
            Ok(opened) => {
                let mut text = String::new();
                opened
                    .take(MAX_SHALLOW_BYTES)
                    .read_to_string(&mut text)
                    .map_err(|error| walk(error.to_string()))?;
                text
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(walk(error.to_string())),
        };
        Ok(text
            .lines()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect())
    }

    /// Git's listing of the commits `window` covers, from HEAD along the first-parent line: as
    /// many as the window takes and one more, so that the stop commit shows when it comes right
    /// after them (a shallow clone's cut commit, which is left out, is the last on the line). Git
    /// ends the listing at the first commit older than the window's oldest time, as the `gix`
    /// source ends its walk.
    ///
    /// # Errors
    ///
    /// Git does not end in time, fails, or lists more than [`MAX_LOG_BYTES`].
    fn log(&self, window: &Window, is_partial: bool) -> Result<Vec<u8>, GitError> {
        let max_count = format!("--max-count={}", window.max_commits.saturating_add(1));
        let max_age = format!("--max-age={}", window.oldest_s);
        // Git compares contents for renames as `gix` does: at most `max_files` squared pairs, and
        // in a partial clone none, which would need contents it does not hold.
        let renames = if is_partial {
            "--find-renames=100%"
        } else {
            "--find-renames=50%"
        };
        let limit = format!("-l{}", window.max_files.max(1));
        let args = [
            "-c",
            "log.showRoot=true",
            "log",
            "--first-parent",
            "--diff-merges=first-parent",
            renames,
            &limit,
            "--name-status",
            "-z",
            "--no-color",
            "--no-decorate",
            "--no-show-signature",
            "--no-relative",
            "--no-ext-diff",
            "--no-textconv",
            FORMAT,
            &max_count,
            &max_age,
            "HEAD",
            "--",
        ];
        let listing =
            run(&mut self.command(&args), LOG_WITHIN, MAX_LOG_BYTES).map_err(GitError::Walk)?;
        if !listing.status.success() {
            return Err(GitError::Walk(format!("git ended with {}", listing.status)));
        }
        if listing.is_cut {
            return Err(GitError::Walk(format!(
                "git listed more than {MAX_LOG_BYTES} bytes of history"
            )));
        }
        Ok(listing.stdout)
    }
}

impl HistorySource for CliHistory {
    fn head(&self) -> Result<Option<String>, GitError> {
        let id = self
            .ask(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
            .map_err(GitError::Head)?;
        Ok(id.map(|id| id.trim_end().to_owned()))
    }

    fn read(&self, window: &Window) -> Result<Mined, GitError> {
        let mut mined = Mined {
            is_partial: self.is_partial()?,
            ..Mined::default()
        };
        if self.head()?.is_none() {
            return Ok(mined);
        }
        let cut = self.shallow_cut()?;
        mined.is_shallow = !cut.is_empty();
        let listing = self.log(window, mined.is_partial)?;
        let listed = git_log::parse(&listing).map_err(|error| GitError::Walk(error.to_string()))?;
        for commit in listed {
            if window.stop_at.as_deref() == Some(commit.id.as_str()) {
                mined.is_stop_reached = true;
                break;
            }
            if mined.commits.len() >= window.max_commits {
                break;
            }
            if cut.contains(&commit.id) {
                continue;
            }
            let changes = (commit.changes.len() <= window.max_files).then(|| {
                let mut changes: Vec<PathChange> = commit
                    .changes
                    .iter()
                    .filter_map(|(path, from)| PathChange::from_git(path, from.as_deref()))
                    .collect();
                changes.sort();
                changes
            });
            mined.commits.push(MinedCommit {
                id: commit.id,
                time_s: commit.time_s,
                changes,
            });
        }
        Ok(mined)
    }
}

/// The file git reads as an empty configuration.
const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// What `command` printed, run for at most `within` and `max_bytes` of output; why not, as text.
fn run(command: &mut Command, within: Duration, max_bytes: u64) -> Result<Captured, String> {
    capture(command, within, max_bytes).map_err(|error| format!("git did not answer ({error})"))
}
