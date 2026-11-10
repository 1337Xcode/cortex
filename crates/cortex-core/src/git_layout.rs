//! Where git keeps a working tree's repository, read from the files git writes, without running
//! git: the hook reads it on its path, where no process may be spawned.
//!
//! A working tree's top holds `.git`. In a main working tree it is the repository's folder. In a
//! linked working tree (`git worktree add`) it is a file, `gitdir: <path>`, naming the worktree's
//! own folder of state (`<common>/worktrees/<name>`), which holds `commondir`, the path of the
//! folder every working tree shares (objects, refs, configuration, `info/exclude`), and `gitdir`,
//! the path of the `.git` file that names it back. A submodule's `.git` is a file too, naming a
//! folder with no `commondir`: it is a repository of its own, not a linked working tree.
//!
//! Each file is read once, at most [`MAX_GIT_FILE_BYTES`], and only when it is a regular file (a
//! named pipe would hold its reader until a writer came), and the folders are canonicalized. A file
//! that is too large or not a regular file, a path that leads nowhere, a link that loops, a folder
//! of state that is not under its common folder's `worktrees`, or one that does not name this
//! `.git` back, is no layout, never a guess: a `.git` file planted in one repository cannot make
//! its folder a linked working tree of another.

use std::path::{Path, PathBuf};

use crate::{Identifier, canonical_root, read_text_file};

/// The most bytes read of any of git's small files (`.git`, `commondir`, `gitdir`, `HEAD`): each
/// holds one path or one reference, far below this.
pub const MAX_GIT_FILE_BYTES: u64 = 4 * 1024;

/// The name of the folder or file at a working tree's top that leads to its repository.
const DOT_GIT: &str = ".git";

/// The prefix of the one line a `.git` file holds.
const GITDIR_PREFIX: &str = "gitdir:";

/// The prefix of a `HEAD` that names a branch.
const BRANCH_PREFIX: &str = "ref: refs/heads/";

/// Where git keeps a working tree's repository. Every path is canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitLayout {
    /// The working tree's top: the folder holding `.git`.
    pub worktree_root: PathBuf,
    /// The folder of this working tree's own state (its `HEAD` and index): the repository's folder
    /// for a main working tree, `<common>/worktrees/<name>` for a linked one.
    pub git_dir: PathBuf,
    /// The folder every working tree of the repository shares: `git rev-parse --git-common-dir`.
    pub common_dir: PathBuf,
    /// The repository's main working tree: the working tree itself unless it is a linked one; for
    /// a linked one, the folder whose `.git` is the common folder; none when no folder is (a bare
    /// repository's linked working tree, or one whose repository keeps its folder elsewhere).
    pub main_root: Option<PathBuf>,
}

impl GitLayout {
    /// Whether this is a linked working tree: its state is not the shared folder.
    #[must_use]
    pub fn is_linked(&self) -> bool {
        self.git_dir != self.common_dir
    }

    /// A linked working tree's name, as git names it: the name of its folder of state under
    /// `worktrees/`. None for a main working tree, or a name that is not one line of plain text.
    #[must_use]
    pub fn worktree_name(&self) -> Option<Identifier> {
        if !self.is_linked() {
            return None;
        }
        let name = self.git_dir.file_name()?.to_str()?;
        Identifier::new(name).ok()
    }

    /// The branch this working tree has checked out, as its `HEAD` names it now: none when `HEAD`
    /// is detached, cannot be read, or names a branch that is not one line of plain text.
    #[must_use]
    pub fn branch(&self) -> Option<Identifier> {
        let head = read_git_file(&self.git_dir.join("HEAD"))?;
        let name = head
            .trim_end_matches(['\n', '\r'])
            .strip_prefix(BRANCH_PREFIX)?;
        Identifier::new(name).ok()
    }
}

/// The layout of the working tree whose top is `folder`, the folder holding `.git`; none when it
/// holds none, or what it holds cannot be read as git writes it (see the module).
#[must_use]
pub fn git_layout(folder: &Path) -> Option<GitLayout> {
    let worktree_root = canonical_root(folder).ok()?;
    let dot_git = worktree_root.join(DOT_GIT);
    let kind = std::fs::symlink_metadata(&dot_git).ok()?;
    if kind.is_dir() {
        let git_dir = canonical_root(&dot_git).ok()?;
        return Some(GitLayout {
            main_root: Some(worktree_root.clone()),
            worktree_root,
            common_dir: git_dir.clone(),
            git_dir,
        });
    }
    if !kind.is_file() {
        return None;
    }
    let git_dir = named_folder(&dot_git, &worktree_root, GITDIR_PREFIX)?;
    let Shared::At(common_dir) = shared_of(&git_dir)? else {
        // No `commondir`: a submodule's folder, or a repository kept apart from its working tree.
        return Some(GitLayout {
            main_root: Some(worktree_root.clone()),
            worktree_root,
            common_dir: git_dir.clone(),
            git_dir,
        });
    };
    // INVARIANT: true-at-render. A linked working tree's folder of state is one of its common
    // folder's `worktrees`: a `.git` file naming a folder elsewhere names no repository's tree.
    let worktrees = canonical_root(&common_dir.join("worktrees")).ok()?;
    if git_dir.parent() != Some(worktrees.as_path()) || !names_back(&git_dir, &dot_git)? {
        return None;
    }
    Some(GitLayout {
        main_root: main_root_of(&common_dir),
        worktree_root,
        git_dir,
        common_dir,
    })
}

/// The folder the one line of the file `file` names after `prefix` (or the whole line, with no
/// prefix), a path relative to `base` when it is not absolute, canonical; none when the file is
/// too large, is not that line, or names no folder.
fn named_folder(file: &Path, base: &Path, prefix: &str) -> Option<PathBuf> {
    let text = read_git_file(file)?;
    let line = text.trim_end_matches(['\n', '\r']);
    let named = line.strip_prefix(prefix)?.trim();
    if named.is_empty() || named.contains(['\n', '\r']) {
        return None;
    }
    canonical_root(&base.join(named)).ok()
}

/// What a folder of state says of the folder its working tree shares with others.
enum Shared {
    /// It holds no `commondir`: it is its repository's own folder.
    Own,
    /// Its `commondir` names this folder.
    At(PathBuf),
}

/// What `git_dir` says of the folder it shares, its `commondir` read relative to it; none when
/// there is a `commondir` that cannot be read or names no folder.
fn shared_of(git_dir: &Path) -> Option<Shared> {
    let file = git_dir.join("commondir");
    match std::fs::symlink_metadata(&file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Shared::Own),
        Err(_) => None,
        Ok(_) => named_folder(&file, git_dir, "").map(Shared::At),
    }
}

/// The text of the regular file at `path`, at most [`MAX_GIT_FILE_BYTES`]; none for anything else
/// (opening a named pipe waits for a writer, and a link could lead anywhere), for a file too large,
/// and for one that cannot be read as text.
fn read_git_file(path: &Path) -> Option<String> {
    // INVARIANT: fail-open. Nothing but a regular file is opened, so no read waits on a writer.
    if !std::fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    read_text_file(path, MAX_GIT_FILE_BYTES).ok()?
}

/// Whether `git_dir`, a linked working tree's folder of state, names `dot_git` back in its
/// `gitdir`: a `.git` file copied or crafted elsewhere does not make its folder that working tree.
/// None when `gitdir` cannot be read.
fn names_back(git_dir: &Path, dot_git: &Path) -> Option<bool> {
    let text = read_git_file(&git_dir.join("gitdir"))?;
    let named = text.trim_end_matches(['\n', '\r']).trim();
    if named.is_empty() || named.contains(['\n', '\r']) {
        return None;
    }
    let named = std::fs::canonicalize(git_dir.join(named)).ok()?;
    let ours = std::fs::canonicalize(dot_git).ok()?;
    Some(named == ours)
}

/// The main working tree of the repository whose common folder is `common_dir`: the folder above
/// it when the common folder is that folder's `.git`; none otherwise, as for a bare repository.
fn main_root_of(common_dir: &Path) -> Option<PathBuf> {
    if common_dir.file_name()? != DOT_GIT {
        return None;
    }
    let parent = common_dir.parent()?;
    let is_its_git = canonical_root(&parent.join(DOT_GIT)).is_ok_and(|found| found == common_dir);
    is_its_git.then(|| parent.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A linked working tree written as git writes one: a main tree's `.git` folder with
    /// `worktrees/wt` naming the `.git` file of `linked` back and its common folder as `../..`.
    /// Returns the main tree and the linked one.
    fn written(base: &Path) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
        let main = base.join("main");
        let state = main.join(".git").join("worktrees").join("wt");
        std::fs::create_dir_all(&state)?;
        let linked = base.join("wt");
        std::fs::create_dir_all(&linked)?;
        std::fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", state.display()),
        )?;
        std::fs::write(state.join("commondir"), "../..\n")?;
        std::fs::write(
            state.join("gitdir"),
            format!("{}\n", linked.join(".git").display()),
        )?;
        std::fs::write(state.join("HEAD"), "ref: refs/heads/feature/one\n")?;
        Ok((canonical_root(&main)?, canonical_root(&linked)?))
    }

    /// A linked working tree written by hand reads as git would: linked, its main tree the folder
    /// above the common one, named `wt`, on the branch `feature/one`. Its HEAD detached, it names
    /// no branch.
    #[test]
    fn a_linked_working_tree_names_its_main_tree_name_and_branch() -> TestResult {
        let base = tempfile::tempdir()?;
        let (main, linked) = written(base.path())?;
        let layout = git_layout(&linked).ok_or("no layout")?;
        assert!(layout.is_linked());
        assert_eq!(layout.main_root.as_deref(), Some(main.as_path()));
        assert_eq!(layout.common_dir, main.join(".git"));
        assert_eq!(layout.worktree_root, linked);
        assert_eq!(layout.worktree_name(), Some(Identifier::new("wt")?));
        assert_eq!(layout.branch(), Some(Identifier::new("feature/one")?));
        std::fs::write(layout.git_dir.join("HEAD"), "0123456789abcdef\n")?;
        assert_eq!(layout.branch(), None);
        Ok(())
    }

    /// No layout for a `.git` file that names a folder that is not there, nor for one over
    /// 4 KiB, nor for a `commondir` over 4 KiB, nor for a worktree folder that names another
    /// `.git` back, nor for a folder holding no `.git`.
    #[test]
    fn what_cannot_be_read_as_git_writes_it_is_no_layout() -> TestResult {
        let base = tempfile::tempdir()?;
        let (_, linked) = written(base.path())?;
        let state = git_layout(&linked).ok_or("no layout")?.git_dir;
        let dot_git = linked.join(".git");
        let original = std::fs::read(&dot_git)?;

        std::fs::write(&dot_git, "gitdir: ../nowhere\n")?;
        assert_eq!(git_layout(&linked), None);
        let padded = format!(
            "{}{}",
            String::from_utf8(original.clone())?,
            " ".repeat(4 * 1024)
        );
        std::fs::write(&dot_git, padded)?;
        assert_eq!(git_layout(&linked), None);
        std::fs::write(&dot_git, &original)?;
        assert!(git_layout(&linked).is_some());

        std::fs::write(
            state.join("commondir"),
            format!("../..{}\n", " ".repeat(4 * 1024)),
        )?;
        assert_eq!(git_layout(&linked), None);
        std::fs::write(state.join("commondir"), "../..\n")?;

        let elsewhere = base.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere)?;
        std::fs::write(elsewhere.join(".git"), &original)?;
        assert_eq!(git_layout(&elsewhere), None);
        assert_eq!(git_layout(base.path()), None);
        Ok(())
    }

    /// A `.git` file planted in another repository's folder, naming a folder of state outside the
    /// main tree's `worktrees` that holds a common folder pointing at the main tree's `.git` and a
    /// back-pointer to the planted file, is no layout: only a folder under `worktrees` is one.
    #[test]
    fn a_folder_of_state_outside_worktrees_is_no_layout() -> TestResult {
        let base = tempfile::tempdir()?;
        let (main, _) = written(base.path())?;
        let planted = base.path().join("other").join("sub");
        std::fs::create_dir_all(&planted)?;
        let fake = base.path().join("other").join("fake").join("wt");
        std::fs::create_dir_all(&fake)?;
        std::fs::write(planted.join(".git"), "gitdir: ../fake/wt\n")?;
        std::fs::write(
            fake.join("commondir"),
            format!("{}\n", main.join(".git").display()),
        )?;
        std::fs::write(
            fake.join("gitdir"),
            format!("{}\n", planted.join(".git").display()),
        )?;
        assert_eq!(git_layout(&planted), None);
        Ok(())
    }

    /// A named pipe where git keeps `commondir`, `gitdir` or `HEAD` is never opened, which would
    /// wait for a writer: the layout is none, or the branch is none, at once.
    #[cfg(unix)]
    #[test]
    fn a_named_pipe_is_never_opened() -> TestResult {
        let base = tempfile::tempdir()?;
        let (_, linked) = written(base.path())?;
        let state = git_layout(&linked).ok_or("no layout")?.git_dir;
        let fifo = |path: &Path| -> TestResult {
            std::fs::remove_file(path)?;
            let made = std::process::Command::new("mkfifo").arg(path).status()?;
            assert!(made.success(), "mkfifo {}", path.display());
            Ok(())
        };
        fifo(&state.join("HEAD"))?;
        let layout = git_layout(&linked).ok_or("no layout")?;
        assert_eq!(layout.branch(), None);
        let back = format!("{}\n", linked.join(".git").display());
        for (name, text) in [("commondir", "../..\n"), ("gitdir", back.as_str())] {
            fifo(&state.join(name))?;
            assert_eq!(git_layout(&linked), None, "{name}");
            std::fs::remove_file(state.join(name))?;
            std::fs::write(state.join(name), text)?;
            assert!(git_layout(&linked).is_some(), "{name}");
        }
        Ok(())
    }
}
