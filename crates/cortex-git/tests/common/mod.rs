//! Repositories the tests make with the `git` program, each in a temporary folder of its own, and
//! the commits and changes they expect to read. Git reads no configuration but the repository's
//! own, and every commit is made by one author at the time its test gives, so each test knows its
//! history exactly.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cortex_core::RepoPath;
use cortex_git::{CliHistory, GixHistory, HistorySource, Mined, MinedCommit, PathChange, Window};
use tempfile::TempDir;

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// 2026-01-01T00:00:00Z in seconds since the Unix epoch: day 0 of every history here.
pub const DAY_0_S: i64 = 1_767_225_600;

/// The seconds in a day: a commit made on day `n` is made at `DAY_0_S + n * DAY_S`.
pub const DAY_S: i64 = 86_400;

/// A repository made for one test.
pub struct Repo {
    /// The folder holding the work tree (`repo`) and the empty home folder git runs with
    /// (`home`).
    folder: TempDir,
}

impl Repo {
    /// A new repository on a branch named `main`, with no commit yet.
    pub fn new() -> TestResult<Self> {
        let repo = Self::empty()?;
        std::fs::create_dir(repo.root())?;
        repo.git(&["init", "-q", "-b", "main"])?;
        // Files are stored as the tests write them on every platform, and nothing signs.
        repo.git(&["config", "core.autocrlf", "false"])?;
        repo.git(&["config", "commit.gpgsign", "false"])?;
        // The store a test opens lives in `.cortex/`, as Cortex's own does, and git leaves it out as
        // it would with the `.gitignore` lines `cortex init` writes: a commit never reads the
        // store's files while they are open (on Windows SQLite's `-shm` file cannot be read then).
        std::fs::write(repo.root().join(".git/info/exclude"), ".cortex/\n")?;
        Ok(repo)
    }

    /// A clone of `origin` made with `options` (such as `--depth 2`), through git's transport as
    /// `--no-local` asks: git honours a depth or a filter for a repository on the same machine
    /// only that way.
    pub fn clone_of(origin: &Repo, options: &[&str]) -> TestResult<Self> {
        let clone = Self::empty()?;
        let source = origin.root();
        let source = source
            .to_str()
            .ok_or("a temporary folder that is not UTF-8")?;
        let mut args = vec!["clone", "-q", "--no-local"];
        args.extend_from_slice(options);
        args.extend([source, "repo"]);
        run_git(clone.folder.path(), &clone.home(), &args, None, None)?;
        Ok(clone)
    }

    /// A temporary folder with an empty home folder and no repository yet.
    fn empty() -> TestResult<Self> {
        let folder = tempfile::tempdir()?;
        std::fs::create_dir(folder.path().join("home"))?;
        Ok(Self { folder })
    }

    /// The top of the work tree.
    pub fn root(&self) -> PathBuf {
        self.folder.path().join("repo")
    }

    /// The home folder git runs with, empty unless a test puts something there.
    pub fn home(&self) -> PathBuf {
        self.folder.path().join("home")
    }

    /// Writes `text` to the file at `path` in the work tree, making its folders.
    pub fn write(&self, path: &str, text: &str) -> TestResult {
        let file = self.root().join(path);
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(file, text)?;
        Ok(())
    }

    /// Gives the file at `path` in the work tree a modification time of 2001, unlike the one git's
    /// index holds for it, so that git reads the file to tell whether it changed.
    pub fn backdate(&self, path: &str) -> TestResult {
        let file = std::fs::File::options()
            .write(true)
            .open(self.root().join(path))?;
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))?;
        Ok(())
    }

    /// Deletes the file at `path` in the work tree.
    pub fn remove(&self, path: &str) -> TestResult {
        std::fs::remove_file(self.root().join(path))?;
        Ok(())
    }

    /// Moves the file at `from` to `to` in the work tree, with its last line replaced by
    /// `last_line` when one is given.
    pub fn move_file(&self, from: &str, to: &str, last_line: Option<&str>) -> TestResult {
        let mut text = std::fs::read_to_string(self.root().join(from))?;
        if let Some(last_line) = last_line {
            let kept = text
                .trim_end_matches('\n')
                .rfind('\n')
                .map_or(0, |end| end + 1);
            text.truncate(kept);
            text.push_str(last_line);
            text.push('\n');
        }
        self.remove(from)?;
        self.write(to, &text)
    }

    /// Commits every change in the work tree on `day`, returning the new commit's id.
    pub fn commit(&self, day: i64) -> TestResult<String> {
        self.git(&["add", "-A"])?;
        self.git_on(day, &["commit", "-q", "-m", &format!("day {day}")])?;
        self.git(&["rev-parse", "HEAD"])
    }

    /// Deletes the object `id` from the repository's store of loose objects, where git keeps what
    /// a test commits.
    pub fn remove_object(&self, id: &str) -> TestResult {
        let (folder, name) = id.split_at_checked(2).ok_or("an object id too short")?;
        let file = self
            .root()
            .join(".git")
            .join("objects")
            .join(folder)
            .join(name);
        // Git writes its objects read-only, which Windows will not delete.
        let mut permissions = std::fs::metadata(&file)?.permissions();
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "a test's own temporary file, deleted at once"
        )]
        permissions.set_readonly(false);
        std::fs::set_permissions(&file, permissions)?;
        std::fs::remove_file(file)?;
        Ok(())
    }

    /// Runs git with `args` in the work tree, returning what it printed without the final line
    /// break.
    pub fn git(&self, args: &[&str]) -> TestResult<String> {
        run_git(&self.root(), &self.home(), args, None, None)
    }

    /// Runs git as [`Repo::git`] does, with the author's and committer's time set to `day`.
    pub fn git_on(&self, day: i64, args: &[&str]) -> TestResult<String> {
        run_git(&self.root(), &self.home(), args, Some(day), None)
    }

    /// Runs git as [`Repo::git_on`] does, with `input` on its stdin.
    pub fn git_fed(&self, day: i64, args: &[&str], input: &[u8]) -> TestResult<String> {
        run_git(&self.root(), &self.home(), args, Some(day), Some(input))
    }
}

/// Runs git with `args` in `folder`, any folder of a test's own, as [`Repo::git`] runs it with
/// `home` as the home folder.
pub fn git_in(folder: &Path, home: &Path, args: &[&str]) -> TestResult<String> {
    run_git(folder, home, args, None, None)
}

/// The major and minor version of the git on the `PATH`, from `git version`.
pub fn git_version() -> TestResult<(u32, u32)> {
    let output = Command::new("git").arg("version").output()?;
    let text = String::from_utf8(output.stdout)?;
    let number = text
        .split_whitespace()
        .nth(2)
        .ok_or("git version printed no number")?;
    let mut parts = number.split('.').map(str::parse::<u32>);
    let major = parts.next().ok_or("no major version")??;
    let minor = parts.next().ok_or("no minor version")??;
    Ok((major, minor))
}

/// Text of `count` numbered lines starting with `word`: one file's contents, which a file made
/// with a different word shares no line with.
pub fn lines(word: &str, count: usize) -> String {
    (1..=count).map(|line| format!("{word} {line}\n")).collect()
}

/// Both sources of the history of the repository at `root`, each with its name: what `gix` reads
/// and what the `git` program lists. A test's expectations hold for both unless it says otherwise.
pub fn sources(root: &Path) -> TestResult<Vec<(&'static str, Box<dyn HistorySource>)>> {
    Ok(vec![
        ("gix", Box::new(GixHistory::open(root)?)),
        ("git", Box::new(CliHistory::open(root)?)),
    ])
}

/// What each source reads of the repository at `root` within `window`, with the source's name.
pub fn read_both(root: &Path, window: &Window) -> TestResult<Vec<(&'static str, Mined)>> {
    sources(root)?
        .into_iter()
        .map(|(name, history)| Ok((name, history.read(window)?)))
        .collect()
}

/// A window holding every commit of these histories, with a mass commit cut at 30 files.
pub fn every_commit() -> Window {
    Window {
        max_commits: 1000,
        oldest_s: 0,
        max_files: 30,
        stop_at: None,
        from: None,
    }
}

/// A change of the file at `path` that is not a rename.
pub fn changed(path: &str) -> TestResult<PathChange> {
    Ok(PathChange {
        path: RepoPath::new(path)?,
        renamed_from: None,
    })
}

/// A rename of the file at `from` to `path`.
pub fn renamed(from: &str, path: &str) -> TestResult<PathChange> {
    Ok(PathChange {
        path: RepoPath::new(path)?,
        renamed_from: Some(RepoPath::new(from)?),
    })
}

/// The commit `id`, made on `day`, with `changes`.
pub fn commit(id: &str, day: i64, changes: Option<Vec<PathChange>>) -> MinedCommit {
    MinedCommit {
        id: id.to_owned(),
        time_s: DAY_0_S + day * DAY_S,
        changes,
    }
}

/// Commits `a.txt` on `day` with text of that day's own, returning the commit's id.
pub fn commit_day(repo: &Repo, day: i64) -> TestResult<String> {
    repo.write("a.txt", &lines(&format!("day {day}"), 3))?;
    repo.commit(day)
}

/// Runs git with `args` in `folder` with `home` as the home folder, reading no configuration but
/// the repository's own and looking for no repository above the system's temporary folder; with
/// `day`, every time it records is that day's; with `input`, that on its stdin.
///
/// # Errors
///
/// Git cannot start or ends with a failure (its message is the error).
fn run_git(
    folder: &Path,
    home: &Path,
    args: &[&str],
    day: Option<i64>,
    input: Option<&[u8]>,
) -> TestResult<String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(folder)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CEILING_DIRECTORIES", std::env::temp_dir())
        .env("GIT_AUTHOR_NAME", "Cortex Test")
        .env("GIT_AUTHOR_EMAIL", "test")
        .env("GIT_COMMITTER_NAME", "Cortex Test")
        .env("GIT_COMMITTER_EMAIL", "test")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "XDG_CONFIG_HOME",
        "HOMEDRIVE",
        "HOMEPATH",
    ] {
        command.env_remove(name);
    }
    if let Some(day) = day {
        // Git's own time format: seconds since the Unix epoch and the zone.
        let time = format!("{} +0000", DAY_0_S + day * DAY_S);
        command
            .env("GIT_AUTHOR_DATE", &time)
            .env("GIT_COMMITTER_DATE", &time);
    }
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.unwrap_or_default())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {args:?} failed: {message}").into());
    }
    let printed = String::from_utf8(output.stdout)?;
    Ok(printed.trim_end_matches(['\n', '\r']).to_owned())
}
