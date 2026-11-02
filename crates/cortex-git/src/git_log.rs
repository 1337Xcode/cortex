//! The listing `git log` prints for the history source run as a program: per commit, a record
//! mark, its id and commit time, then each changed file as `--name-status -z` writes it, a status
//! and the path, or for a rename the status, the path before and the path after, every field
//! ended by a NUL. Read here into commits without running anything; [`crate::cli_history`] runs
//! git and decides what each one means.
//!
//! A path may hold any byte but NUL, the record mark and line breaks included, so the listing is
//! read field by field: a field is a path only where a path comes, right after its status, and
//! a commit starts only where a status could.

use thiserror::Error;

/// What starts each commit in the listing; [`FORMAT`] writes it first.
const RECORD_START: u8 = 0x01;

/// The `--format` that writes each commit's head: the record mark, the full id and the commit
/// time in seconds since the Unix epoch.
pub(crate) const FORMAT: &str = "--format=%x01%H %ct";

/// One commit of the listing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Listed {
    /// The commit's id, in lowercase hexadecimal.
    pub(crate) id: String,
    /// When it was committed, in seconds since the Unix epoch.
    pub(crate) time_s: i64,
    /// Each file it changed, in git's order: the path, and for a rename the path before.
    pub(crate) changes: Vec<(Vec<u8>, Option<Vec<u8>>)>,
}

/// Why a listing cannot be read: git printed something other than the format asked for.
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ListingError {
    /// A commit's head is not an id and a time, or a changed file comes before any commit.
    #[error("a commit's head in git's listing is not an id and a time")]
    Head,
    /// A changed file's status is not one, or a path is missing.
    #[error("a changed file in git's listing of commit {0} is not a status and its paths")]
    Change(String),
}

/// The commits `listing` holds, in its order.
///
/// # Errors
///
/// The listing is not in the format asked for.
pub(crate) fn parse(listing: &[u8]) -> Result<Vec<Listed>, ListingError> {
    let mut fields = listing.split(|byte| *byte == 0);
    let mut listed: Vec<Listed> = Vec::new();
    while let Some(field) = fields.next() {
        // Git writes a line break between a commit's head and its first status.
        let field = field.strip_prefix(b"\n").unwrap_or(field);
        if let Some(head) = field.strip_prefix(&[RECORD_START]) {
            listed.push(parse_head(head)?);
            continue;
        }
        // The field after the listing's last NUL.
        if field.is_empty() {
            continue;
        }
        let commit = listed.last_mut().ok_or(ListingError::Head)?;
        let broken = || ListingError::Change(commit.id.clone());
        if !is_status(field) {
            return Err(broken());
        }
        let mut next_path = || {
            fields
                .next()
                .filter(|path| !path.is_empty())
                .map(<[u8]>::to_vec)
                .ok_or_else(broken)
        };
        let first = next_path()?;
        // A rename names the path before, then the path after; every other status one path.
        let change = if field.first() == Some(&b'R') {
            (next_path()?, Some(first))
        } else {
            (first, None)
        };
        commit.changes.push(change);
    }
    Ok(listed)
}

/// A commit's head after the record mark: its full id, a space and its commit time.
fn parse_head(head: &[u8]) -> Result<Listed, ListingError> {
    let head = std::str::from_utf8(head).map_err(|_| ListingError::Head)?;
    let (id, time_s) = head.split_once(' ').ok_or(ListingError::Head)?;
    let time_s = time_s.parse().map_err(|_| ListingError::Head)?;
    let is_id = id.len() >= 40 && id.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !is_id {
        return Err(ListingError::Head);
    }
    Ok(Listed {
        id: id.to_owned(),
        time_s,
        changes: Vec::new(),
    })
}

/// Whether `field` is a status `--name-status` writes: a capital letter, then a similarity for a
/// rename or a copy.
fn is_status(field: &[u8]) -> bool {
    match field.split_first() {
        Some((kind, rest)) => kind.is_ascii_uppercase() && rest.iter().all(u8::is_ascii_digit),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listing of two commits as git writes it with `-z`: the newer renames `a.txt` to `b.txt`
    /// and modifies `c.txt`, the older adds both; read back as written.
    #[test]
    fn a_listing_reads_as_written() -> Result<(), ListingError> {
        let newer = "1111111111111111111111111111111111111111";
        let older = "2222222222222222222222222222222222222222";
        let mut listing = Vec::new();
        listing.extend_from_slice(
            format!("\x01{newer} 1767398400\0\nR087\0a.txt\0b.txt\0M\0c.txt\0").as_bytes(),
        );
        listing.extend_from_slice(
            format!("\x01{older} 1767312000\0\nA\0a.txt\0A\0c.txt\0").as_bytes(),
        );
        let read = parse(&listing)?;
        assert_eq!(
            read,
            vec![
                Listed {
                    id: newer.to_owned(),
                    time_s: 1_767_398_400,
                    changes: vec![
                        (b"b.txt".to_vec(), Some(b"a.txt".to_vec())),
                        (b"c.txt".to_vec(), None),
                    ],
                },
                Listed {
                    id: older.to_owned(),
                    time_s: 1_767_312_000,
                    changes: vec![(b"a.txt".to_vec(), None), (b"c.txt".to_vec(), None)],
                },
            ]
        );
        Ok(())
    }

    /// A commit with no changed file, a path that is not UTF-8 kept as bytes, and listings git
    /// would never print: a head without a time, an id that is not one, a rename missing its
    /// second path.
    #[test]
    fn odd_listings_read_or_fail_plainly() {
        let id = "3333333333333333333333333333333333333333";
        let empty = format!("\x01{id} 5\0");
        assert_eq!(
            parse(empty.as_bytes()),
            Ok(vec![Listed {
                id: id.to_owned(),
                time_s: 5,
                changes: vec![],
            }])
        );
        let mut raw = format!("\x01{id} 5\0\nA\0").into_bytes();
        raw.extend_from_slice(b"bad\xff.txt\0");
        assert_eq!(
            parse(&raw).map(|read| read
                .into_iter()
                .flat_map(|listed| listed.changes)
                .collect::<Vec<_>>()),
            Ok(vec![(b"bad\xff.txt".to_vec(), None)])
        );
        assert_eq!(
            parse(format!("\x01{id}\0").as_bytes()),
            Err(ListingError::Head)
        );
        assert_eq!(parse(b"\x01HEAD 5\0"), Err(ListingError::Head));
        assert_eq!(
            parse(format!("\x01{id} 5\0\nR100\0a.txt\0").as_bytes()),
            Err(ListingError::Change(id.to_owned()))
        );
        assert_eq!(parse(b"\nA\0a.txt\0"), Err(ListingError::Head));
        assert_eq!(
            parse(format!("\x01{id} 5\0\nnot a status\0a.txt\0").as_bytes()),
            Err(ListingError::Change(id.to_owned()))
        );
    }

    /// A path may hold the record mark and line breaks: where a path comes it is a path, so a
    /// file named to look like a commit's head adds no commit, and an empty commit between two
    /// others is read as one.
    #[test]
    fn a_path_cannot_start_a_commit() -> Result<(), ListingError> {
        let genuine = "4444444444444444444444444444444444444444";
        let forged = "5555555555555555555555555555555555555555";
        let empty = "6666666666666666666666666666666666666666";
        let older = "7777777777777777777777777777777777777777";
        let listing = format!(
            "\x01{genuine} 30\0\nA\0z\x01{forged} 2000000000\0A\0\nzz.txt\0\x01{empty} 20\0\x01{older} 10\0\nA\0a.txt\0"
        );
        let commits = parse(listing.as_bytes())?;
        let heads: Vec<(&str, i64, usize)> = commits
            .iter()
            .map(|listed| (listed.id.as_str(), listed.time_s, listed.changes.len()))
            .collect();
        assert_eq!(heads, [(genuine, 30, 2), (empty, 20, 0), (older, 10, 1)]);
        let paths: Vec<Vec<u8>> = commits
            .into_iter()
            .flat_map(|listed| listed.changes)
            .map(|(path, _)| path)
            .collect();
        assert_eq!(
            paths,
            [
                format!("z\x01{forged} 2000000000").into_bytes(),
                b"\nzz.txt".to_vec(),
                b"a.txt".to_vec(),
            ]
        );
        Ok(())
    }
}
