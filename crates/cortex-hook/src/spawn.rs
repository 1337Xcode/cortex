//! Starting a repository's daemon from a hook that found none: in the background, detached from
//! the agent, never waited for. The daemon binds its endpoint before anything else, so the next
//! hook finds it within moments; the hook that started it goes without. A folder not set up yet is
//! handed the same way to a daemon started to set it up first (`--adopt`), which decides whether
//! it may.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use cortex_core::{RunFiles, create_private_folder, open_private_file};

/// The least time between two starts of one repository's daemon by hooks. A daemon binds within
/// moments; hooks that find none meanwhile leave the start under way alone, and a daemon that
/// keeps failing costs one process every ten seconds, not one per tool call.
const RESTART_GAP: Duration = Duration::from_secs(10);

/// The flag of `cortex daemon` that has it set the folder up on its own before it runs.
pub const ADOPT_FLAG: &str = "--adopt";

/// What a daemon is started for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Serving a repository set up for Cortex.
    Serve,
    /// Setting a folder up on its own first, when the configuration allows it, then serving it.
    Adopt,
}

/// Whether a start was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    /// A daemon process was created.
    Now,
    /// A hook started one less than [`RESTART_GAP`] ago; none was created.
    Recently,
}

/// Starts `cortex daemon --root <root>` from the binary at `program`, detached, in the root with no
/// standard streams, and returns at once, unless a hook started one for this repository, whose run
/// files are `files`, less than [`RESTART_GAP`] before `now`. A daemon already serving the root
/// refuses to run and exits, so two starts make one daemon.
///
/// # Errors
///
/// The run folder or the start's marker could not be written, or the process could not be created.
pub fn start_daemon(
    program: &Path,
    root: &Path,
    files: &RunFiles,
    now: SystemTime,
) -> io::Result<Started> {
    start_gapped(program, (root, Purpose::Serve), files, now)
}

/// Starts `cortex daemon --root <folder> --adopt` as [`start_daemon`] starts a daemon, with the
/// same gap between two starts for one folder, whose run files are `files`: the daemon sets the
/// folder up first when it may, and remembers a refusal otherwise.
///
/// # Errors
///
/// As [`start_daemon`].
pub fn start_adoption(
    program: &Path,
    folder: &Path,
    files: &RunFiles,
    now: SystemTime,
) -> io::Result<Started> {
    start_gapped(program, (folder, Purpose::Adopt), files, now)
}

/// Starts `cortex daemon` for `root` and `purpose` from the binary at `program`, unless a hook
/// started one for the folder whose run files are `files` less than [`RESTART_GAP`] before `now`.
///
/// # Errors
///
/// As [`start_daemon`].
fn start_gapped(
    program: &Path,
    (root, purpose): (&Path, Purpose),
    files: &RunFiles,
    now: SystemTime,
) -> io::Result<Started> {
    let marker = files.start_path();
    let is_recent = std::fs::metadata(&marker)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|started| now.duration_since(started).ok())
        .is_some_and(|gap| gap < RESTART_GAP);
    if is_recent {
        return Ok(Started::Recently);
    }
    start_marked(program, (root, purpose), files, now).map(|()| Started::Now)
}

/// Starts `cortex daemon --root <root>` as [`start_daemon`] does, whenever a start was made: for a
/// command the user runs, which is no storm of hooks, and finds no daemon. The start is marked all
/// the same, so that hooks keep their gap after it. On Windows the process started is
/// `cortex daemon --relaunch`, which starts the daemon anew holding none of this process's handles
/// (see [`crate::relaunch`]).
///
/// # Errors
///
/// The run folder or the start's marker could not be written, or the process could not be created.
pub fn start_daemon_now(
    program: &Path,
    root: &Path,
    files: &RunFiles,
    now: SystemTime,
) -> io::Result<()> {
    start_marked(program, (root, Purpose::Serve), files, now)
}

/// Marks a start at `now` in the run files `files` and starts `cortex daemon --root <root>`,
/// `--adopt` added for [`Purpose::Adopt`], as [`start_daemon_now`] says.
///
/// # Errors
///
/// As [`start_daemon_now`].
fn start_marked(
    program: &Path,
    (root, purpose): (&Path, Purpose),
    files: &RunFiles,
    now: SystemTime,
) -> io::Result<()> {
    let marker = files.start_path();
    if let Some(folder) = marker.parent() {
        create_private_folder(folder)?;
    }
    open_private_file(&marker)?.set_modified(now)?;
    let mut command = Command::new(program);
    command
        .arg("daemon")
        .arg("--root")
        .arg(root)
        // Not the agent's working folder, which the daemon would keep from being removed while it
        // runs: the root, which its watcher holds anyway.
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if purpose == Purpose::Adopt {
        command.arg(ADOPT_FLAG);
    }
    #[cfg(windows)]
    command.arg(crate::relaunch::RELAUNCH_FLAG);
    // The child is not waited for: it outlives the hook by design, and dropping the handle neither
    // stops nor waits for it.
    detach(&mut command).map(drop)
}

/// Spawns `command` in a process group of its own, so that a signal to the agent's group (an
/// interrupt from its terminal) does not reach the daemon.
#[cfg(unix)]
fn detach(command: &mut Command) -> io::Result<std::process::Child> {
    std::os::unix::process::CommandExt::process_group(command, 0);
    command.spawn()
}

/// A console of its own that is never shown, a process group of its own, and outside the agent's
/// job object when the job allows it: a job that ends its processes with the agent would otherwise
/// end the daemon too. A job that forbids breaking away refuses the start, which is then tried
/// again without it.
#[cfg(windows)]
fn detach(command: &mut Command) -> io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;

    /// A console that no window shows. With no console at all, each console program the daemon
    /// runs (git, whoami) would open a window of its own; with a hidden one, they share it.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// The process heads a process group of its own, so the agent's Ctrl+C does not reach it.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    /// The process leaves the job object the hook runs in, when the job allows it.
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let hidden = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
    match command
        .creation_flags(hidden | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
    {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            command.creation_flags(hidden).spawn()
        }
        spawned => spawned,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use cortex_core::{RepoId, UserFolder};

    use super::*;

    /// A start leaves its marker, made private with its folder; within ten seconds of it no other
    /// start is made, and ten seconds or more later one is. The program here does not exist, so a
    /// start that is made fails, and one that is not succeeds without a process.
    #[test]
    fn one_start_in_ten_seconds() -> Result<(), Box<dyn Error>> {
        let home = tempfile::tempdir()?;
        let root = home.path().join("repo");
        std::fs::create_dir(&root)?;
        let files = UserFolder::in_home(home.path()).run_files(&RepoId::of(&root));
        let program = home.path().join("no-such-program");
        let now = SystemTime::now();
        let first = start_daemon(&program, &root, &files, now);
        assert!(first.is_err(), "{first:?}");
        assert!(files.start_path().exists());
        let soon = start_daemon(&program, &root, &files, now + Duration::from_secs(9));
        assert_eq!(soon?, Started::Recently);
        let later = start_daemon(&program, &root, &files, now + Duration::from_secs(10));
        assert!(later.is_err(), "{later:?}");
        Ok(())
    }

    /// A start to set a folder up keeps the same gap as a start to serve it: within ten seconds of
    /// either, neither is made again. The program here does not exist, so a start that is made
    /// fails.
    #[test]
    fn an_adoption_keeps_the_gap() -> Result<(), Box<dyn Error>> {
        let home = tempfile::tempdir()?;
        let folder = home.path().join("repo");
        std::fs::create_dir(&folder)?;
        let files = UserFolder::in_home(home.path()).run_files(&RepoId::of(&folder));
        let program = home.path().join("no-such-program");
        let now = SystemTime::now();
        assert!(start_adoption(&program, &folder, &files, now).is_err());
        let soon = now + Duration::from_secs(9);
        assert_eq!(
            start_adoption(&program, &folder, &files, soon)?,
            Started::Recently
        );
        assert_eq!(
            start_daemon(&program, &folder, &files, soon)?,
            Started::Recently
        );
        Ok(())
    }

    /// A command's start is made within the hooks' gap, and moves the marker, so that a hook keeps
    /// its gap after it. The program here does not exist, so a start that is made fails.
    #[test]
    fn a_command_starts_within_the_gap() -> Result<(), Box<dyn Error>> {
        let home = tempfile::tempdir()?;
        let root = home.path().join("repo");
        std::fs::create_dir(&root)?;
        let files = UserFolder::in_home(home.path()).run_files(&RepoId::of(&root));
        let program = home.path().join("no-such-program");
        let now = SystemTime::now();
        assert!(start_daemon(&program, &root, &files, now).is_err());
        let soon = now + Duration::from_secs(5);
        let command = start_daemon_now(&program, &root, &files, soon);
        assert!(command.is_err(), "{command:?}");
        let after = start_daemon(&program, &root, &files, now + Duration::from_secs(12));
        assert_eq!(after?, Started::Recently);
        Ok(())
    }
}
