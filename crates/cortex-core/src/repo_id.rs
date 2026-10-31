//! The id of a repository on this machine.

use std::fmt;
use std::path::Path;

use crate::path::without_verbatim_prefix;

/// Number of hexadecimal characters in a repository id: short enough to keep the daemon's socket
/// path under the macOS length limit, long enough that two repositories on one machine do not
/// collide.
const REPO_ID_LEN: usize = 16;

/// The id of a repository: the first 16 hexadecimal characters of the BLAKE3 hash of its canonical
/// root path. It names the daemon's socket, lock and process-id files, and is stored with the
/// index.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoId(String);

impl RepoId {
    /// The id of the repository whose canonical root is `root` (see
    /// [`canonical_root`](crate::canonical_root)). The Windows verbatim prefix (`\\?\`) that
    /// canonicalizing adds is left out of the hash, so both spellings of one root give one id.
    #[must_use]
    pub fn of(root: &Path) -> Self {
        let root = without_verbatim_prefix(root);
        let mut id = crate::ContentHash::of(root.as_os_str().as_encoded_bytes()).to_string();
        id.truncate(REPO_ID_LEN);
        Self(id)
    }

    /// The id as text: 16 lower-case hexadecimal characters.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoId {
    /// The 16 hexadecimal characters.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Debug for RepoId {
    /// The 16 hexadecimal characters, inside the type name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RepoId({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An id is 16 lower-case hexadecimal characters, the same for the same root and for its
    /// verbatim spelling, and different for another root.
    #[test]
    fn ids_are_short_stable_and_spelling_blind() {
        let id = RepoId::of(Path::new(r"C:\work\repo"));
        assert_eq!(id.as_str().len(), 16);
        assert!(
            id.as_str()
                .chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
        );
        assert_eq!(RepoId::of(Path::new(r"C:\work\repo")), id);
        assert_eq!(RepoId::of(Path::new(r"\\?\C:\work\repo")), id);
        assert_eq!(
            RepoId::of(Path::new(r"\\?\UNC\host\share\repo")),
            RepoId::of(Path::new(r"\\host\share\repo"))
        );
        assert_ne!(RepoId::of(Path::new(r"C:\work\other")), id);
    }

    /// The id is the hash's leading hexadecimal: for a root spelled as the empty path, the first
    /// 16 characters of the published BLAKE3 hash of no bytes.
    #[test]
    fn id_is_the_leading_hash_text() {
        assert_eq!(RepoId::of(Path::new("")).as_str(), "af1349b9f5f9a1a6");
    }
}
