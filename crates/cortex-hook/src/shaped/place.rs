//! Where the files and folders a tool names lie: a path as the agent gives it, absolute or relative
//! to its working folder, taken to the repository; and the files a search found, from its response.

use std::path::Path;

use cortex_core::{MAX_HITS, MAX_HITS_BYTES, RepoPath};

use super::payload::{ToolResponse, content_paths};
use crate::adapter::Located;

/// Most files of a search's response examined for its hits: room to pass over duplicates and files
/// outside the repository on the way to [`MAX_HITS`], and a bound on what a long list costs the
/// hook.
const MAX_LISTED: usize = 4 * MAX_HITS;

/// Where a file or folder a tool names lies.
pub(crate) enum Place {
    /// The tool names none.
    Unnamed,
    /// Inside the repository, at this path.
    Inside(RepoPath),
    /// Outside the repository.
    Outside,
}

impl Place {
    /// The repository path, when the place is one inside the repository.
    pub(crate) fn path(self) -> Option<RepoPath> {
        match self {
            Self::Inside(path) => Some(path),
            Self::Unnamed | Self::Outside => None,
        }
    }
}

/// Where `path` lies: a relative path is the agent's working folder `cwd` joined with it, then the
/// whole is taken relative to the repository's root as the agent spells it, else as it is on
/// disk.
pub(crate) fn place(path: Option<&str>, cwd: Option<&str>, located: &Located) -> Place {
    let Some(path) = path else {
        return Place::Unnamed;
    };
    let path = Path::new(path);
    let full = match cwd {
        Some(cwd) if path.is_relative() => Path::new(cwd).join(path),
        _ => path.to_path_buf(),
    };
    RepoPath::from_native(&located.agent_root, &full)
        .or_else(|_| RepoPath::from_native(&located.root, &full))
        .map_or(Place::Outside, Place::Inside)
}

/// The files a search's `response` lists, each resolved as a tool's path is, from the working
/// folder `cwd`: of the first [`MAX_LISTED`] it names (the file list, then the files a search's
/// output names), those inside the repository and not its top folder, each once, in the order
/// listed, until [`MAX_HITS`] of them or [`MAX_HITS_BYTES`] of paths.
pub(crate) fn hits(
    response: &ToolResponse<'_>,
    cwd: Option<&str>,
    located: &Located,
) -> Vec<RepoPath> {
    let named = response.content.map(content_paths).unwrap_or_default();
    let listed = response
        .filenames
        .iter()
        .flatten()
        .chain(&named)
        .take(MAX_LISTED);
    let mut hits: Vec<RepoPath> = Vec::new();
    let mut size_bytes = 0;
    for path in listed {
        let Place::Inside(path) = place(Some(path), cwd, located) else {
            continue;
        };
        if path == RepoPath::root() || hits.contains(&path) {
            continue;
        }
        size_bytes += path.as_str().len();
        if hits.len() == MAX_HITS || size_bytes > MAX_HITS_BYTES {
            break;
        }
        hits.push(path);
    }
    hits
}
