//! A failed check's stack frames, grounded in the repository: each frame's printed path mapped to a
//! file the index holds, kept only when the file on disk is still the one the index read (so the
//! run saw the lines the index knows), with the line's code as it is now and the innermost
//! definition holding it. A frame outside the repository, in a file the index does not hold, past
//! the file's end or in a file that changed is dropped, and a failure none of whose frames is left
//! grounds nothing: an import, environment or dependency error says nothing about the code.

use std::path::{Component, Path, PathBuf};

use cortex_core::{Identifier, LineRange, RepoPath};
use cortex_learn::Failures;
use cortex_store::{NodeId, Queries, StoreError};

use crate::disk::CheckedFile;
use crate::innermost::innermost_at;

/// The most frames grounded: the failure map shows a few, the seeds take a few more.
pub const MAX_GROUNDED_FRAMES: usize = 8;

/// One frame in the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundedFrame {
    /// The file.
    pub path: RepoPath,
    /// The file's node.
    pub file: NodeId,
    /// The frame's line.
    pub line: u32,
    /// The line's code as it is on disk, without its indentation.
    pub code: String,
    /// The innermost definition holding the line, by node and name; none at the top level of a
    /// file, or when definitions that do not nest share the line.
    pub symbol: Option<(NodeId, Identifier)>,
}

/// What a failed check's output says, grounded in the repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroundedFailure {
    /// The frames left, each failure's innermost first, at most [`MAX_GROUNDED_FRAMES`].
    pub frames: Vec<GroundedFrame>,
    /// The first failed test's name, as the output gave it; whoever shows it checks it first.
    pub test: Option<String>,
}

/// Where a check ran and what may be read to ground it.
#[derive(Debug, Clone, Copy)]
pub struct FailureQuery<'a> {
    /// The repository's root, canonical.
    pub root: &'a Path,
    /// The folder the command ran in, canonical, from which relative paths are read.
    pub cwd: &'a Path,
    /// `index.max_file_bytes`: the most bytes a file may hold to be read.
    pub max_file_bytes: u64,
}

/// `failures` grounded in the repository at `query.root`, as the store `queries` reads holds it;
/// no frames when none of them is in a file the index holds as it is on disk.
///
/// # Errors
///
/// The store's errors.
pub fn ground_failure(
    queries: Queries<'_>,
    failures: &Failures,
    query: &FailureQuery<'_>,
) -> Result<GroundedFailure, StoreError> {
    let mut frames: Vec<GroundedFrame> = Vec::new();
    for frame in &failures.frames {
        if frames.len() >= MAX_GROUNDED_FRAMES {
            break;
        }
        let Some(grounded) = ground_frame(queries, (&frame.path, frame.line), query)? else {
            continue;
        };
        if !frames
            .iter()
            .any(|kept| kept.file == grounded.file && kept.line == grounded.line)
        {
            frames.push(grounded);
        }
    }
    let test = failures.failed_tests.first().cloned();
    Ok(GroundedFailure { frames, test })
}

/// The frame at `line` of the file printed as `printed`, grounded, or `None` when it is not in a
/// file the index holds as it is on disk.
fn ground_frame(
    queries: Queries<'_>,
    (printed, line): (&str, u32),
    query: &FailureQuery<'_>,
) -> Result<Option<GroundedFrame>, StoreError> {
    for path in repository_paths(printed, query) {
        let Some(row) = queries.file(&path)? else {
            continue;
        };
        let (Some(stored), Some(file)) = (row.file.content_hash, queries.file_node(row.id)?) else {
            return Ok(None);
        };
        // INVARIANT: true-at-render (a frame is grounded only in a file whose bytes are the ones
        // the index read, so its line and code are the ones the run and the index both saw)
        let Ok(checked) = CheckedFile::read(query.root, &path, &stored, query.max_file_bytes)
        else {
            return Ok(None);
        };
        let Some(code) = LineRange::new(line, line)
            .ok()
            .and_then(|range| checked.lines(range))
            .and_then(|lines| lines.first().map(|(_, code)| code.trim().to_owned()))
        else {
            return Ok(None);
        };
        let symbol = match innermost_at(queries, row.id, line)?.as_slice() {
            [only] => Some((only.node, only.name.clone())),
            _ => None,
        };
        return Ok(Some(GroundedFrame {
            path,
            file,
            line,
            code,
            symbol,
        }));
    }
    Ok(None)
}

/// The repository paths `printed` may name, in the order to try them: an absolute path as it is;
/// a relative one from the folder the command ran in, then from the repository's root, where
/// runners such as cargo print paths from. A path outside the repository names none.
fn repository_paths(printed: &str, query: &FailureQuery<'_>) -> Vec<RepoPath> {
    let written = Path::new(printed);
    let bases: &[&Path] = if written.is_absolute() {
        &[]
    } else {
        &[query.cwd, query.root]
    };
    let candidates: Vec<PathBuf> = if bases.is_empty() {
        vec![written.to_path_buf()]
    } else {
        bases.iter().map(|base| base.join(written)).collect()
    };
    let mut paths: Vec<RepoPath> = Vec::new();
    for candidate in candidates {
        let Some(normal) = lexically_normal(&candidate) else {
            continue;
        };
        if let Some(path) = under_root(query.root, &normal)
            && !path.is_root()
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    paths
}

/// The repository path of `path`, inside `root` (written in any letters' case on Windows, as a
/// traceback prints a folder the way the command line spelled it), or `None` outside it.
fn under_root(root: &Path, path: &Path) -> Option<RepoPath> {
    RepoPath::from_native_in_any_case(root, path)
}

/// `path` with its `.` components dropped and each `..` taking away the name before it, without
/// asking the file system; `None` when a `..` would climb above the path's root.
fn lexically_normal(path: &Path) -> Option<PathBuf> {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normal.pop() {
                    return None;
                }
            }
            other => normal.push(other.as_os_str()),
        }
    }
    Some(normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A relative path is read from the folder the command ran in, then from the root; `.` and
    /// `..` are resolved on the way; an absolute path is taken as it is; paths that leave the
    /// repository name nothing.
    #[test]
    fn printed_paths_are_read_from_the_command_s_folder_then_the_root() {
        let root = if cfg!(windows) {
            Path::new(r"C:\repo")
        } else {
            Path::new("/repo")
        };
        let cwd = root.join("calc");
        let query = FailureQuery {
            root,
            cwd: &cwd,
            max_file_bytes: 1_000,
        };
        let named = |printed: &str| -> Vec<String> {
            repository_paths(printed, &query)
                .iter()
                .map(|path| path.as_str().to_owned())
                .collect()
        };
        assert_eq!(named("ops.py"), ["calc/ops.py", "ops.py"]);
        assert_eq!(named("./ops.py"), ["calc/ops.py", "ops.py"]);
        assert_eq!(named("../main.py"), ["main.py"]);
        assert_eq!(named("../../elsewhere/x.py"), Vec::<String>::new());
        let absolute = root.join("tests").join("test_ops.py");
        assert_eq!(
            named(&absolute.to_string_lossy().replace('\\', "/")),
            ["tests/test_ops.py"]
        );
        let outside = if cfg!(windows) {
            "C:/other/x.py"
        } else {
            "/other/x.py"
        };
        assert_eq!(named(outside), Vec::<String>::new());
    }

    /// On Windows, a traceback's root in other letters' case is the root: `c:/REPO/calc/ops.py`
    /// is `calc/ops.py` of the repository at `C:\repo`; another folder in any case is not.
    #[cfg(windows)]
    #[test]
    fn a_windows_root_in_another_case_is_the_root() {
        let root = Path::new(r"C:\repo");
        let query = FailureQuery {
            root,
            cwd: root,
            max_file_bytes: 1_000,
        };
        let named = |printed: &str| -> Vec<String> {
            repository_paths(printed, &query)
                .iter()
                .map(|path| path.as_str().to_owned())
                .collect()
        };
        assert_eq!(named("c:/REPO/calc/ops.py"), ["calc/ops.py"]);
        assert_eq!(named(r"c:\repo\tests\test_ops.py"), ["tests/test_ops.py"]);
        assert_eq!(named("c:/repository/x.py"), Vec::<String>::new());
    }
}
