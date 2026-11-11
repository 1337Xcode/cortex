//! The repository a hook's working folder belongs to: the nearest folder at or above it that holds
//! a `.cortex` folder of its own and that the user's own folder records as set up on this machine
//! ([`cortex_core::is_set_up_here`]). A `.cortex` alone says nothing: a repository may commit any
//! file in it, so a clone, or an archive of one, could carry one; that is a folder never set up
//! here, and the hook leaves it alone (or offers it to be set up, when adoption is on, by the
//! same rules as any other). A command finds such a folder's root all the same
//! ([`locate_marked`]): it is still the repository's root, though no hook records there. Inside
//! the user's home folder the search stops below the home folder, whose `.cortex` is the user's own
//! folder and which no repository contains.
//!
//! A linked working tree (`git worktree add`) holds no `.cortex` of its own: its `.git` is a file
//! naming its repository, whose main working tree holds one. Met on the way up before any marked
//! folder, it is where the working folder sits, and the repository is its main tree's, read from
//! git's files with no git run ([`cortex_core::git_layout`]); a layout that cannot be read is no
//! linked working tree, and the search goes on above it as for any folder.
//!
//! A folder whose `.cortex` holds `off` (`cortex off`) ends the hook's search: a working folder at
//! or below it, a linked working tree of it included, has no repository, and the hook stays silent.

use std::path::{Path, PathBuf};

use cortex_core::{UserFolder, canonical_root, git_layout, is_set_up_here, is_turned_off};

use crate::adapter::Located;

/// The folder that marks a repository's root.
const MARKER: &str = ".cortex";

/// What a working tree's top holds that leads to its repository: a file in a linked working tree.
const DOT_GIT: &str = ".git";

/// Where the working folder `cwd`, as the agent spells it, sits: its canonical form, its
/// repository's canonical root, and that root as the agent spells it. `None` when `cwd` is not a
/// folder, no repository set up for Cortex on this machine holds it, or a `.cortex` holding `off`
/// is met first. `home` is the user's home folder: neither it nor anything above it is a root for a
/// folder inside it, and its `.cortex` keeps the records of what is set up; with no home folder,
/// nothing is.
#[must_use]
pub fn locate(cwd: &Path, home: Option<&Path>) -> Option<Located> {
    let user = home.map(UserFolder::in_home)?;
    let is_set_up = |marker: &Path| {
        marker.is_dir()
            && marker
                .parent()
                .is_some_and(|root| is_set_up_here(&user, root))
    };
    located(cwd, home, (is_set_up, is_turned_off))
        .filter(|located| !is_turned_off(located.repository_root()))
}

/// Where `cwd` sits, as [`locate`] says, the root being the nearest folder holding any `.cortex`
/// folder, set up here or not, turned off or not, such as a fresh clone of a repository whose team
/// shares facts.
#[must_use]
pub fn locate_marked(cwd: &Path, home: Option<&Path>) -> Option<Located> {
    located(cwd, home, (Path::is_dir, |_: &Path| false))
}

/// Where `cwd` sits, the root being the nearest folder at or above it, below `home` and below any
/// folder `stops` takes, whose `.cortex` `marks` accepts, or which is a linked working tree of a
/// repository whose main tree's `.cortex` it accepts.
fn located(
    cwd: &Path,
    home: Option<&Path>,
    (marks, stops): (impl Fn(&Path) -> bool, impl Fn(&Path) -> bool),
) -> Option<Located> {
    let canonical = canonical_root(cwd).ok()?;
    let home = home.and_then(|home| canonical_root(home).ok());
    let (root, main_root) = canonical
        .ancestors()
        .take_while(|folder| Some(*folder) != home.as_deref() && !stops(folder))
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

#[cfg(test)]
mod tests {
    use std::error::Error;

    use cortex_core::{OFF_FILE, record_set_up};

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn Error>>;

    /// Gives `root` a `.cortex` and records it as set up for the user whose home is `home`.
    fn set_up(root: &Path, home: &Path) -> TestResult {
        std::fs::create_dir_all(root.join(MARKER))?;
        record_set_up(&UserFolder::in_home(home), &canonical_root(root)?)?;
        Ok(())
    }

    /// The nearest folder set up here is the root, from the root itself or below it. Inside the
    /// home folder the search stops below it. A folder that does not exist has no root, and a
    /// repository inside another is its own. With no home folder, nothing is set up.
    #[test]
    fn the_nearest_folder_set_up_here_is_the_root() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let home = base.join("home");
        std::fs::create_dir_all(home.join(MARKER))?;
        let repo = home.join("repo");
        set_up(&repo, &home)?;
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
        assert_eq!(locate(&repo, None), None);
        assert_eq!(locate(&loose, Some(&home)), None);
        assert_eq!(locate(&home, Some(&home)), None);
        assert_eq!(locate(&repo.join("missing"), Some(&home)), None);
        std::fs::write(repo.join("src/file.txt"), "text")?;
        assert_eq!(locate(&repo.join("src/file.txt"), Some(&home)), None);
        let nested = repo.join("src/inner/nested");
        set_up(&nested, &home)?;
        assert_eq!(
            locate(&nested, Some(&home)).map(|located| located.root),
            Some(nested.clone())
        );
        Ok(())
    }

    /// A `.cortex` the repository carries, whatever it holds (a team's shared facts, a
    /// configuration), is no root until this machine records the repository as set up; a command
    /// finds it as its root all the same, and the search goes on above it for the hook.
    #[test]
    fn a_cortex_folder_alone_is_not_set_up_here() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let home = base.join("home");
        let outer = home.join("outer");
        set_up(&outer, &home)?;
        let clone = outer.join("clone");
        std::fs::create_dir_all(clone.join(MARKER).join("shared"))?;
        std::fs::write(clone.join(MARKER).join("config.toml"), "")?;
        std::fs::create_dir_all(clone.join("src"))?;
        assert_eq!(
            locate(&clone.join("src"), Some(&home)).map(|located| located.root),
            Some(outer.clone())
        );
        assert_eq!(
            locate_marked(&clone.join("src"), Some(&home)).map(|located| located.root),
            Some(clone.clone())
        );
        record_set_up(&UserFolder::in_home(&home), &clone)?;
        assert_eq!(
            locate(&clone.join("src"), Some(&home)).map(|located| located.root),
            Some(clone)
        );
        Ok(())
    }

    /// A folder whose `.cortex` holds `off` ends the search, from itself, from below it, and from
    /// a linked working tree of it: the repository set up above it is not reached. A command still
    /// finds it. Once `off` is gone it is the root again.
    #[test]
    fn a_repository_turned_off_is_left_alone() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let home = base.join("home");
        let outer = home.join("outer");
        set_up(&outer, &home)?;
        let (main, linked) = linked_tree(&outer)?;
        set_up(&main, &home)?;
        std::fs::create_dir_all(main.join("src"))?;
        let off = main.join(MARKER).join(OFF_FILE);
        std::fs::write(&off, "")?;
        assert_eq!(locate(&main.join("src"), Some(&home)), None);
        assert_eq!(locate(&main, Some(&home)), None);
        assert_eq!(locate(&linked.join("src"), Some(&home)), None);
        assert_eq!(
            locate_marked(&main.join("src"), Some(&home)).map(|located| located.root),
            Some(main.clone())
        );
        // Turned off and never set up here: the search stops there all the same.
        let user = UserFolder::in_home(&home);
        cortex_core::forget_set_up(&user, &main)?;
        assert_eq!(locate(&main.join("src"), Some(&home)), None);
        record_set_up(&user, &main)?;
        std::fs::remove_file(&off)?;
        assert_eq!(
            locate(&main.join("src"), Some(&home)).map(|located| located.root),
            Some(main.clone())
        );
        Ok(())
    }

    /// Reached through a link, the root keeps the agent's spelling for its tool paths and the
    /// canonical one for its identity.
    #[test]
    fn a_linked_folder_keeps_both_spellings() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let home = base.join("home");
        let repo = base.join("real");
        set_up(&repo, &home)?;
        std::fs::create_dir_all(repo.join("src"))?;
        let link = base.join("link");
        make_link(&repo, &link)?;

        let located = locate(&link.join("src"), Some(&home)).ok_or("no root")?;
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
        let below = locate(&deep.join("b"), Some(&home)).ok_or("no root")?;
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
    /// paths are relative to, and the repository its main tree, once that is set up here. Not set
    /// up here, or with a `.git` file that names nothing, it is no root, and the search goes on
    /// above it. A linked working tree set up on its own, as before Cortex knew of them, is its own
    /// repository.
    #[test]
    fn a_linked_working_tree_belongs_to_its_main_tree() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let home = base.join("home");
        let (main, linked) = linked_tree(&base)?;
        set_up(&main, &home)?;

        let located = locate(&linked.join("src"), Some(&home)).ok_or("no root")?;
        assert_eq!(located.root, linked);
        assert_eq!(located.agent_root, linked);
        assert_eq!(located.cwd, linked.join("src"));
        assert_eq!(located.main_root.as_deref(), Some(main.as_path()));
        assert_eq!(located.repository_root(), main.as_path());
        let from_main = locate(&main, Some(&home)).ok_or("no root")?;
        assert_eq!(
            (from_main.root.as_path(), from_main.main_root),
            (main.as_path(), None)
        );

        std::fs::write(linked.join(DOT_GIT), "gitdir: ../nowhere\n")?;
        assert_eq!(locate(&linked.join("src"), Some(&home)), None);
        let (_, relinked) = linked_tree(&base)?;
        cortex_core::forget_set_up(&UserFolder::in_home(&home), &main)?;
        assert_eq!(locate(&relinked, Some(&home)), None);
        assert_eq!(
            locate_marked(&relinked, Some(&home)).and_then(|located| located.main_root),
            Some(main.clone())
        );

        set_up(&relinked, &home)?;
        let own = locate(&relinked, Some(&home)).ok_or("no root")?;
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
