//! Where a check ran, as far as the line tells: the folder it ran in, and the way from the folder
//! the line started in. Paths are resolved from the line's `cd`s without touching the disk.

use std::path::Path;

use cortex_core::RepoPath;

use crate::commands::folder::{Folder, Named};
use crate::commands::shell::{Command, List, Word};

/// Where a check ran.
#[derive(Debug)]
pub(super) struct Place {
    /// The folder it ran in, relative to the repository.
    pub(super) dir: RepoPath,
    /// The way down from the folder the line started in, when the line moved: `None` when it ran
    /// where the line started.
    pub(super) enter: Option<String>,
}

/// Where the simple command `target` of `list` ran, for a line started in `cwd` of the repository
/// whose top is `root`. `None` when the line moved in a way not followed: a `cd` the line does
/// not say the target of, a group before the command, a folder outside the repository, or up from
/// where it started.
pub(super) fn of(list: &List, target: &[Word], root: &Path, cwd: &Path) -> Option<Place> {
    let start = Folder::new(cwd);
    let mut folder = start.clone();
    if !walk(list, target, &mut folder) {
        return None;
    }
    let dir = repo_path(&folder, root)?;
    let from = repo_path(&start, root)?;
    let way = within(&from, &dir)?;
    Some(Place {
        dir,
        enter: (!way.is_empty()).then_some(way),
    })
}

/// Walks the commands of `list` in order, moving `folder` by each `cd` that moves the shell of
/// the line, until `target`; whether it was found with no group before it.
fn walk(list: &List, target: &[Word], folder: &mut Folder) -> bool {
    for pipeline in list.items.iter().flat_map(|item| &item.pipelines) {
        let is_alone = pipeline.commands.len() == 1;
        for command in &pipeline.commands {
            let Command::Simple(words) = command else {
                return false;
            };
            if std::ptr::eq(words.as_slice(), target) {
                return true;
            }
            if let Some(moved) = folder.after(words).filter(|_| is_alone) {
                *folder = moved;
            }
        }
    }
    false
}

/// The repository path of `folder`.
fn repo_path(folder: &Folder, root: &Path) -> Option<RepoPath> {
    let here = Word {
        text: ".".to_owned(),
        is_literal: true,
    };
    match folder.named(&here, root)? {
        Named::Everything => Some(RepoPath::root()),
        Named::Path(path) => Some(path),
    }
}

/// The path of `path` below the folder `base`, `/` between names: empty when they are the same
/// place, `None` when `path` is not inside `base`.
pub(super) fn within(base: &RepoPath, path: &RepoPath) -> Option<String> {
    if base.is_root() {
        return Some(path.as_str().to_owned());
    }
    if path == base {
        return Some(String::new());
    }
    path.as_str()
        .strip_prefix(base.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
        .map(str::to_owned)
}
