//! The listing `git log` prints for the history source run as a program: per commit, a record
//! mark, its id and commit time, then each changed file as `--name-status -z` writes it, a status
//! and the path, or for a rename the status, the path before and the path after. Read here into
//! commits without running anything; [`crate::cli_history`] runs git and decides what each one
//! means.

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
    /// A commit's head is not an id and a time.
    #[error("a commit's head in git's listing is not an id and a time")]
    Head,
    /// A changed file has no status or is missing a path.
    #[error("a changed file in git's listing of commit {0} is not a status and its paths")]
    Change(String),
}

/// The commits `listing` holds, in its order.
///
/// # Errors
///
/// The listing is not in the format asked for.
pub(crate) fn parse(listing: &[u8]) -> Result<Vec<Listed>, ListingError> {
    listing
        .split(|byte| *byte == RECORD_START)
        .skip(1)
        .map(parse_commit)
        .collect()
}

/// One commit's record: its head up to the first NUL, then each changed file's status and paths,
/// each ended by a NUL. Git puts a line break between the head and the first status.
fn parse_commit(record: &[u8]) -> Result<Listed, ListingError> {
    let mut fields = record.split(|byte| *byte == 0);
    let head = fields.next().ok_or(ListingError::Head)?;
    let head = std::str::from_utf8(head).map_err(|_| ListingError::Head)?;
    let (id, time_s) = head.split_once(' ').ok_or(ListingError::Head)?;
    let time_s = time_s.parse().map_err(|_| ListingError::Head)?;
    let is_id = id.len() >= 40 && id.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !is_id {
        return Err(ListingError::Head);
    }
    let broken = || ListingError::Change(id.to_owned());
    let mut changes = Vec::new();
    while let Some(field) = fields.next() {
        let status = field.strip_prefix(b"\n").unwrap_or(field);
        let Some(&kind) = status.first() else {
            continue;
        };
        // Git never lists an empty path; the field after a record's last NUL is empty.
        let mut next_path = || {
            fields
                .next()
                .filter(|path| !path.is_empty())
                .map(<[u8]>::to_vec)
                .ok_or_else(broken)
        };
        let first = next_path()?;
        // A rename names the path before, then the path after; every other status one path.
        changes.push(if kind == b'R' {
            (next_path()?, Some(first))
        } else {
            (first, None)
        });
    }
    Ok(Listed {
        id: id.to_owned(),
        time_s,
        changes,
    })
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
    }
}
