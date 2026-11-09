//! The repository a hook's working folder belongs to: the nearest folder at or above it that holds
//! a `.cortex` folder of its own. A `.cortex` holding nothing but `shared` is not one: that is what
//! git checks out in a clone of a repository whose team shares facts, where the user who cloned it
//! never set Cortex up. Inside the user's home folder the search stops below the home folder, whose
//! `.cortex` is the user's own folder and which no repository contains. A folder with no marked
//! folder above it was never set up for Cortex, and the hook leaves it alone.

use std::path::Path;

use cortex_core::canonical_root;

use crate::adapter::Located;

/// The folder that marks a repository's root.
const MARKER: &str = ".cortex";

/// The folder inside the marker that a team commits, and that alone marks nothing.
const SHARED: &str = "shared";

/// Where the working folder `cwd`, as the agent spells it, sits: its canonical form, its
/// repository's canonical root, and that root as the agent spells it. `None` when `cwd` is not a
/// folder or no repository holds it. `home` is the user's home folder: neither it nor anything
/// above it is a root for a folder inside it.
pub fn locate(cwd: &Path, home: Option<&Path>) -> Option<Located> {
    let canonical = canonical_root(cwd).ok()?;
    let home = home.and_then(|home| canonical_root(home).ok());
    let root = canonical
        .ancestors()
        .take_while(|folder| Some(*folder) != home.as_deref())
        .find(|folder| is_set_up(&folder.join(MARKER)))?
        .to_path_buf();
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
    })
}

/// Whether `marker`, a folder's `.cortex`, marks a repository set up here: a folder holding
/// anything but the shared folder alone, or nothing at all. A folder that cannot be read marks
/// nothing, and the hook leaves it alone.
fn is_set_up(marker: &Path) -> bool {
    if !marker.is_dir() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(marker) else {
        return false;
    };
    // Two names at most tell the shared folder alone from anything else.
    let mut names = entries.map_while(Result::ok).map(|entry| entry.file_name());
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
    /// of Cortex's: it is not set up, and the search goes on above it, to the home folder (here the
    /// temporary folder), where it stops. Once the
    /// user sets it up and `.cortex` holds more, it is the root; an empty `.cortex` marks it too.
    #[test]
    fn the_shared_folder_alone_marks_nothing() -> TestResult {
        let base = tempfile::tempdir()?;
        let base = canonical_root(base.path())?;
        let clone = base.join("clone");
        std::fs::create_dir_all(clone.join(MARKER).join(SHARED))?;
        std::fs::write(clone.join(MARKER).join(SHARED).join("facts.json"), "{}")?;
        std::fs::create_dir_all(clone.join("src"))?;
        assert_eq!(locate(&clone.join("src"), Some(&base)), None);
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
