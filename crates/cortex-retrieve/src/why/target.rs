//! What a `why` target names: a stored file by its repository path; the innermost definition that
//! holds a line of one (`path:line`), by the rule a prompt's `path:line` is read by, or the file
//! itself, said so, for a line no definition holds; a symbol of one (`path:symbol`); or a symbol by
//! its name alone, when one symbol has it. A target that names nothing or several things (a line
//! two definitions share among them) is refused in words saying which, and a line is looked up
//! only in a file whose copy on disk is the one the index read: the index's line numbers are not
//! trusted for a file that changed since.

use std::fmt::Write as _;
use std::path::Path;

use cortex_core::{Identifier, RepoPath, is_plain};
use cortex_store::{FileId, NodeId, Queries, StoreError};

use crate::disk::CheckedFile;
use crate::innermost::innermost_at;

/// Most symbols a refusal names when a name fits several.
const MAX_CANDIDATES: usize = 5;

/// What a target names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Named {
    /// The file, or the file the symbol is defined in.
    pub(crate) path: RepoPath,
    /// The file's node.
    pub(crate) file: NodeId,
    /// The symbol, when the target names one: its node and its qualified name.
    pub(crate) symbol: Option<(NodeId, String)>,
    /// The line asked of, when no definition holds it and the target is the file.
    pub(crate) outside_line: Option<u32>,
}

/// A stored file, as a target finds it.
struct Stored {
    /// Its row.
    id: FileId,
    /// Its node.
    node: NodeId,
    /// Its path.
    path: RepoPath,
}

/// What `given` names in the store `queries` reads, a line read from the repository at `root` up
/// to `max_bytes`; the refusal, in words, when it names nothing or several things.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn named(
    queries: Queries<'_>,
    given: &str,
    (root, max_bytes): (&Path, u64),
) -> Result<Result<Named, String>, StoreError> {
    let given = given.trim();
    if given.is_empty() || !is_plain(given) {
        return Ok(Err(
            "name a file, `path:line`, `path:symbol` or a symbol, in plain text".to_owned(),
        ));
    }
    if let Some(file) = stored(queries, given)? {
        return Ok(Ok(file_named(file)));
    }
    if let Some((path_text, rest)) = given.rsplit_once(':')
        && let Some(file) = stored(queries, path_text)?
    {
        return if !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()) {
            at_line(queries, file, rest, (root, max_bytes))
        } else {
            in_file(queries, file, rest)
        };
    }
    by_name(queries, given)
}

/// The stored file at `text`, when `text` is a repository path and a file is stored there.
fn stored(queries: Queries<'_>, text: &str) -> Result<Option<Stored>, StoreError> {
    let Ok(path) = RepoPath::new(text) else {
        return Ok(None);
    };
    if path.is_root() {
        return Ok(None);
    }
    let (Some(row), Some(node)) = (queries.file(&path)?, queries.file_node_at(&path)?) else {
        return Ok(None);
    };
    Ok(Some(Stored {
        id: row.id,
        node,
        path,
    }))
}

/// `file` named whole.
fn file_named(file: Stored) -> Named {
    Named {
        path: file.path,
        file: file.node,
        symbol: None,
        outside_line: None,
    }
}

/// The innermost definition of `file` holding line `line`, the file itself when none does; a
/// refusal when the copy on disk is not the one the index read, it has no such line, or
/// definitions that do not nest share it.
fn at_line(
    queries: Queries<'_>,
    file: Stored,
    line: &str,
    (root, max_bytes): (&Path, u64),
) -> Result<Result<Named, String>, StoreError> {
    let path = file.path.as_str();
    let Some(content) = queries
        .file(&file.path)?
        .and_then(|row| row.file.content_hash)
    else {
        return Ok(Err(format!(
            "Cortex does not read {path}, so it knows none of its lines"
        )));
    };
    let Ok(checked) = CheckedFile::read(root, &file.path, &content, max_bytes) else {
        return Ok(Err(format!(
            "{path} on disk is not the copy Cortex last read, so its lines are not known yet: \
             ask again in a moment"
        )));
    };
    let count = checked.line_count();
    if count == 0 {
        return Ok(Err(format!("{path} is empty")));
    }
    let line = match line.parse::<u32>() {
        Ok(line) if (1..=count).contains(&line) => line,
        _ => return Ok(Err(format!("{path} has lines 1 to {count}"))),
    };
    let nodes: Vec<NodeId> = innermost_at(queries, file.id, line)?
        .into_iter()
        .map(|span| span.node)
        .collect();
    let states = queries.anchor_states(&nodes)?;
    let qualified: Vec<&str> = states
        .iter()
        .filter_map(|state| state.symbol.as_ref().map(Identifier::as_str))
        .collect();
    Ok(match (states.as_slice(), qualified.as_slice()) {
        ([], _) => Ok(Named {
            outside_line: Some(line),
            ..file_named(file)
        }),
        ([only], [name]) => Ok(symbol_named(file, only.node, name)),
        _ => Err(format!(
            "line {line} of {path} holds {} definitions: {}; name one as `path:symbol`",
            nodes.len(),
            listed(&qualified)
        )),
    })
}

/// The symbol of `file` that `name` names, or a refusal naming the ones it could be.
fn in_file(
    queries: Queries<'_>,
    file: Stored,
    name: &str,
) -> Result<Result<Named, String>, StoreError> {
    let rows = queries.symbols_in_file_called(file.id, name)?;
    let path = file.path.as_str();
    match rows.as_slice() {
        [] => Ok(Err(format!("{path} defines no symbol {name}"))),
        [only] => Ok(match queries.symbol_node(only.id)? {
            Some(node) => Ok(symbol_named(file, node, only.qual_name.as_str())),
            None => Err(format!("{path} defines no symbol {name}")),
        }),
        many => {
            let names: Vec<&str> = many.iter().map(|row| row.qual_name.as_str()).collect();
            Ok(Err(format!(
                "{path} defines {} symbols named {name}: {}; name one by its qualified name",
                many.len(),
                listed(&names)
            )))
        }
    }
}

/// The one symbol in the repository that `name` names, or a refusal naming the ones it could be.
fn by_name(queries: Queries<'_>, name: &str) -> Result<Result<Named, String>, StoreError> {
    let rows = queries.symbols_called(name)?;
    let nothing = || format!("{name} names no file or symbol Cortex indexes");
    match rows.as_slice() {
        [] => Ok(Err(nothing())),
        [only] => {
            let Some(path) = queries.file_path(only.file)? else {
                return Ok(Err(nothing()));
            };
            let found = (
                queries.file(&path)?,
                queries.file_node_at(&path)?,
                queries.symbol_node(only.id)?,
            );
            let (Some(row), Some(file), Some(node)) = found else {
                return Ok(Err(nothing()));
            };
            let file = Stored {
                id: row.id,
                node: file,
                path,
            };
            Ok(Ok(symbol_named(file, node, only.qual_name.as_str())))
        }
        many => {
            let mut names = Vec::new();
            for row in many {
                if let Some(path) = queries.file_path(row.file)? {
                    names.push(format!("{}:{}", path.as_str(), row.qual_name.as_str()));
                }
            }
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            Ok(Err(format!(
                "{} symbols are named {name}: {}; name one as `path:symbol`",
                many.len(),
                listed(&names)
            )))
        }
    }
}

/// The symbol of `file` whose node is `node`, qualified as `qualified`.
fn symbol_named(file: Stored, node: NodeId, qualified: &str) -> Named {
    Named {
        symbol: Some((node, qualified.to_owned())),
        ..file_named(file)
    }
}

/// `names` that are plain text, at most [`MAX_CANDIDATES`], joined, with how many more there are.
fn listed(names: &[&str]) -> String {
    let plain: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| is_plain(name))
        .collect();
    let shown = plain.get(..MAX_CANDIDATES).unwrap_or(&plain);
    let mut text = shown.join(", ");
    let more = names.len() - shown.len();
    if more > 0 {
        let _ = write!(text, " and {more} more");
    }
    text
}
