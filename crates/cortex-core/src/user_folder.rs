//! The user's Cortex folder, `.cortex` in the home folder, and the files a repository's daemon
//! keeps there.

use std::path::{Path, PathBuf};

use crate::RepoId;

/// The user's Cortex folder: their configuration, the logs, and the run files of every
/// repository's daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFolder(PathBuf);

impl UserFolder {
    /// The folder of the user whose home folder is `home`.
    #[must_use]
    pub fn in_home(home: &Path) -> Self {
        Self(home.join(".cortex"))
    }

    /// The current user's folder, in the home folder the operating system reports, or `None` when
    /// it reports none.
    #[must_use]
    pub fn of_current_user() -> Option<Self> {
        std::env::home_dir().map(|home| Self::in_home(&home))
    }

    /// The folder itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The user's configuration file.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.0.join("config.toml")
    }

    /// The folder of the logs.
    #[must_use]
    pub fn logs_folder(&self) -> PathBuf {
        self.0.join("logs")
    }

    /// The folder of every daemon's run files, readable by the user alone.
    #[must_use]
    pub fn run_folder(&self) -> PathBuf {
        self.0.join("run")
    }

    /// The run files of the daemon of the repository with id `repo_id`.
    #[must_use]
    pub fn run_files(&self, repo_id: &RepoId) -> RunFiles {
        RunFiles {
            folder: self.run_folder(),
            repo_id: repo_id.clone(),
        }
    }
}

/// Where one repository's daemon listens and what marks it as running, all named by the
/// repository's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFiles {
    /// The run folder.
    folder: PathBuf,
    /// The repository's id.
    repo_id: RepoId,
}

impl RunFiles {
    /// The Unix socket the daemon listens on: `<id>.sock` in the run folder, a short path that
    /// stays under the length macOS allows a socket's path.
    #[must_use]
    pub fn socket_path(&self) -> PathBuf {
        self.folder.join(format!("{}.sock", self.repo_id))
    }

    /// The Windows named pipe the daemon listens on, without its `\\.\pipe\` prefix.
    #[must_use]
    pub fn pipe_name(&self) -> String {
        format!("cortex-{}", self.repo_id)
    }

    /// The file whose lock the running daemon holds: `<id>.lock` in the run folder.
    #[must_use]
    pub fn lock_path(&self) -> PathBuf {
        self.folder.join(format!("{}.lock", self.repo_id))
    }

    /// The file holding the running daemon's process id: `<id>.pid` in the run folder.
    #[must_use]
    pub fn pid_path(&self) -> PathBuf {
        self.folder.join(format!("{}.pid", self.repo_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every file is named by the id, in the folders written out here by hand.
    #[test]
    fn files_are_named_by_the_repository_id() {
        let folder = UserFolder::in_home(Path::new("/home/dev"));
        let id = RepoId::of(Path::new("/work/repo"));
        let files = folder.run_files(&id);
        let run = Path::new("/home/dev/.cortex/run");
        assert_eq!(folder.path(), Path::new("/home/dev/.cortex"));
        assert_eq!(
            folder.config_path(),
            Path::new("/home/dev/.cortex/config.toml")
        );
        assert_eq!(folder.logs_folder(), Path::new("/home/dev/.cortex/logs"));
        assert_eq!(files.socket_path(), run.join(format!("{id}.sock")));
        assert_eq!(files.lock_path(), run.join(format!("{id}.lock")));
        assert_eq!(files.pid_path(), run.join(format!("{id}.pid")));
        assert_eq!(files.pipe_name(), format!("cortex-{id}"));
    }
}
