//! Replacing a daemon older than this build. A command that finds one asks it to stop, waits for
//! it to let go of the repository, starts a daemon of this build in its place and asks that one. A
//! newer daemon is left alone, so two installed versions never take turns replacing each other.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use cortex_core::ipc::{RequestBody, Role, is_older_than_this_build};
use cortex_core::{RepoId, UserFolder};
use cortex_daemon::is_running;
use cortex_hook::{Answer, ClientError, Stop, ask, start_daemon, stop_if_older};

/// How long a command gives an older daemon to stop and the new one to answer: the old one drains
/// its inbox and gives its connections a second; the new one answers as soon as it listens.
const REPLACE_WITHIN: Duration = Duration::from_secs(20);

/// How often a command looks again while it waits.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// What a command got from the repository's daemon.
pub(crate) struct Asked {
    /// The daemon's answer, or why there is none.
    pub(crate) answer: Result<Answer, ClientError>,
    /// The Cortex version of the daemon this command stopped and replaced, if it did.
    pub(crate) replaced: Option<String>,
}

/// Asks the daemon of the repository at `root`, whose user folder is `user`, `body` as a command
/// before `deadline`, after replacing it when it is older than this build.
pub(crate) fn ask_current(
    root: &Path,
    user: &UserFolder,
    body: &RequestBody,
    deadline: Instant,
) -> Asked {
    let repo_id = RepoId::of(root);
    let files = user.run_files(&repo_id);
    let first = ask(&files, Role::Cli, body.clone(), deadline);
    let version = match &first {
        Ok(answer) => answer.welcome.cortex_version.as_str(),
        Err(ClientError::Mismatch(theirs)) => theirs.cortex_version.as_str(),
        Err(_) => "",
    };
    if !is_older_than_this_build(version) {
        return Asked {
            answer: first,
            replaced: None,
        };
    }
    match replace(root, (user, &repo_id), body) {
        Ok((answer, replaced)) => Asked {
            answer: Ok(answer),
            replaced,
        },
        Err(error) => Asked {
            answer: Err(error),
            replaced: None,
        },
    }
}

/// Asks the older daemon of the repository at `root` to stop, on a connection whose handshake shows
/// it still older, waits until it has let go of the repository, starts a daemon of this build and
/// asks it `body`, all within [`REPLACE_WITHIN`]. Returns the answer, with the version stopped when
/// this command stopped one: a daemon that already took the older one's place is asked as it is.
fn replace(
    root: &Path,
    (user, repo_id): (&UserFolder, &RepoId),
    body: &RequestBody,
) -> Result<(Answer, Option<String>), ClientError> {
    let files = user.run_files(repo_id);
    let until = Instant::now() + REPLACE_WITHIN;
    // Found before anything is stopped: a command that cannot start a daemon stops none.
    let program = std::env::current_exe().map_err(ClientError::Io)?;
    let replaced = match stop_if_older(&files, Role::Cli, until) {
        Ok(Stop::Asked(version)) => Some(version),
        Ok(Stop::Declined(_)) | Err(ClientError::NotRunning) => None,
        Err(error) => return Err(error),
    };
    if replaced.is_some() {
        while is_running(user, repo_id).map_err(ClientError::Io)? {
            if Instant::now() >= until {
                return Err(ClientError::TimedOut);
            }
            std::thread::sleep(LOOK_EVERY);
        }
    }
    if !is_running(user, repo_id).map_err(ClientError::Io)? {
        start_daemon(&program, root, &files, SystemTime::now()).map_err(ClientError::Io)?;
    }
    loop {
        match ask(&files, Role::Cli, body.clone(), until) {
            Err(ClientError::NotRunning) if Instant::now() < until => {
                std::thread::sleep(LOOK_EVERY);
            }
            other => return other.map(|answer| (answer, replaced)),
        }
    }
}
