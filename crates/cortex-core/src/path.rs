//! Paths inside a repository.
//!
//! Every path Cortex stores, compares or sends is relative to the repository root and uses `/`
//! between folders, whatever the platform. A native path is converted once, where it enters
//! (the file walk, a hook payload), and converted back only to open the file.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// Why a path cannot be a repository path.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    /// The text starts at a root or a drive, so it is not relative.
    #[error("{0:?} is absolute, not relative to the repository root")]
    Absolute(String),
    /// The text holds a `\`, which Cortex never writes inside a stored path.
    #[error("{0:?} holds a backslash; repository paths use `/`")]
    Backslash(String),
    /// A folder name is empty, `.` or `..`.
    #[error("{0:?} has an empty, `.` or `..` component")]
    BadComponent(String),
    /// The native path lies outside the repository root.
    #[error("{path} is not inside {root}")]
    Outside {
        /// The path that was converted.
        path: String,
        /// The repository root it was converted against.
        root: String,
    },
    /// A folder name is not valid UTF-8, so it cannot be stored as text.
    #[error("{0} has a name that is not valid UTF-8")]
    NotUtf8(String),
}

/// A path inside a repository: relative to its root, `/` between folders, no empty, `.` or `..`
/// components. The empty path is the root itself.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPath(String);

impl RepoPath {
    /// The repository root itself.
    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Reads a repository-relative path written with `/`, such as `src/lib.rs`.
    ///
    /// # Errors
    ///
    /// Refuses absolute paths, drive prefixes, backslashes and empty, `.` or `..` components.
    pub fn new(text: &str) -> Result<Self, PathError> {
        if text.is_empty() {
            return Ok(Self::root());
        }
        if text.starts_with('/') || has_drive_prefix(text) {
            return Err(PathError::Absolute(text.to_owned()));
        }
        if text.contains('\\') {
            return Err(PathError::Backslash(text.to_owned()));
        }
        if text
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(PathError::BadComponent(text.to_owned()));
        }
        Ok(Self(text.to_owned()))
    }

    /// The repository path of `path`, which must lie inside `root`. Either may carry the Windows
    /// verbatim prefix (`\\?\C:\`, `\\?\UNC\`) that `canonicalize` adds; the plain and verbatim
    /// spellings of one folder give the same result.
    ///
    /// # Errors
    ///
    /// Refuses a path outside `root`, a `.` or `..` component after the root, and names that are
    /// not valid UTF-8.
    pub fn from_native(root: &Path, path: &Path) -> Result<Self, PathError> {
        let root = without_verbatim_prefix(root);
        let full = without_verbatim_prefix(path);
        let relative = full.strip_prefix(&root).map_err(|_| PathError::Outside {
            path: path.display().to_string(),
            root: root.display().to_string(),
        })?;
        let mut names = Vec::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(PathError::BadComponent(path.display().to_string()));
            };
            let name = name
                .to_str()
                .ok_or_else(|| PathError::NotUtf8(path.display().to_string()))?;
            names.push(name);
        }
        Self::new(&names.join("/"))
    }

    /// The path as stored, such as `src/lib.rs`; empty for the root.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the repository root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The folder holding this path: `None` for the root, the root for a top-level name.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rsplit_once('/') {
            Some((parent, _)) => Self(parent.to_owned()),
            None => Self::root(),
        })
    }

    /// The last component, such as `lib.rs`; empty for the root.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or_default()
    }

    /// The text after the last `.` of the file name, if the name has one that does not start it.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name();
        let (stem, extension) = name.rsplit_once('.')?;
        (!stem.is_empty()).then_some(extension)
    }

    /// This path with one more component.
    ///
    /// # Errors
    ///
    /// Refuses a name that is empty, `.`, `..` or holds a separator.
    pub fn join(&self, name: &str) -> Result<Self, PathError> {
        if name.contains('/') {
            return Err(PathError::BadComponent(name.to_owned()));
        }
        if self.is_root() {
            Self::new(name)
        } else {
            Self::new(&format!("{}/{name}", self.0))
        }
    }

    /// The native path of this repository path under `root`.
    #[must_use]
    pub fn to_native(&self, root: &Path) -> PathBuf {
        self.0
            .split('/')
            .filter(|component| !component.is_empty())
            .fold(root.to_path_buf(), |path, component| path.join(component))
    }
}

/// Whether `text` starts with a drive such as `C:`.
fn has_drive_prefix(text: &str) -> bool {
    let mut characters = text.chars();
    matches!(
        (characters.next(), characters.next()),
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic()
    )
}

/// `path` without the Windows verbatim prefix: `\\?\C:\x` becomes `C:\x` and `\\?\UNC\host\x`
/// becomes `\\host\x`. Other paths are returned as they are.
fn without_verbatim_prefix(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if has_drive_prefix(rest) => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

impl fmt::Display for RepoPath {
    /// The stored form, `/` between folders.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Debug for RepoPath {
    /// The stored form, quoted.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RepoPath({:?})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative paths with `/` are accepted as written; every other form is refused.
    #[test]
    fn new_accepts_only_relative_slash_paths() {
        for good in ["src/lib.rs", "README.md", "a/b/c.d.ts", ".github/x.yml", ""] {
            assert_eq!(
                RepoPath::new(good).map(|path| path.as_str().to_owned()),
                Ok(good.to_owned())
            );
        }
        let bad = [
            ("/etc/passwd", PathError::Absolute("/etc/passwd".to_owned())),
            ("C:/x", PathError::Absolute("C:/x".to_owned())),
            (
                "src\\lib.rs",
                PathError::Backslash("src\\lib.rs".to_owned()),
            ),
            (
                "src//lib.rs",
                PathError::BadComponent("src//lib.rs".to_owned()),
            ),
            (
                "src/./lib.rs",
                PathError::BadComponent("src/./lib.rs".to_owned()),
            ),
            ("../x", PathError::BadComponent("../x".to_owned())),
            ("src/", PathError::BadComponent("src/".to_owned())),
        ];
        for (text, error) in bad {
            assert_eq!(RepoPath::new(text), Err(error), "{text}");
        }
    }

    /// Parent, file name and extension, worked out by hand.
    #[test]
    fn parts_of_a_path() -> Result<(), PathError> {
        let path = RepoPath::new("src/index/lib.rs")?;
        assert_eq!(path.parent(), Some(RepoPath::new("src/index")?));
        assert_eq!(path.file_name(), "lib.rs");
        assert_eq!(path.extension(), Some("rs"));
        let top = RepoPath::new("Makefile")?;
        assert_eq!(top.parent(), Some(RepoPath::root()));
        assert_eq!(top.extension(), None);
        assert_eq!(RepoPath::new(".gitignore")?.extension(), None);
        assert_eq!(RepoPath::new("a.tar.gz")?.extension(), Some("gz"));
        assert_eq!(RepoPath::root().parent(), None);
        assert_eq!(RepoPath::root().join("a")?.join("b")?.as_str(), "a/b");
        assert!(RepoPath::root().join("a/b").is_err());
        Ok(())
    }

    /// A native path inside the root converts to `/` form; outside, or with `..`, it is refused.
    #[test]
    fn native_paths_convert_inside_the_root() -> Result<(), PathError> {
        let root = PathBuf::from("repo-root");
        let inside = root.join("src").join("lib.rs");
        assert_eq!(
            RepoPath::from_native(&root, &inside)?.as_str(),
            "src/lib.rs"
        );
        assert_eq!(RepoPath::from_native(&root, &root)?, RepoPath::root());
        let outside = PathBuf::from("elsewhere").join("x.rs");
        assert!(matches!(
            RepoPath::from_native(&root, &outside),
            Err(PathError::Outside { .. })
        ));
        let escaping = root.join("..").join("x.rs");
        assert!(RepoPath::from_native(&root, &escaping).is_err());
        assert_eq!(
            RepoPath::new("src/lib.rs")?.to_native(&root),
            root.join("src").join("lib.rs")
        );
        Ok(())
    }

    /// The verbatim prefix is removed from drive and network spellings, worked out by hand; other
    /// paths are left alone.
    #[test]
    fn verbatim_prefixes_are_removed() {
        let cases = [
            (r"\\?\C:\work\repo", r"C:\work\repo"),
            (r"\\?\UNC\host\share\x", r"\\host\share\x"),
            (r"C:\work\repo", r"C:\work\repo"),
            ("/home/dev/repo", "/home/dev/repo"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                without_verbatim_prefix(Path::new(input)),
                PathBuf::from(expected)
            );
        }
    }

    /// The verbatim and plain spellings of a root or a path give the same repository path.
    // Backslash-separated drive paths only split into components on Windows, so this case has no
    // meaning elsewhere.
    #[cfg(windows)]
    #[test]
    fn verbatim_and_plain_spellings_agree() -> Result<(), PathError> {
        let plain_root = Path::new(r"C:\work\repo");
        let verbatim_root = Path::new(r"\\?\C:\work\repo");
        let plain = Path::new(r"C:\work\repo\src\lib.rs");
        let verbatim = Path::new(r"\\?\C:\work\repo\src\lib.rs");
        for (root, path) in [
            (plain_root, plain),
            (plain_root, verbatim),
            (verbatim_root, plain),
            (verbatim_root, verbatim),
        ] {
            assert_eq!(RepoPath::from_native(root, path)?.as_str(), "src/lib.rs");
        }
        Ok(())
    }
}
