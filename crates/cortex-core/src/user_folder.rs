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

    /// The user's price table, which receipts estimate costs from.
    #[must_use]
    pub fn pricing_path(&self) -> PathBuf {
        self.0.join("pricing.toml")
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

    /// The file whose presence turns adoption on: Cortex then sets up on its own a repository an
    /// agent works in that is not set up yet (see [`crate::adoption_candidate`]).
    #[must_use]
    pub fn adopt_path(&self) -> PathBuf {
        self.0.join("adopt")
    }

    /// The folder of the folders Cortex refused to set up on its own, one file each, so that hooks
    /// stop asking.
    #[must_use]
    pub fn refused_folder(&self) -> PathBuf {
        self.run_folder().join("refused")
    }

    /// The file that says Cortex refused to set up on its own the folder with id `repo_id`.
    #[must_use]
    pub fn refusal_path(&self, repo_id: &RepoId) -> PathBuf {
        self.refused_folder().join(repo_id.as_str())
    }

    /// The file whose lock a daemon setting up the folder with id `repo_id` holds while it does,
    /// and while it then serves the folder, so that a second start for the folder sets nothing up.
    #[must_use]
    pub fn adopting_path(&self, repo_id: &RepoId) -> PathBuf {
        self.run_folder().join(format!("{repo_id}.adopt"))
    }

    /// The file that records the repository with id `repo_id` as set up for Cortex on this
    /// machine (see [`crate::is_set_up_here`]).
    #[must_use]
    pub fn set_up_path(&self, repo_id: &RepoId) -> PathBuf {
        self.0.join("repositories").join(repo_id.as_str())
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

/// Where a daemon listens, in the form its platform uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix domain socket at this path.
    Socket(PathBuf),
    /// A Windows named pipe with this name, without its `\\.\pipe\` prefix.
    Pipe(String),
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

    /// Where the daemon listens on this platform: its named pipe on Windows, its socket elsewhere.
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        if cfg!(windows) {
            Endpoint::Pipe(self.pipe_name())
        } else {
            Endpoint::Socket(self.socket_path())
        }
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

    /// The file whose modification time says when a hook last started the daemon: `<id>.start` in
    /// the run folder.
    #[must_use]
    pub fn start_path(&self) -> PathBuf {
        self.folder.join(format!("{}.start", self.repo_id))
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
        assert_eq!(folder.adopt_path(), Path::new("/home/dev/.cortex/adopt"));
        assert_eq!(
            folder.refusal_path(&id),
            run.join("refused").join(id.as_str())
        );
        assert_eq!(folder.adopting_path(&id), run.join(format!("{id}.adopt")));
        assert_eq!(
            folder.set_up_path(&id),
            Path::new("/home/dev/.cortex/repositories").join(id.as_str())
        );
        assert_eq!(files.socket_path(), run.join(format!("{id}.sock")));
        assert_eq!(files.lock_path(), run.join(format!("{id}.lock")));
        assert_eq!(files.pid_path(), run.join(format!("{id}.pid")));
        assert_eq!(files.start_path(), run.join(format!("{id}.start")));
        assert_eq!(files.pipe_name(), format!("cortex-{id}"));
        let endpoint = if cfg!(windows) {
            Endpoint::Pipe(format!("cortex-{id}"))
        } else {
            Endpoint::Socket(run.join(format!("{id}.sock")))
        };
        assert_eq!(files.endpoint(), endpoint);
    }
}
