//! The repository a hook's working folder belongs to: the nearest folder at or above it that holds
//! a `.cortex` folder. Inside the user's home folder the search stops below the home folder, whose
//! `.cortex` is the user's own folder and which no repository contains. A folder with no marked
//! folder above it was never set up for Cortex, and the hook leaves it alone.

use std::path::Path;

use cortex_core::canonical_root;

use crate::adapter::Located;

/// The folder that marks a repository's root.
const MARKER: &str = ".cortex";

/// Where the working folder `cwd`, as the agent spells it, sits: its canonical form, its
/// repository's canonical root, and that root as the agent spells it. `None` when `cwd` is not a
/// folder or no repository holds it. `home` is the user's home folder: neither it nor anything
/// above it is a root for a folder inside it.
pub(crate) fn locate(cwd: &Path, home: Option<&Path>) -> Option<Located> {
    let canonical = canonical_root(cwd).ok()?;
    let home = home.and_then(|home| canonical_root(home).ok());
    let root = canonical
        .ancestors()
        .take_while(|folder| Some(*folder) != home.as_deref())
        .find(|folder| folder.join(MARKER).is_dir())?
        .to_path_buf();
    // The agent's spelling of the root drops as many folders from its working folder as lie
    // between the canonical working folder and the root: through a link, the names differ.
    let depth = canonical.components().count() - root.components().count();
    let agent_root = cwd
        .ancestors()
        .nth(depth)
        .map_or_else(|| root.clone(), Path::to_path_buf);
    Some(Located {
        cwd: canonical,
        root,
        agent_root,
    })
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
        // Outside the home folder the search goes up as far as it must: here, to the marked base.
        let outside = base.join("outside");
        std::fs::create_dir(&outside)?;
        assert_eq!(
            locate(&outside, Some(&home)).map(|located| located.root),
            Some(base.clone())
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
