//! The `git` program as the history source runs it: the one the `PATH` names outside the
//! repository, in the repository's top, with none of the caller's `GIT_` variables, no system or
//! global configuration, no repository looked for above the top, nothing fetched that a partial
//! clone left out, and a bounded time and answer. The work tree's status is read the same way but
//! with the user's own configuration, so that it lists what the user's own git lists.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use cortex_core::{Captured, GIT, capture, find_program};

use crate::history::has_repository;

/// What one run of git may take: the time it has and the output read.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bounds {
    /// How long git has, from its start to its end.
    pub(crate) within: Duration,
    /// The most of its output read; more is cut and says so.
    pub(crate) max_bytes: u64,
}

/// A short question to git about HEAD, the configuration or a file in the git folder: an id, a
/// path, a few settings.
const ANSWER: Bounds = Bounds {
    within: Duration::from_secs(10),
    max_bytes: 64 * 1024,
};

/// The listing of the history a window covers: 20,000 commits of a large repository take git tens
/// of seconds; a listing cut short is not read at all.
pub(crate) const LISTING: Bounds = Bounds {
    within: Duration::from_secs(300),
    max_bytes: 256 * 1024 * 1024,
};

/// The exit code of `git config --get` and `git rev-parse --verify --quiet` for a value or a
/// revision that is not there.
const ABSENT: i32 = 1;

/// The file git reads as an empty configuration.
pub(crate) const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// The first git that can be told not to fetch what a partial clone left out, with
/// `GIT_NO_LAZY_FETCH` or `--no-lazy-fetch`: 2.45, as its major and minor versions.
const NO_LAZY_FETCH_SINCE: (u32, u32) = (2, 45);

/// The keys of a repository's own configuration that decide how its work tree may be read: a
/// filter's, those that make it a partial clone, and the list of commits blame skips, which git
/// opens when it starts, wherever the key points.
const OWN_CONFIG_KEYS: &str =
    r"^filter\.|^extensions\.partialclone$|^remote\..*\.promisor$|^blame\.ignorerevsfile$";

/// The list of commits blame skips that a repository keeps in its top folder, which is the one
/// list a blame may use.
pub(crate) const IGNORE_REVS_FILE: &str = ".git-blame-ignore-revs";

/// What a repository's own configuration says of `blame.ignoreRevsFile`, a file git opens before
/// it blames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum IgnoreList {
    /// The key is not set.
    #[default]
    Unset,
    /// Every value of the key is [`IGNORE_REVS_FILE`].
    Top,
    /// A value names another file, which may be anywhere, a network share included.
    Other,
}

/// What a repository's own configuration, with the files it includes, says that decides how
/// Cortex may run git there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OwnConfig {
    /// It names a filter (any `filter.<driver>.<key>`), whose command git runs on a file the
    /// attributes give it to when it reads the file.
    pub(crate) names_filter: bool,
    /// It is a partial clone: `extensions.partialClone` is set, or `promisor` is true on a
    /// remote, so git fetches from the remote what the clone left out unless told not to.
    pub(crate) is_partial: bool,
    /// What it says of the list of commits blame skips.
    pub(crate) ignore_list: IgnoreList,
}

/// Why a repository whose own configuration says it is as [`OwnConfig`] says may not be read with
/// git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The configuration names a filter, whose command git would run on a file it reads.
    Filter,
    /// The repository is a partial clone and this git cannot be told not to fetch what it left
    /// out.
    WouldFetch,
}

impl OwnConfig {
    /// Why git may not read the repository, or `None` when it may; `can_refuse_fetching` answers
    /// whether this git can be told not to fetch, and is asked only of a partial clone.
    ///
    /// # Errors
    ///
    /// `can_refuse_fetching` says it cannot tell.
    pub(crate) fn refusal(
        self,
        can_refuse_fetching: impl FnOnce() -> Result<bool, String>,
    ) -> Result<Option<Refusal>, String> {
        if self.names_filter {
            return Ok(Some(Refusal::Filter));
        }
        // INVARIANT: local-only. Git 2.45 and later read `GIT_NO_LAZY_FETCH`, which every git
        // Cortex runs is given; an older git in a partial clone would fetch what a read needs.
        if self.is_partial && !can_refuse_fetching()? {
            return Ok(Some(Refusal::WouldFetch));
        }
        Ok(None)
    }
}

/// Why a run of git gave no output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunFailure {
    /// Git did not end within the time given; it was stopped.
    TimedOut,
    /// Git could not be run, ended with a failure, or its output could not be read.
    Failed(String),
}

impl fmt::Display for RunFailure {
    /// The failure as a sentence.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut => formatter.write_str("git did not end in time"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

/// Whose configuration git reads besides the repository's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settings {
    /// None: no system or global configuration, so that the answer does not depend on the
    /// machine or the user.
    Isolated,
    /// The user's own and the system's, for an answer that should be the one the user's own git
    /// gives.
    User,
}

/// Why a repository cannot be opened for reading with git.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
    /// The `PATH` names no `git` program outside the repository.
    #[error("no git program is on the PATH")]
    NoGit,
    /// The folder holds no `.git`, so it is not a git repository's top.
    #[error("no git repository is at the root")]
    NoRepository,
    /// The repository's own configuration names a filter, whose command git could run.
    #[error("the repository's own git configuration names a filter")]
    Refused,
    /// The repository is a partial clone, and git, older than 2.45, cannot be told not to fetch
    /// what the clone left out.
    #[error("the repository is a partial clone, and git older than 2.45 could fetch in it")]
    WouldFetch,
    /// The repository's own configuration sets `blame.ignoreRevsFile` to a file other than the
    /// top `.git-blame-ignore-revs`, which git would open wherever it is.
    #[error("the repository's own git configuration names a blame ignore list outside its top")]
    NamesIgnoreList,
    /// Git did not answer the questions that decide whether it may be run.
    #[error("asking git about the repository: {0}")]
    Git(String),
}

/// The git program for one repository.
pub(crate) struct GitProgram {
    /// The program.
    program: PathBuf,
    /// The repository's top, in its canonical form.
    root: PathBuf,
}

impl GitProgram {
    /// The git the `PATH` names outside `root`, a repository's top in its canonical form; `None`
    /// when there is none.
    pub(crate) fn find(root: &Path) -> Option<Self> {
        let program = find_program(GIT, root)?;
        Some(Self {
            program,
            root: root.to_path_buf(),
        })
    }

    /// The git for the repository at `root`, a repository's top in its canonical form, with what
    /// its own configuration says, when git may be run there: the repository has no filter and,
    /// being a partial clone, only a git that can be told not to fetch.
    ///
    /// # Errors
    ///
    /// No repository, no git, or one of the refusals, as [`OpenError`] says.
    pub(crate) fn open(root: &Path) -> Result<(Self, OwnConfig), OpenError> {
        if !has_repository(root) {
            return Err(OpenError::NoRepository);
        }
        let git = Self::find(root).ok_or(OpenError::NoGit)?;
        let own = git.own_config().map_err(OpenError::Git)?;
        match own
            .refusal(|| git.can_refuse_fetching())
            .map_err(OpenError::Git)?
        {
            Some(Refusal::Filter) => Err(OpenError::Refused),
            Some(Refusal::WouldFetch) => Err(OpenError::WouldFetch),
            None => Ok((git, own)),
        }
    }

    /// The repository's top.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Git with `options` before its command (such as `--no-lazy-fetch`), then `args`, reading the
    /// configuration `settings` says.
    fn command(&self, settings: Settings, options: &[&str], args: &[&str]) -> Command {
        let mut command = Command::new(&self.program);
        // The caller's `GIT_` variables could name another repository, object store, shallow
        // list, configuration or program; none of them applies to this one.
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("GIT_") {
                command.env_remove(&name);
            }
        }
        command
            .arg("--no-pager")
            .args(options)
            .args(args)
            .current_dir(&self.root)
            // A partial clone would otherwise fetch what it left out from its remote. Git reads
            // this from version 2.45; a listing in a partial clone also gives `--no-lazy-fetch`,
            // which older git refuses.
            .env("GIT_NO_LAZY_FETCH", "1");
        if settings == Settings::Isolated {
            command
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", NULL_DEVICE);
        }
        if let Some(parent) = self.root.parent() {
            command.env("GIT_CEILING_DIRECTORIES", parent);
        }
        command
    }

    /// What git printed for `args`, with `options` before its command, within `bounds`.
    ///
    /// # Errors
    ///
    /// Git does not start, or does not end in time.
    fn run(
        &self,
        settings: Settings,
        options: &[&str],
        args: &[&str],
        bounds: Bounds,
    ) -> Result<Captured, RunFailure> {
        let mut command = self.command(settings, options, args);
        capture(&mut command, bounds.within, bounds.max_bytes).map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                RunFailure::TimedOut
            } else {
                RunFailure::Failed(format!("git did not answer ({error})"))
            }
        })
    }

    /// Git's short answer to `args` as text; `None` when git says the thing asked for is not
    /// there.
    ///
    /// # Errors
    ///
    /// Git does not answer in time, fails, or answers more than a short answer or other than text.
    pub(crate) fn ask(&self, args: &[&str]) -> Result<Option<String>, String> {
        let answer = self
            .run(Settings::Isolated, &[], args, ANSWER)
            .map_err(|failure| failure.to_string())?;
        if answer.status.code() == Some(ABSENT) && answer.stdout.is_empty() {
            return Ok(None);
        }
        if !answer.status.success() {
            return Err(format!("git ended with {}", answer.status));
        }
        if answer.is_cut {
            return Err(format!("git answered more than {} bytes", ANSWER.max_bytes));
        }
        String::from_utf8(answer.stdout)
            .map(Some)
            .map_err(|_| "git's answer is not UTF-8 text".to_owned())
    }

    /// What the repository's own configuration, with the files it includes, says of filters, of a
    /// partial clone and of blame's list of commits to skip, from one question to git, which
    /// reads no system or global configuration for it.
    ///
    /// # Errors
    ///
    /// Git does not answer, or answers more than a short answer or other than text.
    pub(crate) fn own_config(&self) -> Result<OwnConfig, String> {
        let answer = self.ask(&[
            "config",
            "--includes",
            "--null",
            "--get-regexp",
            OWN_CONFIG_KEYS,
        ])?;
        Ok(own_config_of(answer.as_deref().unwrap_or_default()))
    }

    /// Whether this git can be told not to fetch what a partial clone left out: whether its
    /// version, as `git version` prints it, is [`NO_LAZY_FETCH_SINCE`] or later.
    ///
    /// # Errors
    ///
    /// Git does not answer, or prints a version that is not `git version <major>.<minor>...`.
    pub(crate) fn can_refuse_fetching(&self) -> Result<bool, String> {
        refuses_fetching(&self.ask(&["version"])?.unwrap_or_default())
    }

    /// What git lists for `args`, with `options` before its command, within `bounds`; `None` when
    /// it lists more than `bounds.max_bytes`.
    ///
    /// # Errors
    ///
    /// Git does not end in time ([`RunFailure::TimedOut`]) or fails.
    pub(crate) fn list(
        &self,
        settings: Settings,
        options: &[&str],
        args: &[&str],
        bounds: Bounds,
    ) -> Result<Option<Vec<u8>>, RunFailure> {
        let listing = self.run(settings, options, args, bounds)?;
        if !listing.status.success() {
            return Err(RunFailure::Failed(format!(
                "git ended with {}",
                listing.status
            )));
        }
        Ok((!listing.is_cut).then_some(listing.stdout))
    }
}

/// Whether the git whose `git version` answer is `answer` (`git version 2.45.1`, say) can be told
/// not to fetch: whether its version is [`NO_LAZY_FETCH_SINCE`] or later.
///
/// # Errors
///
/// The answer is not `git version <major>.<minor>...`.
fn refuses_fetching(answer: &str) -> Result<bool, String> {
    let mut numbers = answer
        .split_whitespace()
        .nth(2)
        .unwrap_or_default()
        .split('.')
        .map(str::parse::<u32>);
    match (numbers.next(), numbers.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => Ok((major, minor) >= NO_LAZY_FETCH_SINCE),
        _ => Err("git printed a version that cannot be read".to_owned()),
    }
}

/// [`OwnConfig`] from `listing`, git's `config --null --get-regexp` answer for
/// [`OWN_CONFIG_KEYS`]: entries ended by a NUL, each a key, then a newline and its value when it
/// has one.
fn own_config_of(listing: &str) -> OwnConfig {
    let mut config = OwnConfig::default();
    for entry in listing.split('\0').filter(|entry| !entry.is_empty()) {
        let (key, value) = match entry.split_once('\n') {
            Some((key, value)) => (key, Some(value)),
            None => (entry, None),
        };
        if key.starts_with("filter.") {
            config.names_filter = true;
        }
        if key == "blame.ignorerevsfile" {
            let is_top = value == Some(IGNORE_REVS_FILE);
            let is_only_top = is_top && config.ignore_list != IgnoreList::Other;
            config.ignore_list = if is_only_top {
                IgnoreList::Top
            } else {
                IgnoreList::Other
            };
        }
        let is_promisor = key.starts_with("remote.") && key.ends_with(".promisor");
        if key == "extensions.partialclone" || (is_promisor && is_true(value)) {
            config.is_partial = true;
        }
    }
    config
}

/// Whether git reads `value`, a configuration value or none for a key given without one, as true:
/// a key without a value, `true`, `yes`, `on` or a whole number other than 0 is; `false`, `no`,
/// `off`, `0` and an empty value are not. Any other text counts as true, as a value git cannot read
/// as a boolean counts for a remote that might fetch.
fn is_true(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return true;
    };
    match value.to_ascii_lowercase().as_str() {
        "" | "false" | "no" | "off" => false,
        "true" | "yes" | "on" => true,
        other => other.parse::<i64>() != Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Versions as `git version` prints them on Linux, Windows and macOS: 2.45 and later can be
    /// told not to fetch, 2.44, 2.43 and Apple's 2.39 cannot, and text that names no version is
    /// an error rather than a guess.
    #[test]
    fn a_version_from_2_45_refuses_fetching() {
        let cases = [
            ("git version 2.43.0\n", Ok(false)),
            ("git version 2.44.2", Ok(false)),
            ("git version 2.39.3 (Apple Git-146)", Ok(false)),
            ("git version 2.45.0", Ok(true)),
            ("git version 2.55.0.windows.1\n", Ok(true)),
            ("git version 3.0.0", Ok(true)),
        ];
        for (answer, expected) in cases {
            assert_eq!(refuses_fetching(answer), expected, "{answer}");
        }
        for answer in ["", "git version", "git version two.45", "version 2.45.0"] {
            assert!(refuses_fetching(answer).is_err(), "{answer}");
        }
    }

    /// Git's booleans, as its configuration documentation gives them: no value, `true`, `yes`,
    /// `on` and a number other than 0 are true, in any case; `false`, `no`, `off`, `0` and the
    /// empty value are false; text git cannot read as a boolean counts as true.
    #[test]
    fn values_read_as_git_reads_a_boolean() {
        for value in [
            None,
            Some("true"),
            Some("Yes"),
            Some("ON"),
            Some("1"),
            Some("-2"),
        ] {
            assert!(is_true(value), "{value:?}");
        }
        for value in ["false", "No", "OFF", "0", ""] {
            assert!(!is_true(Some(value)), "{value:?}");
        }
        assert!(is_true(Some("maybe")));
    }

    /// The answer git gives, its layout as `git config --null --get-regexp` prints it (a key, a
    /// newline and the value, then a NUL; a key without a value alone): a filter's key, whatever
    /// its driver's case, names a filter; `extensions.partialclone` makes a partial clone, and so
    /// does a promisor that reads as true, but not one that reads as false; nothing at all is
    /// neither.
    #[test]
    fn the_answer_says_what_the_configuration_names() {
        let read = |listing: &str| {
            let config = own_config_of(listing);
            (config.names_filter, config.is_partial)
        };
        assert_eq!(read(""), (false, false));
        assert_eq!(read("filter.Marking.clean\ncat\0"), (true, false));
        assert_eq!(read("extensions.partialclone\norigin\0"), (false, true));
        assert_eq!(read("remote.o.promisor\0"), (false, true));
        assert_eq!(read("remote.o.promisor\noff\0"), (false, false));
        assert_eq!(
            read("filter.x.process\nrun\0remote.p.promisor\nfalse\0remote.q.promisor\ntrue\0"),
            (true, true)
        );
    }

    /// The list of commits blame skips: unset says nothing; the top file alone is the top file,
    /// whatever its key's case (git prints keys in lower case); any other value, a path outside the
    /// repository or a share, makes it another one, wherever in the answer it comes.
    #[test]
    fn the_answer_says_which_ignore_list_the_configuration_names() {
        let read = |listing: &str| own_config_of(listing).ignore_list;
        assert_eq!(read(""), IgnoreList::Unset);
        assert_eq!(
            read("blame.ignorerevsfile\n.git-blame-ignore-revs\0"),
            IgnoreList::Top
        );
        for other in [
            "/etc/revs",
            "//server/share/revs",
            "missing",
            "./.git-blame-ignore-revs",
            "",
        ] {
            let listing = format!("blame.ignorerevsfile\n{other}\0");
            assert_eq!(read(&listing), IgnoreList::Other, "{other:?}");
        }
        let top_then_other =
            "blame.ignorerevsfile\n.git-blame-ignore-revs\0blame.ignorerevsfile\nx\0";
        assert_eq!(read(top_then_other), IgnoreList::Other);
        let other_then_top =
            "blame.ignorerevsfile\nx\0blame.ignorerevsfile\n.git-blame-ignore-revs\0";
        assert_eq!(read(other_then_top), IgnoreList::Other);
    }
}
