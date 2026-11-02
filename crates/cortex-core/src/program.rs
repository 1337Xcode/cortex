//! Finding another program Cortex runs on the `PATH`, as a shell finds it, but never inside the
//! repository, which could have placed one there.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The first program named one of `names` in the folders the `PATH` environment variable lists,
/// each folder in turn and the names in order within it, as a shell finds it, but never one inside
/// `repository`, which could have placed it there.
#[must_use]
pub fn find_program(names: &[&str], repository: &Path) -> Option<PathBuf> {
    found_in(&std::env::var_os("PATH")?, names, repository)
}

/// The first program named one of `names` in the folders `path`, a `PATH` value, lists and
/// [`searched`] keeps, outside `repository`; none when `repository` has no real path.
fn found_in(path: &OsStr, names: &[&str], repository: &Path) -> Option<PathBuf> {
    let repository = repository.canonicalize().ok()?;
    searched(path)
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|candidate| is_runnable(candidate) && is_outside(candidate, &repository))
}

/// The folders `path`, a `PATH` value, lists that are searched: the absolute ones. An empty entry
/// or `.` stands for the current folder, which is often the repository.
fn searched(path: &OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(path).filter(|folder| folder.is_absolute())
}

/// Whether `candidate`, its links followed, lies outside `folder`, a real path. One whose real
/// path cannot be found is taken as inside, and so is not run.
fn is_outside(candidate: &Path, folder: &Path) -> bool {
    candidate
        .canonicalize()
        .is_ok_and(|real| !real.starts_with(folder))
}

/// Whether `candidate` is a file this user may run.
#[cfg(unix)]
fn is_runnable(candidate: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    candidate
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// Whether `candidate` is a file: Windows runs a file by its name's extension, which the names
/// looked for carry.
#[cfg(not(unix))]
fn is_runnable(candidate: &Path) -> bool {
    candidate.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a program at `path` that does nothing.
    fn write_program(path: &Path) -> std::io::Result<()> {
        if cfg!(windows) {
            std::fs::write(path, "@echo off\r\n")
        } else {
            std::fs::write(path, "#!/bin/sh\n")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
            }
            Ok(())
        }
    }

    /// Of the folders a `PATH` lists, a relative one is not searched, one inside the repository
    /// holds a program that is passed over, one holding only a folder of the program's name is
    /// passed over, and the next folder's program is the one found; with none outside the
    /// repository, nothing is.
    #[test]
    fn programs_are_found_outside_the_repository() -> Result<(), Box<dyn std::error::Error>> {
        let scratch = tempfile::tempdir()?;
        let repository = scratch.path().join("repo");
        let inside = repository.join("bin");
        let (first, second) = (scratch.path().join("first"), scratch.path().join("second"));
        let name = if cfg!(windows) { "tool.cmd" } else { "tool" };
        std::fs::create_dir_all(first.join(name))?;
        for folder in [&inside, &second] {
            std::fs::create_dir_all(folder)?;
            write_program(&folder.join(name))?;
        }
        let path = std::env::join_paths([
            PathBuf::from("bin"),
            inside.clone(),
            first.clone(),
            second.clone(),
        ])?;
        assert_eq!(
            searched(&path).collect::<Vec<_>>(),
            [inside.clone(), first.clone(), second.clone()]
        );
        assert_eq!(
            found_in(&path, &[name], &repository),
            Some(second.join(name))
        );
        let path = std::env::join_paths([inside, first])?;
        assert_eq!(found_in(&path, &[name], &repository), None);
        Ok(())
    }
}
