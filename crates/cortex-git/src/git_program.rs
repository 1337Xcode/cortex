//! The `git` program as the history source runs it: the one the `PATH` names outside the
//! repository, in the repository's top, with none of the caller's `GIT_` variables, no system or
//! global configuration, no repository looked for above the top, nothing fetched that a partial
//! clone left out, and a bounded time and answer.

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
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
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
}

/// What `command` printed, run for at most `within` and `max_bytes` of output; why not, as text.
fn run(command: &mut Command, within: Duration, max_bytes: u64) -> Result<Captured, String> {
    capture(command, within, max_bytes).map_err(|error| format!("git did not answer ({error})"))
}
