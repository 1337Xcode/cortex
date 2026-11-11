//! Offering a folder that no repository set up for Cortex holds to a daemon that sets it up, once
//! the user turned adoption on (`cortex init --global` writes `~/.cortex/adopt`). The hook reads
//! no configuration for it, runs no git and waits for nothing: it looks at file metadata alone
//! (the switch, the folder's way up to the home folder, a refusal remembered), and starts `cortex
//! daemon --adopt` for the folder, detached, which checks everything again with the whole
//! configuration and either sets the folder up or remembers that it refused.

use std::io;
use std::path::Path;
use std::time::SystemTime;

use cortex_core::{Places, RepoId, UserFolder, adoption_candidate};

use crate::spawn::{Started, start_adoption};

/// What became of an offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Offered {
    /// Adoption is off: the switch's file is not there.
    Off,
    /// The home folder cannot be found, so no folder can be checked against it.
    NoHome,
    /// The folder is never set up on its own (see [`cortex_core::Refusal`]).
    Refused,
    /// The daemon refused the folder before, and remembered it.
    Remembered,
    /// A daemon was started for the folder now, or a hook started one moments ago.
    Started(Started),
}

/// Offers the folder holding the working folder `cwd`, which no repository set up for Cortex
/// holds, to a daemon that sets it up, the user's home folder being `home` and the system's
/// temporary folder `temp`, at `now`, as the module says.
///
/// # Errors
///
/// This program's path cannot be read, or the daemon could not be started.
pub(crate) fn offer(
    cwd: &Path,
    (home, temp): (&Path, &Path),
    now: SystemTime,
) -> io::Result<Offered> {
    let user = UserFolder::in_home(home);
    if !user.adopt_path().is_file() {
        return Ok(Offered::Off);
    }
    let Some(places) = Places::of(home, temp) else {
        return Ok(Offered::NoHome);
    };
    let Ok(candidate) = adoption_candidate(cwd, &places) else {
        return Ok(Offered::Refused);
    };
    let repo_id = RepoId::of(&candidate.folder);
    if user.refusal_path(&repo_id).exists() {
        return Ok(Offered::Remembered);
    }
    let program = std::env::current_exe()?;
    let files = user.run_files(&repo_id);
    start_adoption(&program, &candidate.folder, &files, now).map(Offered::Started)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use cortex_core::canonical_root;

    use super::*;

    /// With no switch, nothing is looked at further; with it, the home folder is refused, and a
    /// folder refused before is not offered again. The start itself is not made here: the test
    /// binary is not `cortex`.
    #[test]
    fn only_a_switched_on_user_s_folders_are_offered() -> Result<(), Box<dyn Error>> {
        let base = tempfile::tempdir()?;
        let home = canonical_root(base.path())?.join("home");
        let repo = home.join("repo");
        std::fs::create_dir_all(repo.join(".git"))?;
        // The test's folders are in the system's temporary folder: another stands in for it.
        let temp = canonical_root(base.path())?.join("tmp");
        std::fs::create_dir_all(&temp)?;
        let places = (home.as_path(), temp.as_path());
        let now = SystemTime::now();
        assert_eq!(offer(&repo, places, now)?, Offered::Off);
        let user = UserFolder::in_home(&home);
        std::fs::create_dir_all(user.path())?;
        std::fs::write(user.adopt_path(), "")?;
        assert_eq!(offer(&home, places, now)?, Offered::Refused);
        let refusal = user.refusal_path(&RepoId::of(&canonical_root(&repo)?));
        std::fs::create_dir_all(refusal.parent().ok_or("no folder")?)?;
        std::fs::write(&refusal, "")?;
        assert_eq!(offer(&repo.join(".git"), places, now)?, Offered::Remembered);
        Ok(())
    }
}
