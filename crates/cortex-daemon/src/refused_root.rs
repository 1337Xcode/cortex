//! The refusal to serve the user's home folder, a folder holding it, or a file system's root as a
//! repository (see [`cortex_core::is_never_a_root`]): the home folder's `.cortex` is the user's
//! own, and a daemon at a file system's root would index everything below it. The commands that
//! start a daemon refuse such a root first; the daemon refuses it too, whoever started it.

use std::fmt;
use std::path::{Path, PathBuf};

use cortex_core::is_never_a_root;

/// A root the daemon refuses: the user's home folder, a folder holding it, or a file system's
/// root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedRoot {
    /// The root asked for, canonical.
    pub root: PathBuf,
}

impl fmt::Display for RefusedRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} is the home folder, a folder holding it, or a file system's root, which Cortex \
             never serves as a repository",
            self.root.display()
        )
    }
}

impl std::error::Error for RefusedRoot {}

/// Refuses `root` when it can never be a repository's root for the user whose home folder is
/// `home`, both canonical.
///
/// # Errors
///
/// The root is the home folder, holds it, or is a file system's root.
pub(crate) fn refuse(root: &Path, home: &Path) -> Result<(), RefusedRoot> {
    if is_never_a_root(root, home) {
        return Err(RefusedRoot {
            root: root.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The home folder, each folder above it and so the file system's root are refused, each
    /// named; a folder inside the home folder, or beside it with a name that starts the same, is
    /// not.
    #[test]
    fn the_home_folder_what_holds_it_and_a_file_system_root_are_refused() {
        let home = std::env::temp_dir().join("users").join("ann");
        for root in home.ancestors() {
            assert_eq!(
                refuse(root, &home),
                Err(RefusedRoot {
                    root: root.to_path_buf()
                }),
                "{}",
                root.display()
            );
        }
        assert_eq!(refuse(&home.join("work").join("repo"), &home), Ok(()));
        let beside = std::env::temp_dir().join("users").join("anna");
        assert_eq!(refuse(&beside, &home), Ok(()));
    }

    /// The refusal names the root and why.
    #[test]
    fn the_refusal_says_why() {
        let refused = RefusedRoot {
            root: PathBuf::from("/home/ann"),
        };
        assert_eq!(
            refused.to_string(),
            format!(
                "{} is the home folder, a folder holding it, or a file system's root, which \
                 Cortex never serves as a repository",
                Path::new("/home/ann").display()
            )
        );
    }
}
