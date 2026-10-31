//! The canonical form of a repository's root folder.
//!
//! A repository is known by its root folder: the root's hash names its daemon's socket and is
//! stored with its index. A folder has many spellings (a relative path, `..` segments, a link to
//! it, another letter case on a file system that ignores case), so everything that names a
//! repository by its root first passes the root through [`canonical_root`].

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RepoId;

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
