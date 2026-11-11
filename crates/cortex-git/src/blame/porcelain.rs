//! Git's `blame --porcelain` output read into runs of lines and the commits that own them.
//!
//! Each line of the blamed range has a header: the commit's id, the line's number in the commit's
//! file and in the final file, and, on the first line of a group the same commit owns, how many
//! lines the group has. The first time a commit appears its details follow: author, committer,
//! time, summary, maybe `previous <id> <path>` and `boundary`, and last `filename <path>`. A
//! commit seen again has a header only. Then comes the line's text after a tab. Lines of
//! details that are not named here are skipped, as git's documentation asks of a reader.

use std::collections::BTreeMap;

use cortex_core::is_object_id;

use super::BlameError;

/// The id of the commit before the one a line was last changed in, and the path the file had
/// there: where `git blame` would look next to see what the line was before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Previous {
    /// The commit that came before.
    pub oid: String,
    /// The file's path in it.
    pub path: String,
}

/// What porcelain output says of one commit that owns lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlamedCommit {
    /// The commit and path before, when git reports them.
    pub previous: Option<Previous>,
    /// Whether the commit is a boundary: the range's lines have not changed since it, as far as
    /// the history the blame was allowed to read goes, so it is where the blame stopped, not
    /// necessarily where the lines came from.
    pub is_boundary: bool,
}

/// Lines `start..=end` of the blamed file, as numbers of the final file's lines, that one commit
/// owns, and where they stood in that commit's file. Lines are one run only when both numbers
/// run on: lines that sit together now but came from places apart in the commit are runs of their
/// own, since a caller who looks at the commit's file needs the lines as they were there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameRun {
    /// The first line, counted from 1, in the file at the commit blamed from.
    pub start: u32,
    /// The last line, inclusive, in the same file.
    pub end: u32,
    /// The commit that last changed these lines.
    pub oid: String,
    /// The first line's number in the commit's own file, counted from 1; the run's lines are
    /// `source_start` and the `end - start` lines after it there.
    pub source_start: u32,
}

/// A blame: the runs of lines in order, and what is known of each commit that owns one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blame {
    /// The runs, by first line; consecutive lines of one commit are one run.
    pub runs: Vec<BlameRun>,
    /// The commits that own a run, by id.
    pub commits: BTreeMap<String, BlamedCommit>,
}

impl BlameRun {
    /// Whether the line `next` follows this run's last line both in the final file and in the
    /// commit's own file.
    fn continues_with(&self, next: &Pending) -> bool {
        let source_end = self
            .source_start
            .saturating_add(self.end.saturating_sub(self.start));
        self.end.checked_add(1) == Some(next.line)
            && source_end.checked_add(1) == Some(next.source_line)
    }
}

/// A header being read: the line whose text comes next.
struct Pending {
    /// The commit that owns the line.
    oid: String,
    /// The line's number in the final file.
    line: u32,
    /// The line's number in the commit's own file.
    source_line: u32,
    /// Whether this is the commit's first header, which must be followed by its details.
    is_first: bool,
    /// Whether the `filename` line that ends the details was read.
    has_filename: bool,
}

/// Reads `output`, the standard output of `git blame --porcelain`, into a [`Blame`].
///
/// # Errors
///
/// [`BlameError::Malformed`] when a header is not an object id and two numbers, line numbers do
/// not rise, a commit's first appearance lacks its `filename` line, a `previous` line is not an
/// id and a UTF-8 path, a line's text is missing, or a header is not UTF-8. The details that are
/// not kept (an author's name, a summary) may hold any bytes: git writes a name as the commit
/// records it.
pub fn parse_porcelain(output: &[u8]) -> Result<Blame, BlameError> {
    let mut blame = Blame::default();
    let mut pending: Option<Pending> = None;
    let mut lines = output.split(|byte| *byte == b'\n').peekable();
    while let Some(line) = lines.next() {
        if let Some(current) = pending.as_mut() {
            if line.first() == Some(&b'\t') {
                finish_line(&mut blame, current)?;
                pending = None;
            } else {
                read_detail(&mut blame, current, line)?;
            }
        } else if !(line.is_empty() && lines.peek().is_none()) {
            pending = Some(read_header(&mut blame, line)?);
        }
    }
    if pending.is_some() {
        return Err(malformed("the output ends before a line's text"));
    }
    Ok(blame)
}

/// A [`BlameError::Malformed`] saying `what`.
fn malformed(what: &str) -> BlameError {
    BlameError::Malformed(what.to_owned())
}

/// The header `line` as the line it begins; a commit not seen before is added to `blame`.
fn read_header(blame: &mut Blame, line: &[u8]) -> Result<Pending, BlameError> {
    let text = std::str::from_utf8(line).map_err(|_| malformed("a header is not UTF-8"))?;
    let mut fields = text.split(' ');
    let (Some(oid), Some(source_line), Some(final_line)) =
        (fields.next(), fields.next(), fields.next())
    else {
        return Err(malformed("a header has fewer than three fields"));
    };
    if !is_object_id(oid) {
        return Err(malformed("a header does not begin with an object id"));
    }
    let number = |text: &str| {
        text.parse::<u32>()
            .ok()
            .filter(|number| *number > 0)
            .ok_or_else(|| malformed("a header's line number is not a positive number"))
    };
    let (line, source_line) = (number(final_line)?, number(source_line)?);
    let is_first = !blame.commits.contains_key(oid);
    if is_first {
        blame
            .commits
            .insert(oid.to_owned(), BlamedCommit::default());
    }
    Ok(Pending {
        oid: oid.to_owned(),
        line,
        source_line,
        is_first,
        has_filename: false,
    })
}

/// One line of details after a header: what it says of the commit is kept in `blame`, the rest
/// is skipped without being decoded.
fn read_detail(blame: &mut Blame, current: &mut Pending, line: &[u8]) -> Result<(), BlameError> {
    let mut parts = line.splitn(2, |byte| *byte == b' ');
    let key = parts.next().unwrap_or_default();
    let value = parts.next().unwrap_or_default();
    let Some(commit) = blame.commits.get_mut(&current.oid) else {
        return Err(malformed("details for a commit that was not announced"));
    };
    // INVARIANT: no-content-stored. The author, committer, summary and the other details are
    // read past here and dropped; only the previous commit and the boundary are kept.
    match key {
        b"previous" => {
            let text = std::str::from_utf8(value)
                .map_err(|_| malformed("a previous path is not UTF-8"))?;
            let (oid, path) = text
                .split_once(' ')
                .filter(|(oid, path)| is_object_id(oid) && !path.is_empty())
                .ok_or_else(|| malformed("a previous line is not an object id and a path"))?;
            commit.previous = Some(Previous {
                oid: oid.to_owned(),
                path: unquote_path(path)?,
            });
        }
        b"boundary" => commit.is_boundary = true,
        b"filename" => current.has_filename = true,
        _ => {}
    }
    Ok(())
}

/// The line whose text just came, added to the runs of `blame`.
fn finish_line(blame: &mut Blame, current: &Pending) -> Result<(), BlameError> {
    if current.is_first && !current.has_filename {
        return Err(malformed("a commit's details have no filename line"));
    }
    match blame.runs.last_mut() {
        Some(run) if current.line <= run.end => {
            return Err(malformed("line numbers do not rise"));
        }
        Some(run) if run.oid == current.oid && run.continues_with(current) => {
            run.end = current.line;
        }
        _ => blame.runs.push(BlameRun {
            start: current.line,
            end: current.line,
            oid: current.oid.clone(),
            source_start: current.source_line,
        }),
    }
    Ok(())
}

/// The path as git wrote it: as it is, or when it is in double quotes, with the escapes of C
/// strings that git uses (`\"`, `\\`, `\n`, `\t` and the other letters, and `\ooo` for a byte in
/// octal) undone.
fn unquote_path(path: &str) -> Result<String, BlameError> {
    let Some(inner) = path
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return Ok(path.to_owned());
    };
    let bad = || malformed("a quoted path has an escape git does not write");
    let mut bytes = inner.bytes();
    let mut decoded = Vec::with_capacity(inner.len());
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let escaped = bytes.next().ok_or_else(bad)?;
        decoded.push(match escaped {
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0b,
            b'"' | b'\\' => escaped,
            b'0'..=b'3' => {
                let (second, third) =
                    (bytes.next().ok_or_else(bad)?, bytes.next().ok_or_else(bad)?);
                let digits = [escaped, second, third];
                if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
                    return Err(bad());
                }
                digits
                    .iter()
                    .fold(0_u8, |value, digit| value.wrapping_mul(8) + (digit - b'0'))
            }
            _ => return Err(bad()),
        });
    }
    String::from_utf8(decoded).map_err(|_| malformed("a quoted path is not UTF-8"))
}

#[cfg(test)]
mod tests;
