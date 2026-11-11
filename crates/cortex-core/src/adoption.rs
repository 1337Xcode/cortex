//! Which folder Cortex may set up on its own, once the user turned that on (`cortex init
//! --global`): a hook whose working folder is in no repository set up for Cortex names the nearest
//! folder at or above it that holds a `.git`, and the daemon started for that folder sets it up.
//!
//! Never a folder in no git working tree (`cortex init` alone sets one up); never the home folder,
//! nor anything outside it; never the user's own Cortex folder (`~/.cortex`) or anything in it;
//! never the system's temporary folder, anything inside it or a folder holding it; never the file
//! system's root; and never a folder at or below one whose `.cortex` holds `off`, which `cortex
//! off` writes. A home folder that is relative or the file system's root gives no places at all:
//! against it, every rule would read the wrong way. Only file metadata is read here and no git
//! runs, so that a hook may ask.

use std::path::{Path, PathBuf};

use crate::canonical_root;

/// The folder that marks a repository set up for Cortex.
const MARKER: &str = ".cortex";

/// What a working tree's top holds: git's folder, or in a linked working tree a file naming it.
const DOT_GIT: &str = ".git";

/// The file in a repository's `.cortex` that turns Cortex off there, and at every folder below it.
pub const OFF_FILE: &str = "off";

/// The file in a repository's `.cortex` that says Cortex set it up on its own and has not told
/// the user yet.
pub const ADOPTED_FILE: &str = "adopted";

/// The folders adoption keeps away from, canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    /// The user's home folder.
    pub home: PathBuf,
    /// The system's temporary folder; none when it does not exist, so that nothing is in it.
    pub temp: Option<PathBuf>,
}

impl Places {
    /// The places for the home folder `home` and the temporary folder `temp`, in any spelling;
    /// none when the home folder cannot be found, is spelled relative to the working folder, or
    /// is the file system's root.
    #[must_use]
    pub fn of(home: &Path, temp: &Path) -> Option<Self> {
        if !home.is_absolute() {
            return None;
        }
        let home = canonical_root(home).ok()?;
        home.parent()?;
        Some(Self {
            home,
            temp: canonical_root(temp).ok(),
        })
    }
}

/// A folder Cortex may set up on its own, as far as file metadata tells: a git working tree's top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The folder, canonical.
    pub folder: PathBuf,
}

/// Why a folder is never set up on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The working folder cannot be found, or is not a folder.
    NoFolder,
    /// The folder is the home folder or outside it.
    OutsideHome,
    /// The folder is in no git working tree below the home folder.
    NotGit,
    /// The folder is the user's own Cortex folder or inside it.
    UserFolder,
    /// The folder is the file system's root.
    FileSystemRoot,
    /// The folder is the temporary folder, inside it, or holds it.
    Temporary,
    /// The folder, or one above it below the home folder, has a `.cortex` holding `off`.
    TurnedOff,
}

impl Refusal {
    /// Why, in words.
    #[must_use]
    pub fn said(self) -> &'static str {
        match self {
            Self::NoFolder => "the folder cannot be found",
            Self::OutsideHome => "it is the home folder or outside it",
            Self::NotGit => "it is in no git working tree; `cortex init` sets such a folder up",
            Self::UserFolder => "it is in the user's own Cortex folder",
            Self::FileSystemRoot => "it is the file system's root",
            Self::Temporary => "it is the temporary folder, inside it, or holds it",
            Self::TurnedOff => "`cortex off` turned Cortex off there",
        }
    }
}

/// The folder Cortex may set up on its own for a hook whose working folder is `cwd`, in any
/// spelling: the nearest folder at or above it, below the home folder, holding a `.git`.
///
/// # Errors
///
/// The reason the folder is never set up on its own, as the module says.
pub fn adoption_candidate(cwd: &Path, places: &Places) -> Result<Candidate, Refusal> {
    let cwd = canonical_root(cwd).map_err(|_| Refusal::NoFolder)?;
    let home = places.home.as_path();
    if cwd == home || !cwd.starts_with(home) {
        return Err(Refusal::OutsideHome);
    }
    if below_home(&cwd, home).any(is_turned_off) {
        return Err(Refusal::TurnedOff);
    }
    let folder = working_tree_top(&cwd, home)
        .ok_or(Refusal::NotGit)?
        .to_path_buf();
    if folder.parent().is_none() {
        return Err(Refusal::FileSystemRoot);
    }
    if folder.starts_with(home.join(MARKER)) {
        return Err(Refusal::UserFolder);
    }
    if let Some(temp) = &places.temp
        && (folder.starts_with(temp) || temp.starts_with(&folder))
    {
        return Err(Refusal::Temporary);
    }
    Ok(Candidate { folder })
}

/// The nearest folder at or above `cwd`, below `home`, that holds a `.git`, both canonical with
/// `cwd` inside `home`; none when none does.
#[must_use]
pub fn working_tree_top<'a>(cwd: &'a Path, home: &'a Path) -> Option<&'a Path> {
    below_home(cwd, home).find(|folder| folder.join(DOT_GIT).exists())
}

/// Whether the folder `folder` has a `.cortex` holding `off`.
#[must_use]
pub fn is_turned_off(folder: &Path) -> bool {
    folder.join(MARKER).join(OFF_FILE).is_file()
}

/// `cwd` and each folder above it below `home`, nearest first.
fn below_home<'a>(cwd: &'a Path, home: &'a Path) -> impl Iterator<Item = &'a Path> {
    cwd.ancestors().take_while(move |folder| *folder != home)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn Error>>;

    /// A base folder holding `home` and `tmp`, and the places they make.
    fn places() -> Result<(tempfile::TempDir, Places), Box<dyn Error>> {
        let base = tempfile::tempdir()?;
        std::fs::create_dir_all(base.path().join("home"))?;
        std::fs::create_dir_all(base.path().join("tmp"))?;
        let places =
            Places::of(&base.path().join("home"), &base.path().join("tmp")).ok_or("no home")?;
        Ok((base, places))
    }

    /// From a folder inside a git working tree below home, the tree's top; a linked working
    /// tree's `.git` file counts as a top.
    #[test]
    fn the_nearest_git_top_below_home_is_the_candidate() -> TestResult {
        let (_base, places) = places()?;
        let repo = places.home.join("code").join("repo");
        std::fs::create_dir_all(repo.join(".git"))?;
        std::fs::create_dir_all(repo.join("src").join("deep"))?;
        let found = adoption_candidate(&repo.join("src").join("deep"), &places);
        assert_eq!(found, Ok(Candidate { folder: repo }));
        let linked = places.home.join("linked");
        std::fs::create_dir_all(&linked)?;
        std::fs::write(linked.join(".git"), "gitdir: elsewhere\n")?;
        assert_eq!(
            adoption_candidate(&linked, &places),
            Ok(Candidate { folder: linked })
        );
        Ok(())
    }

    /// The home folder, a folder outside it, one that does not exist, one in no git working tree,
    /// the temporary folder and anything in it, and the user's own Cortex folder are refused, and
    /// so is a folder whose only `.git` above it is the home folder's.
    #[test]
    fn home_outside_temporary_and_the_user_folder_are_refused() -> TestResult {
        let (base, places) = places()?;
        assert_eq!(
            adoption_candidate(&places.home, &places),
            Err(Refusal::OutsideHome)
        );
        let outside = base.path().join("elsewhere");
        std::fs::create_dir_all(outside.join(".git"))?;
        assert_eq!(
            adoption_candidate(&outside, &places),
            Err(Refusal::OutsideHome)
        );
        assert_eq!(
            adoption_candidate(&places.home.join("missing"), &places),
            Err(Refusal::NoFolder)
        );
        let loose = places.home.join("notes");
        std::fs::create_dir_all(&loose)?;
        assert_eq!(adoption_candidate(&loose, &places), Err(Refusal::NotGit));
        let own = places.home.join(".cortex").join("clone");
        std::fs::create_dir_all(own.join(".git"))?;
        assert_eq!(adoption_candidate(&own, &places), Err(Refusal::UserFolder));
        let temp = places.temp.clone().ok_or("no temporary folder")?;
        let in_temp = Places {
            home: canonical_root(base.path())?,
            temp: Some(temp.clone()),
        };
        let scratch = temp.join("clone");
        std::fs::create_dir_all(scratch.join(".git"))?;
        assert_eq!(
            adoption_candidate(&scratch, &in_temp),
            Err(Refusal::Temporary)
        );
        std::fs::create_dir_all(places.home.join(".git"))?;
        assert_eq!(adoption_candidate(&loose, &places), Err(Refusal::NotGit));
        Ok(())
    }

    /// A home folder spelled relative to the working folder, or the file system's root, gives no
    /// places: every rule would read the wrong way against it.
    #[test]
    fn a_relative_or_root_home_gives_no_places() -> TestResult {
        let (base, _) = places()?;
        let temp = base.path().join("tmp");
        assert_eq!(Places::of(Path::new("home"), &temp), None);
        assert_eq!(Places::of(Path::new("."), &temp), None);
        let root = base
            .path()
            .ancestors()
            .last()
            .ok_or("no root")?
            .to_path_buf();
        assert_eq!(Places::of(&root, &temp), None);
        Ok(())
    }

    /// A folder holding the temporary folder is refused: setting it up would index the temporary
    /// folder too.
    #[test]
    fn a_folder_holding_the_temporary_folder_is_refused() -> TestResult {
        let base = tempfile::tempdir()?;
        let home = base.path().join("home");
        let temp = home.join("local").join("temp");
        std::fs::create_dir_all(&temp)?;
        std::fs::create_dir_all(home.join("local").join(".git"))?;
        let places = Places::of(&home, &temp).ok_or("no home")?;
        assert_eq!(
            adoption_candidate(&home.join("local"), &places),
            Err(Refusal::Temporary)
        );
        Ok(())
    }

    /// A `.cortex` holding `off` at the folder or anywhere above it below home refuses it; a
    /// `.cortex` holding anything else, or an `off` folder, does not.
    #[test]
    fn off_at_or_above_refuses() -> TestResult {
        let (_base, places) = places()?;
        let outer = places.home.join("outer");
        let repo = outer.join("repo");
        std::fs::create_dir_all(repo.join(".git"))?;
        std::fs::create_dir_all(outer.join(MARKER).join(OFF_FILE))?;
        assert!(adoption_candidate(&repo, &places).is_ok());
        std::fs::remove_dir(outer.join(MARKER).join(OFF_FILE))?;
        std::fs::write(outer.join(MARKER).join(OFF_FILE), "")?;
        assert_eq!(adoption_candidate(&repo, &places), Err(Refusal::TurnedOff));
        std::fs::remove_file(outer.join(MARKER).join(OFF_FILE))?;
        std::fs::create_dir_all(repo.join(MARKER))?;
        std::fs::write(repo.join(MARKER).join(OFF_FILE), "")?;
        assert_eq!(adoption_candidate(&repo, &places), Err(Refusal::TurnedOff));
        Ok(())
    }
}
