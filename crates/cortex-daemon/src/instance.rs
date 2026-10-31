//! One daemon per repository: the lock on `run/<id>.lock` in the user's Cortex folder, held for
//! the daemon's life, and `run/<id>.pid`, which names its process.
//!
//! The lock is the operating system's exclusive lock on the file, taken without waiting and
//! released when the file is closed, so a daemon that crashed leaves no stale lock. The run folder
//! is the user's alone on Unix (mode 0700), like the files in it.

use std::fs::{File, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use cortex_core::{RepoId, UserFolder, create_private_folder, open_private_file};

/// The running daemon's hold on its repository, given up when dropped.
#[derive(Debug)]
pub(crate) struct Instance {
    /// The locked file; closing it releases the lock.
    _lock: File,
    /// The file naming this process, removed when the hold is given up.
    pid_path: PathBuf,
}

impl Instance {
    /// Takes the hold on the repository with id `repo_id` for this process, creating the run
    /// folder in `user` when it is missing, and writes this process's id into the run files.
    ///
    /// # Errors
    ///
    /// Another daemon holds the repository (the message names its process when its file says
    /// which), and I/O errors.
    pub(crate) fn acquire(user: &UserFolder, repo_id: &RepoId) -> anyhow::Result<Self> {
        let run_folder = user.run_folder();
        create_private_folder(&run_folder)
            .with_context(|| format!("creating {}", run_folder.display()))?;
        let files = user.run_files(repo_id);
        let lock_path = files.lock_path();
        let lock = open_private_file(&lock_path)
            .with_context(|| format!("opening {}", lock_path.display()))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => bail!(
                "another daemon serves this repository{}",
                running_process(&files.pid_path())
                    .map(|pid| format!(" (process {pid})"))
                    .unwrap_or_default()
            ),
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("locking {}", lock_path.display()));
            }
        }
        let pid_path = files.pid_path();
        write_pid(&pid_path, std::process::id())?;
        Ok(Self {
            _lock: lock,
            pid_path,
        })
    }
}

impl Drop for Instance {
    /// Removes the process-id file before the lock goes. A file that cannot be removed is left:
    /// the next daemon writes its own id over it, and only the lock says whether one runs.
    fn drop(&mut self) {
        drop(std::fs::remove_file(&self.pid_path));
    }
}

/// Writes `pid` into the file at `path`, replacing what it held.
fn write_pid(path: &Path, pid: u32) -> anyhow::Result<()> {
    let mut file =
        open_private_file(path).with_context(|| format!("opening {}", path.display()))?;
    file.set_len(0)
        .and_then(|()| writeln!(file, "{pid}"))
        .with_context(|| format!("writing {}", path.display()))
}

/// The process id the file at `path` names, when it can be read as one.
fn running_process(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second hold on one repository is refused, naming the process that has it, while a hold
    /// on another repository is not; once the first is given up, its process-id file is gone and
    /// the repository can be held again.
    #[test]
    fn one_daemon_holds_a_repository() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let user = UserFolder::in_home(home.path());
        let repo = RepoId::of(Path::new("/work/repo"));
        let first = Instance::acquire(&user, &repo)?;
        let pid_path = user.run_files(&repo).pid_path();
        assert_eq!(
            std::fs::read_to_string(&pid_path)?,
            format!("{}\n", std::process::id())
        );
        let refused = Instance::acquire(&user, &repo)
            .err()
            .ok_or("a second hold")?;
        assert_eq!(
            refused.to_string(),
            format!(
                "another daemon serves this repository (process {})",
                std::process::id()
            )
        );
        let other = Instance::acquire(&user, &RepoId::of(Path::new("/work/other")))?;
        drop(first);
        assert!(!pid_path.exists());
        drop(Instance::acquire(&user, &repo)?);
        drop(other);
        Ok(())
    }
}
