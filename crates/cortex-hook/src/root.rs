//! The repository a hook's working folder belongs to: the nearest folder at or above it that holds
//! a `.cortex` folder of its own. A `.cortex` holding nothing but `shared` is not one: that is what
//! git checks out in a clone of a repository whose team shares facts, where the user who cloned it
//! never set Cortex up (files an operating system leaves in any folder it shows, such as
//! `.DS_Store`, count for nothing). A command finds a clone's root all the same
//! ([`locate_marked`]): it is still the repository's root, though no hook records there. Inside
//! the user's home folder the search stops below the home folder, whose `.cortex` is the user's own
//! folder and which no repository contains. A folder with no marked folder above it was never set
//! up for Cortex, and the hook leaves it alone.
//!
//! A linked working tree (`git worktree add`) holds no `.cortex` of its own: its `.git` is a file
//! naming its repository, whose main working tree holds one. Met on the way up before any marked
//! folder, it is where the working folder sits, and the repository is its main tree's, read from
//! git's files with no git run ([`cortex_core::git_layout`]); a layout that cannot be read is no
//! linked working tree, and the search goes on above it as for any folder.

use std::path::{Path, PathBuf};

use cortex_core::{canonical_root, git_layout};

use crate::adapter::Located;

/// The folder that marks a repository's root.
const MARKER: &str = ".cortex";

/// What a working tree's top holds that leads to its repository: a file in a linked working tree.
const DOT_GIT: &str = ".git";

/// The folder inside the marker that a team commits, and that alone marks nothing.
const SHARED: &str = "shared";

/// Files an operating system leaves in a folder it shows, which say nothing of who set it up.
const MADE_BY_THE_SYSTEM: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

/// Where the working folder `cwd`, as the agent spells it, sits: its canonical form, its
/// repository's canonical root, and that root as the agent spells it. `None` when `cwd` is not a
/// folder or no repository set up for Cortex holds it. `home` is the user's home folder: neither it
/// nor anything above it is a root for a folder inside it.
pub fn locate(cwd: &Path, home: Option<&Path>) -> Option<Located> {
    located(cwd, home, is_set_up)
}

/// Where `cwd` sits, as [`locate`] says, the root being the nearest folder holding any `.cortex`
/// folder, set up here or not, such as a fresh clone of a repository whose team shares facts.
pub fn locate_marked(cwd: &Path, home: Option<&Path>) -> Option<Located> {
    located(cwd, home, Path::is_dir)
}

/// Where `cwd` sits, the root being the nearest folder at or above it, below `home`, whose
/// `.cortex` `marks` accepts, or which is a linked working tree of a repository whose main tree's
/// `.cortex` it accepts.
fn located(cwd: &Path, home: Option<&Path>, marks: impl Fn(&Path) -> bool) -> Option<Located> {
    let canonical = canonical_root(cwd).ok()?;
    let home = home.and_then(|home| canonical_root(home).ok());
    let (root, main_root) = canonical
        .ancestors()
        .take_while(|folder| Some(*folder) != home.as_deref())
        .find_map(|folder| root_at(folder, &marks, home.as_deref()))?;
    // The agent's spelling of the root drops as many folders from its working folder as lie
    // between the canonical working folder and the root: through a link, the names differ. A link
    // that leads into the repository below its root gives a folder that is not the root, kept only
    // when it leads to the root itself.
    let depth = canonical.components().count() - root.components().count();
    let agent_root = cwd
        .ancestors()
        .nth(depth)
        .filter(|spelled| canonical_root(spelled).is_ok_and(|resolved| resolved == root))
        .map_or_else(|| root.clone(), Path::to_path_buf);
    Some(Located {
        cwd: canonical,
        root,
        agent_root,
        main_root,
    })
}

/// Whether `folder` is a root for `marks`: itself when its `.cortex` is one `marks` accepts; itself
/// with its repository's main working tree when it is a linked working tree's top and that tree,
/// which is not `home` nor holds it, has a `.cortex` `marks` accepts; none otherwise.
fn root_at(
    folder: &Path,
    marks: &impl Fn(&Path) -> bool,
    home: Option<&Path>,
) -> Option<(PathBuf, Option<PathBuf>)> {
    if marks(&folder.join(MARKER)) {
        return Some((folder.to_path_buf(), None));
    }
    // Only a `.git` file can lead to another tree: a folder's `.git` folder is its own repository.
    if !folder.join(DOT_GIT).is_file() {
        return None;
    }
    let layout = git_layout(folder).filter(cortex_core::GitLayout::is_linked)?;
    let main_root = layout.main_root?;
    let is_below_home = home.is_none_or(|home| !home.starts_with(&main_root));
    (is_below_home && marks(&main_root.join(MARKER)))
        .then(|| (folder.to_path_buf(), Some(main_root)))
}

/// Whether `marker`, a folder's `.cortex`, marks a repository set up here: a folder holding
/// anything but the shared folder alone (files the system made aside), or nothing at all. A folder
/// that cannot be read marks nothing, and the hook leaves it alone.
fn is_set_up(marker: &Path) -> bool {
    if !marker.is_dir() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(marker) else {
        return false;
    };
    // Two names at most tell the shared folder alone from anything else.
    let mut names = entries
        .map_while(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| !MADE_BY_THE_SYSTEM.iter().any(|made| name == made));
    !matches!((names.next(), names.next()), (Some(only), None) if only == SHARED)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn Error>>;

    /// The nearest folder holding `.cortex` is the root, from the root itself or below it. Inside
    /// the home folder the search stops below it: neither the home folder's `.cortex` nor one
    /// above the home folder makes a repository. A folder that does not exist has no root.
    #[test]
    fn the_nearest_marked_folder_below_home_is_the_root() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        std::fs::create_dir(base.join(MARKER))?;
        let home = base.join("home");
        std::fs::create_dir_all(home.join(MARKER))?;
        let repo = home.join("repo");
        std::fs::create_dir_all(repo.join(MARKER))?;
        std::fs::create_dir_all(repo.join("src/inner"))?;
        let loose = home.join("loose");
        std::fs::create_dir(&loose)?;

        let deep = locate(&repo.join("src/inner"), Some(&home)).ok_or("no root")?;
        assert_eq!(deep.root, repo);
        assert_eq!(deep.cwd, repo.join("src").join("inner"));
        assert_eq!(deep.agent_root, repo);
        assert_eq!(
            locate(&repo, Some(&home)).map(|located| located.root),
            Some(repo.clone())
        );
        assert_eq!(locate(&loose, Some(&home)), None);
        assert_eq!(locate(&home, Some(&home)), None);
        assert_eq!(locate(&repo.join("missing"), Some(&home)), None);
        std::fs::write(repo.join("src/file.txt"), "text")?;
        assert_eq!(locate(&repo.join("src/file.txt"), Some(&home)), None);
        // A repository inside another is its own.
        let nested = repo.join("src/inner/nested");
        std::fs::create_dir_all(nested.join(MARKER))?;
        assert_eq!(
            locate(&nested, Some(&home)).map(|located| located.root),
            Some(nested.clone())
        );
        // Outside the home folder the search goes up as far as it must: here, to the marked base.
        let outside = base.join("outside");
        std::fs::create_dir(&outside)?;
        assert_eq!(
            locate(&outside, Some(&home)).map(|located| located.root),
            Some(base.clone())
        );
        Ok(())
    }

    /// A clone of a repository whose team shares facts holds `.cortex/shared/` and nothing else
    /// of Cortex's (a `.DS_Store` the system left beside it counts for nothing): it is not set up,
    /// though a command finds it as its root, and the search goes on above it, to the home folder (here the
    /// temporary folder), where it stops. Once the
    /// user sets it up and `.cortex` holds more, it is the root; an empty `.cortex` marks it too.
    #[test]
    fn the_shared_folder_alone_marks_nothing() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let clone = base.join("clone");
        std::fs::create_dir_all(clone.join(MARKER).join(SHARED))?;
        std::fs::write(clone.join(MARKER).join(SHARED).join("facts.json"), "{}")?;
        std::fs::write(clone.join(MARKER).join(".DS_Store"), "")?;
        std::fs::create_dir_all(clone.join("src"))?;
        assert_eq!(locate(&clone.join("src"), Some(&base)), None);
        assert_eq!(
            locate_marked(&clone.join("src"), Some(&base)).map(|located| located.root),
            Some(clone.clone())
        );
        std::fs::write(clone.join(MARKER).join("config.toml"), "")?;
        assert_eq!(
            locate(&clone.join("src"), Some(&base)).map(|located| located.root),
            Some(clone.clone())
        );
        let empty = base.join("empty");
        std::fs::create_dir_all(empty.join(MARKER))?;
        assert_eq!(
            locate(&empty, Some(&base)).map(|located| located.root),
            Some(empty.clone())
        );
        Ok(())
    }

    /// Reached through a link, the root keeps the agent's spelling for its tool paths and the
    /// canonical one for its identity.
    #[test]
    fn a_linked_folder_keeps_both_spellings() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let repo = base.join("real");
        std::fs::create_dir_all(repo.join(MARKER))?;
        std::fs::create_dir_all(repo.join("src"))?;
        let link = base.join("link");
        make_link(&repo, &link)?;

        let located = locate(&link.join("src"), None).ok_or("no root")?;
        assert_eq!(located.root, repo);
        assert_eq!(located.cwd, repo.join("src"));
        assert_eq!(located.agent_root, link);

        // A link into the repository below its root: `deep` leads to `real/src/a`, so from
        // `deep/b` the root is three folders up the canonical path (past `b`, `a` and `src`),
        // while three folders up the agent's path is the folder above `base`, which is not the
        // root. The root keeps its canonical spelling.
        std::fs::create_dir_all(repo.join("src/a/b"))?;
        let deep = base.join("deep");
        // `cmd` reads a `/` as the start of a switch: the target is joined a folder at a time.
        make_link(&repo.join("src").join("a"), &deep)?;
        let below = locate(&deep.join("b"), None).ok_or("no root")?;
        assert_eq!(below.root, repo);
        assert_eq!(below.agent_root, repo);
        Ok(())
    }

    /// A linked working tree `wt` of the main tree `main` under `base`, written as git writes one:
    /// its `.git` file names `main/.git/worktrees/wt`, which names it back and holds `../..` as
    /// its common folder. Returns both tops.
    fn linked_tree(base: &Path) -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
        let main = base.join("main");
        let state = main.join(".git").join("worktrees").join("wt");
        std::fs::create_dir_all(&state)?;
        let linked = base.join("wt");
        std::fs::create_dir_all(linked.join("src"))?;
        std::fs::write(
            linked.join(DOT_GIT),
            format!("gitdir: {}\n", state.display()),
        )?;
        std::fs::write(state.join("commondir"), "../..\n")?;
        std::fs::write(
            state.join("gitdir"),
            format!("{}\n", linked.join(DOT_GIT).display()),
        )?;
        Ok((main, linked))
    }

    /// A linked working tree holds no `.cortex`: from inside it, the root is its own top, which
    /// paths are relative to, and the repository its main tree, once that is set up. Not set up,
    /// or with a `.git` file that names nothing, it is no root, and the search goes on above it to
    /// the base. A linked working tree set up on its own, as before Cortex knew of them, is its
    /// own repository.
    #[test]
    fn a_linked_working_tree_belongs_to_its_main_tree() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let (main, linked) = linked_tree(&base)?;
        std::fs::create_dir_all(main.join(MARKER))?;
        std::fs::write(main.join(MARKER).join("config.toml"), "")?;

        let located = locate(&linked.join("src"), Some(&base)).ok_or("no root")?;
        assert_eq!(located.root, linked);
        assert_eq!(located.agent_root, linked);
        assert_eq!(located.cwd, linked.join("src"));
        assert_eq!(located.main_root.as_deref(), Some(main.as_path()));
        assert_eq!(located.repository_root(), main.as_path());
        let from_main = locate(&main, Some(&base)).ok_or("no root")?;
        assert_eq!(
            (from_main.root.as_path(), from_main.main_root),
            (main.as_path(), None)
        );

        std::fs::write(linked.join(DOT_GIT), "gitdir: ../nowhere\n")?;
        assert_eq!(locate(&linked.join("src"), Some(&base)), None);
        let (_, relinked) = linked_tree(&base)?;
        std::fs::remove_file(main.join(MARKER).join("config.toml"))?;
        std::fs::create_dir_all(main.join(MARKER).join(SHARED))?;
        assert_eq!(locate(&relinked, Some(&base)), None);
        assert_eq!(
            locate_marked(&relinked, Some(&base)).and_then(|located| located.main_root),
            Some(main.clone())
        );

        std::fs::create_dir_all(relinked.join(MARKER))?;
        let own = locate(&relinked, Some(&base)).ok_or("no root")?;
        assert_eq!((own.root, own.main_root), (relinked, None));
        Ok(())
    }

    /// Makes `link` a symbolic link to the folder `target`.
    #[cfg(unix)]
    fn make_link(target: &Path, link: &Path) -> TestResult {
        Ok(std::os::unix::fs::symlink(target, link)?)
    }

    /// Makes `link` a junction to the folder `target`: unlike a symbolic link, a junction needs no
    /// privilege on Windows.
    #[cfg(windows)]
    fn make_link(target: &Path, link: &Path) -> TestResult {
        let made = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()?;
        assert!(made.status.success(), "{made:?}");
        Ok(())
    }
}
