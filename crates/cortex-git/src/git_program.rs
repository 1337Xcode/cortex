//! The `git` program as the history source runs it: the one the `PATH` names outside the
//! repository, in the repository's top, with none of the caller's `GIT_` variables, no system or
//! global configuration, no repository looked for above the top, nothing fetched that a partial
//! clone left out, and a bounded time and answer. The work tree's status is read the same way but
//! with the user's own configuration, so that it lists what the user's own git lists.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use cortex_core::{Captured, GIT, capture, find_program};

/// How long git has to answer a question about HEAD, the configuration or a file in the git
/// folder.
const ASK_WITHIN: Duration = Duration::from_secs(10);

/// How long git has to list the history a window covers: 20,000 commits of a large repository
/// take it tens of seconds.
const LIST_WITHIN: Duration = Duration::from_secs(300);

/// The most of a short answer read: an id, a path, a few settings.
const MAX_ANSWER_BYTES: u64 = 64 * 1024;

/// The most of a listing read; a listing cut short is not read at all.
const MAX_LISTING_BYTES: u64 = 256 * 1024 * 1024;

/// The exit code of `git config --get` and `git rev-parse --verify --quiet` for a value or a
/// revision that is not there.
const ABSENT: i32 = 1;

/// The file git reads as an empty configuration.
const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// The first git that can be told not to fetch what a partial clone left out, with
/// `GIT_NO_LAZY_FETCH` or `--no-lazy-fetch`: 2.45, as its major and minor versions.
const NO_LAZY_FETCH_SINCE: (u32, u32) = (2, 45);

/// The keys of a repository's own configuration that decide how its work tree may be read: a
/// filter's, and those that make it a partial clone.
const OWN_CONFIG_KEYS: &str = r"^filter\.|^extensions\.partialclone$|^remote\..*\.promisor$";

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

    /// The repository's top.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Git with `options` before its command (such as `--no-lazy-fetch`), then `args`, as the
    /// module says.
    fn command(&self, options: &[&str], args: &[&str]) -> Command {
        let mut command = self.as_user(options, args);
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", NULL_DEVICE);
        command
    }

    /// Git as [`GitProgram::command`] runs it, but reading the user's own and the system's
    /// configuration, for an answer that should be the one the user's own git gives.
    fn as_user(&self, options: &[&str], args: &[&str]) -> Command {
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
    pub(crate) fn ask(&self, args: &[&str]) -> Result<Option<String>, String> {
        let answer = run(&mut self.command(&[], args), ASK_WITHIN, MAX_ANSWER_BYTES)?;
        if answer.status.code() == Some(ABSENT) && answer.stdout.is_empty() {
            return Ok(None);
        }
        if !answer.status.success() {
            return Err(format!("git ended with {}", answer.status));
        }
        if answer.is_cut {
            return Err(format!("git answered more than {MAX_ANSWER_BYTES} bytes"));
        }
        String::from_utf8(answer.stdout)
            .map(Some)
            .map_err(|_| "git's answer is not UTF-8 text".to_owned())
    }

    /// What the repository's own configuration, with the files it includes, says of filters and
    /// of a partial clone, from one question to git, which reads no system or global
    /// configuration for it.
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

    /// What git lists for `args`, with `options` before its command.
    ///
    /// # Errors
    ///
    /// Git does not end in time, fails, or lists more than [`MAX_LISTING_BYTES`].
    pub(crate) fn list(&self, options: &[&str], args: &[&str]) -> Result<Vec<u8>, String> {
        let listing = run(
            &mut self.command(options, args),
            LIST_WITHIN,
            MAX_LISTING_BYTES,
        )?;
        if !listing.status.success() {
            return Err(format!("git ended with {}", listing.status));
        }
        if listing.is_cut {
            return Err(format!("git listed more than {MAX_LISTING_BYTES} bytes"));
        }
        Ok(listing.stdout)
    }

    /// What git lists for `args`, with `options` before its command, reading the user's own
    /// configuration ([`GitProgram::as_user`]), in at most `within`; `None` when it lists more
    /// than `max_bytes`.
    ///
    /// # Errors
    ///
    /// Git does not end in time, or fails.
    pub(crate) fn list_as_user(
        &self,
        options: &[&str],
        args: &[&str],
        (within, max_bytes): (Duration, u64),
    ) -> Result<Option<Vec<u8>>, String> {
        let listing = run(&mut self.as_user(options, args), within, max_bytes)?;
        if !listing.status.success() {
            return Err(format!("git ended with {}", listing.status));
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

/// What `command` printed, run for at most `within` and `max_bytes` of output; why not, as text.
fn run(command: &mut Command, within: Duration, max_bytes: u64) -> Result<Captured, String> {
    capture(command, within, max_bytes).map_err(|error| format!("git did not answer ({error})"))
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
}
