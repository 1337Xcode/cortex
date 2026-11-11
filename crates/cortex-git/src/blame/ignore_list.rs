//! The repository's own list of commits blame skips, `.git-blame-ignore-revs` in its top folder:
//! whether it is a list git can read and one that stays inside the repository.

use std::io::Read as _;
use std::path::Path;

use cortex_core::is_object_id;

use crate::git_program::IGNORE_REVS_FILE;

/// The most of the list read, 1 MiB, about 25,000 ids; a longer file is not used.
const MAX_LIST_BYTES: u64 = 1024 * 1024;

/// Whether the top of the repository at `root` holds [`IGNORE_REVS_FILE`] as a regular file (a
/// link could lead out of the repository) of UTF-8 text within [`MAX_LIST_BYTES`] that git can
/// read whole: see [`is_list_of_full_ids`].
pub(super) fn is_usable(root: &Path) -> bool {
    let path = root.join(IGNORE_REVS_FILE);
    let Ok(kind) = std::fs::symlink_metadata(&path) else {
        return false;
    };
    if !kind.is_file() {
        return false;
    }
    let Ok(file) = std::fs::File::open(&path) else {
        return false;
    };
    let mut text = String::new();
    let read = file
        .take(MAX_LIST_BYTES + 1)
        .read_to_string(&mut text)
        .is_ok();
    read && u64::try_from(text.len()).is_ok_and(|len| len <= MAX_LIST_BYTES)
        && is_list_of_full_ids(&text)
}

/// Whether every line of `text` is blank, a comment (from a `#` to the line's end, which may
/// follow an id) or a full object id. Git refuses a list with a line that is anything else, an
/// abbreviated id included, and then fails every blame that reads it. Git takes an id in either
/// case here, so a list written by hand in upper case is used too.
fn is_list_of_full_ids(text: &str) -> bool {
    text.lines().all(|line| {
        let id = line.split('#').next().unwrap_or_default().trim_end();
        id.is_empty() || is_object_id(&id.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full id, 40 hexadecimal digits.
    const ID: &str = "0123456789abcdef0123456789abcdef01234567";

    /// Lines git reads (full ids in either case, blank lines, comments, a comment after an id,
    /// carriage returns)
    /// and lines it refuses (an abbreviated id, an id with text after it, a word, an id with white
    /// space before it, an id that is too long).
    #[test]
    fn only_comments_blanks_and_full_ids_make_a_list() {
        let good = [
            String::new(),
            format!("{ID}\n"),
            format!("# a bot\n\n{ID}   # the bump\n{ID}\r\n"),
            "   \n#\n".to_owned(),
            "0123456789ABCDEF0123456789ABCDEF01234567\n".to_owned(),
        ];
        for text in good {
            assert!(is_list_of_full_ids(&text), "{text:?}");
        }
        let bad = [
            "0123abc\n".to_owned(),
            format!("{ID} bump\n"),
            "HEAD\n".to_owned(),
            format!(" {ID}\n"),
            format!("{ID}0\n"),
            format!("# fine\n{}\n", &ID[..39]),
        ];
        for text in bad {
            assert!(!is_list_of_full_ids(&text), "{text:?}");
        }
    }
}
