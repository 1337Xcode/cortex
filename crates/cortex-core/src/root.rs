//! The canonical form of a repository's root folder.
//!
//! A repository is known by its root folder: the root's hash names its daemon's socket and is
//! stored with its index. A folder has many spellings (a relative path, `..` segments, a link to
//! it, another letter case on a file system that ignores case), so everything that names a
//! repository by its root first passes the root through [`canonical_root`]. Some folders are never
//! a repository's root ([`is_never_a_root`]).

use std::io;
use std::path::{Path, PathBuf};

use crate::path::without_verbatim_prefix;

/// The canonical form of the folder at `folder`: absolute, with every link, `.` and `..` resolved
/// and each name spelled as the file system stores it. On Windows the verbatim prefix (`\\?\`)
/// that canonicalizing adds is left out, so the result reads as people write paths. Two spellings
/// of one folder give one result.
///
/// # Errors
///
/// The folder does not exist or cannot be reached (the error canonicalizing gives), or `folder`
/// names something other than a folder ([`io::ErrorKind::NotADirectory`]).
pub fn canonical_root(folder: &Path) -> io::Result<PathBuf> {
    let canonical = std::fs::canonicalize(folder)?;
    if !canonical.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a folder", folder.display()),
        ));
    }
    Ok(without_verbatim_prefix(&canonical))
}

/// Whether `folder`, canonical, can never be a repository's root for the user whose canonical home
/// folder is `home`: it is the home folder, whose `.cortex` is the user's own, a folder holding
/// it, or a file system's root (a folder with no parent: `/`, `C:\`, a network share's root),
/// below which everything would be indexed. Every command that serves, sets up or deletes a
/// repository refuses such a folder.
///
/// ```
/// use std::path::Path;
/// use cortex_core::is_never_a_root;
/// let home = Path::new("/home/ann");
/// assert!(is_never_a_root(Path::new("/home/ann"), home));
/// assert!(is_never_a_root(Path::new("/home"), home));
/// assert!(is_never_a_root(Path::new("/"), home));
/// assert!(!is_never_a_root(Path::new("/home/ann/repo"), home));
/// assert!(!is_never_a_root(Path::new("/srv/repo"), home));
/// ```
#[must_use]
pub fn is_never_a_root(folder: &Path, home: &Path) -> bool {
    home.starts_with(folder) || folder.parent().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RepoId;

    /// This system's own file system root, the temporary folder's outermost folder, is never a
    /// root, nor is the home folder or what holds it; a folder inside the home folder or beside
    /// it, its name starting the same, can be.
    #[test]
    fn the_home_folder_what_holds_it_and_a_file_system_root_are_never_roots() -> TestResult {
        let scratch = tempfile::tempdir()?;
        let home = canonical_root(scratch.path())?.join("users").join("ann");
        let file_system_root = home.ancestors().last().ok_or("no outermost folder")?;
        assert!(
            file_system_root.parent().is_none(),
            "{}",
            file_system_root.display()
        );
        assert!(is_never_a_root(
            file_system_root,
            Path::new("/elsewhere/home")
        ));
        for folder in home.ancestors() {
            assert!(is_never_a_root(folder, &home), "{}", folder.display());
        }
        assert!(!is_never_a_root(&home.join("repo"), &home));
        let beside = home.with_file_name("anna");
        assert!(!is_never_a_root(&beside, &home));
        Ok(())
    }

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A folder spelled through a subfolder and `..`, and through `.`, has the canonical form of
    /// its plain spelling, and so the same repository id; on Windows, so has its name in upper
    /// case, since the file system ignores case.
    #[test]
    fn spellings_of_one_folder_agree() -> TestResult {
        let scratch = tempfile::tempdir()?;
        let repo = scratch.path().join("repo");
        std::fs::create_dir_all(repo.join("sub"))?;
        let plain = canonical_root(&repo)?;
        let mut spellings = vec![repo.join("sub").join(".."), repo.join(".")];
        if cfg!(windows) {
            let upper = repo.to_str().ok_or("a path that is not UTF-8")?;
            spellings.push(PathBuf::from(upper.to_uppercase()));
        }
        for spelling in spellings {
            let canonical = canonical_root(&spelling)?;
            assert_eq!(canonical, plain, "{}", spelling.display());
            assert_eq!(RepoId::of(&canonical), RepoId::of(&plain));
        }
        Ok(())
    }

    /// A link to the folder gives the folder's own canonical form.
    #[cfg(unix)]
    #[test]
    fn a_link_names_the_folder_it_points_to() -> TestResult {
        let scratch = tempfile::tempdir()?;
        let repo = scratch.path().join("repo");
        std::fs::create_dir(&repo)?;
        let link = scratch.path().join("link");
        std::os::unix::fs::symlink(&repo, &link)?;
        assert_eq!(canonical_root(&link)?, canonical_root(&repo)?);
        Ok(())
    }

    /// A junction to the folder, the link any Windows user can make, gives the folder's own
    /// canonical form, which carries no verbatim prefix.
    #[cfg(windows)]
    #[test]
    fn a_junction_names_the_folder_it_points_to() -> TestResult {
        let scratch = tempfile::tempdir()?;
        let repo = scratch.path().join("repo");
        std::fs::create_dir(&repo)?;
        let junction = scratch.path().join("junction");
        let made = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&repo)
            .output()?;
        assert!(made.status.success(), "{made:?}");
        let canonical = canonical_root(&junction)?;
        assert_eq!(canonical, canonical_root(&repo)?);
        let text = canonical.to_str().ok_or("a path that is not UTF-8")?;
        assert!(!text.starts_with(r"\\?\"), "{text}");
        Ok(())
    }

    /// A missing folder and a file are refused, each with its own kind of error.
    #[test]
    fn missing_folders_and_files_are_refused() -> TestResult {
        let scratch = tempfile::tempdir()?;
        let missing = canonical_root(&scratch.path().join("missing"));
        assert_eq!(
            missing.err().map(|error| error.kind()),
            Some(io::ErrorKind::NotFound)
        );
        let file = scratch.path().join("file.txt");
        std::fs::write(&file, "text")?;
        let refused = canonical_root(&file);
        assert_eq!(
            refused.err().map(|error| error.kind()),
            Some(io::ErrorKind::NotADirectory)
        );
        Ok(())
    }
}
