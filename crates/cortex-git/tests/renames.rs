//! How each source recognises renamed files, and what neither does while comparing them: read
//! contents it does not need, follow attributes, run a program the repository names. Each test
//! makes a repository with git and compares what both sources read, `gix` and the `git` program,
//! with changes written by hand; where git's own view decides, git is asked.

mod common;

use common::{Repo, TestResult, changed, commit, every_commit, lines, read_both, renamed};
use cortex_git::{CliHistory, GitError, GixHistory, HistorySource, Window};

/// A file moved with nine of its ten lines kept is a rename, named with the path it had. A file
/// moved with every line rewritten shares nothing with what it was: a deletion and an addition.
#[test]
fn a_rename_names_the_path_it_had() -> TestResult {
    let repo = Repo::new()?;
    repo.write("old/name.rs", &lines("kept", 10))?;
    repo.write("other.rs", &lines("first", 10))?;
    let first = repo.commit(1)?;
    repo.move_file("old/name.rs", "new/name.rs", Some("changed 10"))?;
    let moved = repo.commit(2)?;
    repo.remove("other.rs")?;
    repo.write("gone.rs", &lines("second", 10))?;
    let rewritten = repo.commit(3)?;

    let expected = vec![
        commit(
            &rewritten,
            3,
            Some(vec![changed("gone.rs")?, changed("other.rs")?]),
        ),
        commit(
            &moved,
            2,
            Some(vec![renamed("old/name.rs", "new/name.rs")?]),
        ),
        commit(
            &first,
            1,
            Some(vec![changed("old/name.rs")?, changed("other.rs")?]),
        ),
    ];
    for (name, mined) in read_both(&repo.root(), &every_commit())? {
        assert_eq!(mined.commits, expected, "{name}");
    }
    Ok(())
}

/// Renames are found in a commit as large as the cut allows: 3 files each moved with an edit,
/// under a cut of 3, are 3 renames, so the commit is read whole.
#[test]
fn renames_up_to_the_cut_are_found() -> TestResult {
    let repo = Repo::new()?;
    for index in 1..=3 {
        repo.write(
            &format!("old{index}.txt"),
            &lines(&format!("file {index}"), 10),
        )?;
    }
    repo.commit(1)?;
    for index in 1..=3 {
        repo.move_file(
            &format!("old{index}.txt"),
            &format!("new{index}.txt"),
            Some("changed"),
        )?;
    }
    let moved = repo.commit(2)?;

    let window = Window {
        max_files: 3,
        ..every_commit()
    };
    let expected = commit(
        &moved,
        2,
        Some(vec![
            renamed("old1.txt", "new1.txt")?,
            renamed("old2.txt", "new2.txt")?,
            renamed("old3.txt", "new3.txt")?,
        ]),
    );
    for (name, mined) in read_both(&repo.root(), &window)? {
        assert_eq!(mined.commits.first(), Some(&expected), "{name}");
    }
    Ok(())
}

/// A commit with more deletions, or more additions, than the cut is cut without a single file
/// compared for renames. Here 4 files are each moved with an edit, past a cut of 3, and the
/// contents of the 4 files deleted are then removed from the repository: comparing any of them
/// would fail the read.
#[test]
fn a_mass_commit_compares_no_contents() -> TestResult {
    let repo = Repo::new()?;
    for index in 1..=4 {
        repo.write(
            &format!("old{index}.txt"),
            &lines(&format!("file {index}"), 10),
        )?;
    }
    let day_1 = repo.commit(1)?;
    let mut deleted = Vec::new();
    for index in 1..=4 {
        deleted.push(repo.git(&["rev-parse", &format!("HEAD:old{index}.txt")])?);
        repo.move_file(
            &format!("old{index}.txt"),
            &format!("new{index}.txt"),
            Some("changed"),
        )?;
    }
    let day_2 = repo.commit(2)?;
    for id in &deleted {
        repo.remove_object(id)?;
    }

    let window = Window {
        max_files: 3,
        ..every_commit()
    };
    let expected = vec![commit(&day_2, 2, None), commit(&day_1, 1, None)];
    for (name, mined) in read_both(&repo.root(), &window)? {
        assert_eq!(mined.commits, expected, "{name}");
    }
    Ok(())
}

/// For `gix`, a file over 1 MiB is a rename only when its contents did not change, even where the
/// repository's own configuration would compare files up to 512 MiB: one moved with its last line
/// changed is a deletion and an addition, one moved unchanged a rename. Git compares files of any
/// size, so the `git` program finds both renames.
#[test]
fn a_large_file_is_renamed_only_when_unchanged() -> TestResult {
    let repo = Repo::new()?;
    // 100,000 lines of 14 to 18 bytes each: about 1.6 MB a file.
    repo.write("big1.txt", &lines("big one", 100_000))?;
    repo.write("big2.txt", &lines("big two", 100_000))?;
    repo.commit(1)?;
    repo.move_file("big1.txt", "moved1.txt", Some("changed"))?;
    repo.move_file("big2.txt", "moved2.txt", None)?;
    let moved = repo.commit(2)?;
    repo.git(&["config", "core.bigFileThreshold", "512m"])?;

    let by_gix = commit(
        &moved,
        2,
        Some(vec![
            changed("big1.txt")?,
            changed("moved1.txt")?,
            renamed("big2.txt", "moved2.txt")?,
        ]),
    );
    let by_git = commit(
        &moved,
        2,
        Some(vec![
            renamed("big1.txt", "moved1.txt")?,
            renamed("big2.txt", "moved2.txt")?,
        ]),
    );
    let mined = GixHistory::open(&repo.root())?.read(&every_commit())?;
    assert_eq!(mined.commits.first(), Some(&by_gix));
    let mined = CliHistory::open(&repo.root())?.read(&every_commit())?;
    assert_eq!(mined.commits.first(), Some(&by_git));
    Ok(())
}

/// A partial clone holds only the contents it has needed, here those of the newest commit: a file
/// moved with an edit is a deletion and an addition, since comparing it would need the contents
/// it had, and a file moved unchanged is a rename. The read says the clone is partial. The clone's
/// origin is gone before the read, so a source that tried to fetch what the clone left out would
/// fail.
#[test]
fn a_partial_clone_matches_identical_contents_only() -> TestResult {
    let origin = Repo::new()?;
    origin.git(&["config", "uploadpack.allowFilter", "true"])?;
    origin.write("a.txt", &lines("alpha", 10))?;
    origin.write("b.txt", &lines("beta", 10))?;
    let first = origin.commit(1)?;
    origin.move_file("a.txt", "c.txt", Some("changed"))?;
    origin.move_file("b.txt", "d.txt", None)?;
    let moved = origin.commit(2)?;
    let clone = Repo::clone_of(&origin, &["--filter=blob:none"])?;
    drop(origin);

    let expected = vec![
        commit(
            &moved,
            2,
            Some(vec![
                changed("a.txt")?,
                changed("c.txt")?,
                renamed("b.txt", "d.txt")?,
            ]),
        ),
        commit(&first, 1, Some(vec![changed("a.txt")?, changed("b.txt")?])),
    ];
    let mined = GixHistory::open(&clone.root())?.read(&every_commit())?;
    assert_eq!(mined.commits, expected);
    assert!(mined.is_partial);
    // Git older than 2.45 cannot be told not to fetch, so the git source refuses the clone.
    let listed = CliHistory::open(&clone.root())?.read(&every_commit());
    if git_version()? >= (2, 45) {
        let mined = listed?;
        assert_eq!(mined.commits, expected);
        assert!(mined.is_partial);
    } else {
        assert!(matches!(listed, Err(GitError::Walk(_))), "{listed:?}");
    }
    Ok(())
}

/// The major and minor version of the git on the `PATH`, from `git version`.
fn git_version() -> TestResult<(u32, u32)> {
    let output = std::process::Command::new("git").arg("version").output()?;
    let text = String::from_utf8(output.stdout)?;
    let number = text
        .split_whitespace()
        .nth(2)
        .ok_or("git version printed no number")?;
    let mut parts = number.split('.').map(str::parse::<u32>);
    let major = parts.next().ok_or("no major version")??;
    let minor = parts.next().ok_or("no minor version")??;
    Ok((major, minor))
}

/// Attributes take no part in renames, as in git: files marked as not to be diffed, by the
/// repository's `.gitattributes` and by an attributes file its configuration names outside it, are
/// still compared, and git shows the same rename.
#[test]
fn attributes_take_no_part_in_renames() -> TestResult {
    let repo = Repo::new()?;
    repo.write(".gitattributes", "*.txt -diff\n")?;
    repo.write("a.txt", &lines("kept", 10))?;
    repo.commit(1)?;
    let outside = repo.home().join("attributes");
    std::fs::write(&outside, "*.txt -diff\n")?;
    let outside = outside
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    repo.git(&["config", "core.attributesFile", outside])?;
    repo.move_file("a.txt", "b.txt", Some("changed"))?;
    let moved = repo.commit(2)?;

    let shown = repo.git(&["log", "-1", "-M", "--name-status", "--format="])?;
    assert!(
        shown.starts_with('R') && shown.ends_with("a.txt\tb.txt"),
        "{shown}"
    );
    let expected = commit(&moved, 2, Some(vec![renamed("a.txt", "b.txt")?]));
    for (name, mined) in read_both(&repo.root(), &every_commit())? {
        assert_eq!(mined.commits.first(), Some(&expected), "{name}");
    }
    Ok(())
}

/// Finding a rename by content runs no program the repository names. The repository below sets
/// up a text conversion for its `.txt` files; git runs it when it shows the diff of the rename,
/// which proves the set-up works, and reading the same history runs nothing.
#[test]
fn reading_runs_no_program_the_repository_names() -> TestResult {
    let repo = Repo::new()?;
    repo.write(".gitattributes", "*.txt diff=probe\n")?;
    repo.write("a.txt", &lines("kept", 10))?;
    repo.commit(1)?;
    repo.move_file("a.txt", "b.txt", Some("changed"))?;
    let moved = repo.commit(2)?;

    let marker = repo.home().join("ran");
    let probe = repo.home().join("probe.sh");
    let marker_path = marker
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    std::fs::write(
        &probe,
        format!(
            "#!/bin/sh\necho ran > '{}'\ncat \"$1\"\n",
            marker_path.replace('\\', "/")
        ),
    )?;
    make_executable(&probe)?;
    let probe_path = probe
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    let config = repo.root().join(".git").join("config");
    let mut text = std::fs::read_to_string(&config)?;
    text.push_str(&format!(
        "[diff \"probe\"]\n\ttextconv = '{}'\n",
        probe_path.replace('\\', "/")
    ));
    std::fs::write(&config, text)?;
    repo.git(&["diff", "HEAD~1", "HEAD"])?;
    assert!(marker.exists(), "git did not run the text conversion");
    std::fs::remove_file(&marker)?;

    let expected = commit(&moved, 2, Some(vec![renamed("a.txt", "b.txt")?]));
    for (name, mined) in read_both(&repo.root(), &every_commit())? {
        assert_eq!(mined.commits.first(), Some(&expected), "{name}");
        assert!(
            !marker.exists(),
            "{name}: reading the history ran the text conversion"
        );
    }
    Ok(())
}

/// For `gix`, a path no file can have is left out of a commit's changes and the commit's other
/// files stay. The tree below, written by hand as git never writes one, names a file with no name,
/// a file `..` and a file whose name holds a backslash beside `ok.txt`. The `git` program refuses
/// to list such a tree, so that source cannot read the history.
#[test]
fn a_path_no_file_can_have_is_left_out() -> TestResult {
    let repo = Repo::new()?;
    let blob = repo.git_fed(1, &["hash-object", "-w", "--stdin"], b"text\n")?;
    let raw_id = (0..blob.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(blob.get(at..at + 2).unwrap_or_default(), 16))
        .collect::<Result<Vec<u8>, _>>()?;
    let mut tree = Vec::new();
    for name in [&b""[..], b"..", b"a\\b", b"ok.txt"] {
        tree.extend_from_slice(b"100644 ");
        tree.extend_from_slice(name);
        tree.push(0);
        tree.extend_from_slice(&raw_id);
    }
    let tree_id = repo.git_fed(
        1,
        &["hash-object", "-t", "tree", "--literally", "-w", "--stdin"],
        &tree,
    )?;
    let crafted = repo.git_on(1, &["commit-tree", &tree_id, "-m", "crafted"])?;
    repo.git(&["update-ref", "refs/heads/main", &crafted])?;

    let mined = GixHistory::open(&repo.root())?.read(&every_commit())?;
    assert_eq!(
        mined.commits,
        vec![commit(&crafted, 1, Some(vec![changed("ok.txt")?]))]
    );
    let listed = CliHistory::open(&repo.root())?.read(&every_commit());
    assert!(matches!(listed, Err(GitError::Walk(_))), "{listed:?}");
    Ok(())
}

/// A path that is not UTF-8 is left out of a commit's changes and the commit's other files stay;
/// a rename from such a path is a change of the new path alone. Unix only: Windows file names are
/// UTF-16, and git on Windows cannot make a name that is not UTF-8.
#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_is_left_out() -> TestResult {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let repo = Repo::new()?;
    let bad = repo.root().join(OsStr::from_bytes(b"bad\xff.txt"));
    repo.write("ok.txt", &lines("ok", 3))?;
    std::fs::write(&bad, lines("bad", 3))?;
    let first = repo.commit(1)?;
    std::fs::rename(&bad, repo.root().join("good.txt"))?;
    let second = repo.commit(2)?;

    let expected = vec![
        commit(&second, 2, Some(vec![changed("good.txt")?])),
        commit(&first, 1, Some(vec![changed("ok.txt")?])),
    ];
    for (name, mined) in read_both(&repo.root(), &every_commit())? {
        assert_eq!(mined.commits, expected, "{name}");
    }
    Ok(())
}

/// Makes the file at `path` executable by its owner, as git needs of a program it runs.
fn make_executable(path: &std::path::Path) -> TestResult {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    // On Windows a file's name, not a permission, makes it a program; git's shell runs a script.
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
