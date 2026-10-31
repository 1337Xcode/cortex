//! Starting a repository's daemon from a hook that found none: in the background, detached from
//! the agent, never waited for. The daemon binds its endpoint before anything else, so the next
//! hook finds it within moments; the hook that started it goes without.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Starts `cortex daemon --root <root>` from the binary at `program`, detached, with no standard
/// streams, and returns at once. A daemon already serving the root refuses to run and exits, so
/// two hooks that both found none start one daemon between them.
///
/// # Errors
///
/// The process could not be created.
pub(crate) fn start_daemon(program: &Path, root: &Path) -> io::Result<()> {
    let mut command = Command::new(program);
    command
        .arg("daemon")
        .arg("--root")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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

/// No console, a process group of its own, and outside the agent's job object when the job allows
/// it: a job that ends its processes with the agent would otherwise end the daemon too. A job that
/// forbids breaking away refuses the start, which is then tried again without it.
#[cfg(windows)]
fn detach(command: &mut Command) -> io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;

    /// The process gets no console, its own or the agent's.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    /// The process heads a process group of its own, so the agent's Ctrl+C does not reach it.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    /// The process leaves the job object the hook runs in, when the job allows it.
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    match command
        .creation_flags(detached | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
    {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            command.creation_flags(detached).spawn()
        }
        spawned => spawned,
    }
}
