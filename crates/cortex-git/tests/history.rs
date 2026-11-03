//! Reading histories with both sources, `gix` and the `git` program: each test makes a repository
//! with git, commit by commit on known days, and compares what each source reads with the commits
//! and paths written by hand. Commit ids come from git itself. How renamed files are recognised is
//! tested in `renames.rs`.

mod common;

use common::{
    DAY_0_S, DAY_S, Repo, TestResult, changed, commit, commit_day, every_commit, lines, read_both,
    sources,
};
use cortex_git::{CliHistory, GitError, GixHistory, Mined, Window};

/// Commits are read newest first, each against its parent, the first against nothing so that
/// every file it holds counts. A file in a folder is named by its whole path with `/`, and a
/// deletion by the path the file had.
#[test]
fn a_history_reads_newest_first_against_each_parent() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", &lines("alpha", 5))?;
    repo.write("src/lib/b.rs", &lines("beta", 5))?;
    let first = repo.commit(1)?;
    repo.write("a.txt", &lines("aleph", 5))?;
    let second = repo.commit(2)?;
    // The repository's own settings for showing logs change nothing that is read.
    for (key, value) in [
        ("color.ui", "always"),
        ("log.showRoot", "false"),
        ("diff.renames", "copies"),
        ("diff.relative", "true"),
        ("core.quotePath", "true"),
    ] {
        repo.git(&["config", key, value])?;
    }
    repo.remove("src/lib/b.rs")?;
    repo.write("src/c.rs", &lines("gamma", 5))?;
    let third = repo.commit(3)?;

    let expected = Mined {
        commits: vec![
            commit(
                &third,
                3,
                Some(vec![changed("src/c.rs")?, changed("src/lib/b.rs")?]),
            ),
            commit(&second, 2, Some(vec![changed("a.txt")?])),
            commit(
                &first,
                1,
                Some(vec![changed("a.txt")?, changed("src/lib/b.rs")?]),
            ),
        ],
        is_stop_reached: false,
        is_shallow: false,
        is_partial: false,
    };
    for (name, history) in sources(&repo.root())? {
        assert_eq!(history.head()?, Some(third.clone()), "{name}");
        assert_eq!(history.read(&every_commit())?, expected, "{name}");
    }
    Ok(())
}

/// A merge counts against its first parent: the files the merged branch brought in are its
/// changes, and the branch's own commit is not on the line read.
#[test]
fn a_merge_counts_against_its_first_parent() -> TestResult {
    let repo = Repo::new()?;
    repo.write("base.txt", &lines("base", 3))?;
    let base = repo.commit(1)?;
    repo.git(&["checkout", "-q", "-b", "side"])?;
    repo.write("side.txt", &lines("side", 3))?;
    repo.commit(2)?;
    repo.git(&["checkout", "-q", "main"])?;
    repo.write("main.txt", &lines("main", 3))?;
    let main = repo.commit(3)?;
    repo.git_on(4, &["merge", "-q", "--no-ff", "--no-edit", "side"])?;
    let merge = repo.git(&["rev-parse", "HEAD"])?;

    let expected = vec![
        commit(&merge, 4, Some(vec![changed("side.txt")?])),
        commit(&main, 3, Some(vec![changed("main.txt")?])),
        commit(&base, 1, Some(vec![changed("base.txt")?])),
    ];
    for (name, mined) in read_both(&repo.root(), &every_commit())? {
        assert_eq!(mined.commits, expected, "{name}");
    }
    Ok(())
}

/// The walk ends at the most commits the window takes, and at the first commit made before its
/// oldest time: with the cut-off at noon on day 1, days 4, 3 and 2 are read and day 1 is not. It
/// starts at the commit the window names, day 2 for days 2 and 1, however far HEAD has moved since;
/// a name that is no commit's id is an error, never an option passed on.
#[test]
fn the_window_bounds_the_walk() -> TestResult {
    let repo = Repo::new()?;
    let day_1 = commit_day(&repo, 1)?;
    let day_2 = commit_day(&repo, 2)?;
    let day_3 = commit_day(&repo, 3)?;
    let day_4 = commit_day(&repo, 4)?;
    let ids = |mined: Mined| -> Vec<String> {
        mined.commits.into_iter().map(|commit| commit.id).collect()
    };

    let newest_two = Window {
        max_commits: 2,
        ..every_commit()
    };
    let since_day_2 = Window {
        oldest_s: DAY_0_S + DAY_S + DAY_S / 2,
        ..every_commit()
    };
    for (name, history) in sources(&repo.root())? {
        assert_eq!(
            ids(history.read(&newest_two)?),
            [day_4.clone(), day_3.clone()],
            "{name}"
        );
        assert_eq!(
            ids(history.read(&since_day_2)?),
            [day_4.clone(), day_3.clone(), day_2.clone()],
            "{name}"
        );
        let all = ids(history.read(&every_commit())?);
        assert_eq!(all.last(), Some(&day_1), "{name}");
        let from_day_2 = Window {
            from: Some(day_2.clone()),
            ..every_commit()
        };
        assert_eq!(
            ids(history.read(&from_day_2)?),
            [day_2.clone(), day_1.clone()],
            "{name}"
        );
        let from_nothing = Window {
            from: Some("--all".to_owned()),
            ..every_commit()
        };
        assert!(history.read(&from_nothing).is_err(), "{name}");
    }
    Ok(())
}

/// A commit that changed more files than the window's cut, 3 here, is read without its files,
/// whether the files it changed were there before (day 3) or are new (day 4); one that changed
/// exactly as many is read whole (day 1). Folders are not files: day 1 adds two, `d` and `d/x`,
/// besides its three files.
#[test]
fn a_mass_commit_is_read_without_its_files() -> TestResult {
    let repo = Repo::new()?;
    repo.write("d/x/1.txt", &lines("one", 2))?;
    repo.write("d/2.txt", &lines("two", 2))?;
    repo.write("3.txt", &lines("three", 2))?;
    let day_1 = repo.commit(1)?;
    repo.write("4.txt", &lines("four", 2))?;
    let day_2 = repo.commit(2)?;
    for (path, word) in [
        ("d/x/1.txt", "uno"),
        ("d/2.txt", "dos"),
        ("3.txt", "tres"),
        ("4.txt", "cuatro"),
    ] {
        repo.write(path, &lines(word, 2))?;
    }
    let day_3 = repo.commit(3)?;
    for (path, word) in [
        ("5.txt", "five"),
        ("6.txt", "six"),
        ("7.txt", "seven"),
        ("8.txt", "eight"),
    ] {
        repo.write(path, &lines(word, 2))?;
    }
    let day_4 = repo.commit(4)?;

    let window = Window {
        max_files: 3,
        ..every_commit()
    };
    let expected = vec![
        commit(&day_4, 4, None),
        commit(&day_3, 3, None),
        commit(&day_2, 2, Some(vec![changed("4.txt")?])),
        commit(
            &day_1,
            1,
            Some(vec![
                changed("3.txt")?,
                changed("d/2.txt")?,
                changed("d/x/1.txt")?,
            ]),
        ),
    ];
    for (name, mined) in read_both(&repo.root(), &window)? {
        assert_eq!(mined.commits, expected, "{name}");
    }
    Ok(())
}

/// The walk ends at the first commit made before the window's oldest time, even when an older
/// commit's time is newer: committed on days 1, 3 and 2 in that order, with the cut-off at noon on
/// day 2 nothing is read, since the newest commit is older than that, and with it at noon on day 1
/// the commits of days 2 and 3 are.
#[test]
fn the_walk_ends_at_the_first_older_commit() -> TestResult {
    let repo = Repo::new()?;
    commit_day(&repo, 1)?;
    let day_3 = commit_day(&repo, 3)?;
    let day_2 = commit_day(&repo, 2)?;
    let since = |half_days: i64| Window {
        oldest_s: DAY_0_S + half_days * DAY_S / 2,
        ..every_commit()
    };
    for (name, history) in sources(&repo.root())? {
        assert_eq!(history.read(&since(5))?.commits, vec![], "{name}");
        let ids: Vec<String> = history
            .read(&since(3))?
            .commits
            .into_iter()
            .map(|commit| commit.id)
            .collect();
        assert_eq!(ids, [day_2.clone(), day_3.clone()], "{name}");
    }
    Ok(())
}

/// A read stops at the commit it is given, without reading it, and says it got there, also when
/// that commit comes right after the most commits the window takes. An id that is not on the line,
/// or no id at all, is never reached, and the whole window is read.
#[test]
fn a_read_stops_at_the_commit_it_is_given() -> TestResult {
    let repo = Repo::new()?;
    let day_1 = commit_day(&repo, 1)?;
    let day_2 = commit_day(&repo, 2)?;
    let day_3 = commit_day(&repo, 3)?;
    let everything = vec![day_3.clone(), day_2.clone(), day_1.clone()];
    let elsewhere = "0123456789abcdef0123456789abcdef01234567";
    for (name, history) in sources(&repo.root())? {
        let read_to = |stop_at: &str| -> TestResult<(Vec<String>, bool)> {
            let window = Window {
                stop_at: Some(stop_at.to_owned()),
                ..every_commit()
            };
            let mined = history.read(&window)?;
            let ids = mined.commits.into_iter().map(|commit| commit.id).collect();
            Ok((ids, mined.is_stop_reached))
        };
        assert_eq!(
            read_to(&day_1)?,
            (vec![day_3.clone(), day_2.clone()], true),
            "{name}"
        );
        assert_eq!(read_to(&day_3)?, (vec![], true), "{name}");
        let two_then_stop = Window {
            max_commits: 2,
            stop_at: Some(day_1.clone()),
            ..every_commit()
        };
        assert!(history.read(&two_then_stop)?.is_stop_reached, "{name}");
        assert_eq!(read_to(elsewhere)?, (everything.clone(), false), "{name}");
        assert_eq!(read_to("HEAD")?, (everything.clone(), false), "{name}");
    }
    Ok(())
}

/// A stop commit older than the window's oldest time is still reached: it comes first among the
/// older commits, and what was read before still leads up to HEAD. Commits on days 0, 10 and 20,
/// the stop at day 10, the cut-off at day 15: day 20 is read and the stop reached.
#[test]
fn a_stop_commit_older_than_the_window_is_reached() -> TestResult {
    let repo = Repo::new()?;
    commit_day(&repo, 0)?;
    let day_10 = commit_day(&repo, 10)?;
    let day_20 = commit_day(&repo, 20)?;
    let window = Window {
        oldest_s: DAY_0_S + 15 * DAY_S,
        stop_at: Some(day_10),
        ..every_commit()
    };
    for (name, mined) in read_both(&repo.root(), &window)? {
        let ids: Vec<&str> = mined
            .commits
            .iter()
            .map(|commit| commit.id.as_str())
            .collect();
        assert_eq!(ids, [day_20.as_str()], "{name}");
        assert!(mined.is_stop_reached, "{name}");
    }
    Ok(())
}

/// A HEAD that names no commit is an error, not an empty history: one naming a tree, and one
/// naming an object the repository does not hold.
#[test]
fn a_broken_head_is_an_error() -> TestResult {
    let repo = Repo::new()?;
    commit_day(&repo, 1)?;
    let tree = repo.git(&["rev-parse", "HEAD^{tree}"])?;
    let head = repo.root().join(".git").join("HEAD");
    for broken in [tree.as_str(), "0123456789abcdef0123456789abcdef01234567"] {
        std::fs::write(&head, format!("{broken}\n"))?;
        for (name, history) in sources(&repo.root())? {
            assert!(
                matches!(history.read(&every_commit()), Err(GitError::Head(_))),
                "{name}: {broken}"
            );
        }
    }
    Ok(())
}

/// A submodule's change counts as a changed file whatever the repository says of submodules: with
/// `diff.ignoreSubmodules` set to `all`, the commit that adds the submodule `sub` and the one that
/// moves it to another commit each list `sub`.
#[test]
fn a_submodule_change_counts_whatever_the_settings() -> TestResult {
    let repo = Repo::new()?;
    let first = commit_day(&repo, 1)?;
    let second = commit_day(&repo, 2)?;
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{first},sub"),
    ])?;
    repo.git_on(3, &["commit", "-q", "-m", "submodule"])?;
    let added = repo.git(&["rev-parse", "HEAD"])?;
    repo.git(&[
        "update-index",
        "--cacheinfo",
        &format!("160000,{second},sub"),
    ])?;
    repo.git_on(4, &["commit", "-q", "-m", "moved"])?;
    let moved = repo.git(&["rev-parse", "HEAD"])?;
    repo.git(&["config", "diff.ignoreSubmodules", "all"])?;

    for (name, mined) in read_both(&repo.root(), &every_commit())? {
        let newest: Vec<_> = mined.commits.iter().take(2).cloned().collect();
        assert_eq!(
            newest,
            vec![
                commit(&moved, 4, Some(vec![changed("sub")?])),
                commit(&added, 3, Some(vec![changed("sub")?])),
            ],
            "{name}"
        );
    }
    Ok(())
}

/// A shallow clone's cut commit names a parent the clone does not hold: it is left out, rather
/// than counted as adding every file, and the read says the history is shallow.
#[test]
fn a_shallow_clone_leaves_its_cut_commit_out() -> TestResult {
    let origin = Repo::new()?;
    commit_day(&origin, 1)?;
    commit_day(&origin, 2)?;
    let day_3 = commit_day(&origin, 3)?;
    let clone = Repo::clone_of(&origin, &["--depth", "2"])?;

    let expected = Mined {
        commits: vec![commit(&day_3, 3, Some(vec![changed("a.txt")?]))],
        is_stop_reached: false,
        is_shallow: true,
        is_partial: false,
    };
    for (name, mined) in read_both(&clone.root(), &every_commit())? {
        assert_eq!(mined, expected, "{name}");
    }
    Ok(())
}

/// A repository without a commit has no HEAD commit and an empty history.
#[test]
fn a_repository_without_commits_has_no_history() -> TestResult {
    let repo = Repo::new()?;
    for (name, history) in sources(&repo.root())? {
        assert_eq!(history.head()?, None, "{name}");
        assert_eq!(history.read(&every_commit())?, Mined::default(), "{name}");
    }
    Ok(())
}

/// A detached HEAD is read from the commit it points to.
#[test]
fn a_detached_head_reads_from_its_commit() -> TestResult {
    let repo = Repo::new()?;
    let day_1 = commit_day(&repo, 1)?;
    commit_day(&repo, 2)?;
    repo.git(&["checkout", "-q", "--detach", &day_1])?;

    for (name, history) in sources(&repo.root())? {
        assert_eq!(history.head()?, Some(day_1.clone()), "{name}");
        let mined = history.read(&every_commit())?;
        assert_eq!(
            mined.commits,
            vec![commit(&day_1, 1, Some(vec![changed("a.txt")?]))],
            "{name}"
        );
    }
    Ok(())
}

/// The history is read from commits alone: a repository whose index file is damaged reads as
/// before.
#[test]
fn the_index_takes_no_part() -> TestResult {
    let repo = Repo::new()?;
    let day_1 = commit_day(&repo, 1)?;
    std::fs::write(repo.root().join(".git").join("index"), b"not an index")?;

    for (name, mined) in read_both(&repo.root(), &every_commit())? {
        assert_eq!(
            mined.commits,
            vec![commit(&day_1, 1, Some(vec![changed("a.txt")?]))],
            "{name}"
        );
    }
    Ok(())
}

/// Only a work tree's top opens: a folder without git and a folder inside a repository hold no
/// repository of their own, since none is searched for above the folder given. A repository whose
/// configuration names another work tree is refused: its paths are not this folder's.
#[test]
fn only_a_work_tree_s_top_opens() -> TestResult {
    let plain = tempfile::tempdir()?;
    assert!(matches!(
        GixHistory::open(plain.path()),
        Err(GitError::NoRepository(_))
    ));
    assert!(matches!(
        CliHistory::open(plain.path()),
        Err(GitError::NoRepository(_))
    ));

    let repo = Repo::new()?;
    repo.write("inner/a.txt", &lines("inner", 2))?;
    repo.commit(1)?;
    let inner = repo.root().join("inner");
    assert!(matches!(
        GixHistory::open(&inner),
        Err(GitError::NoRepository(_))
    ));
    assert!(matches!(
        CliHistory::open(&inner),
        Err(GitError::NoRepository(_))
    ));

    let elsewhere = repo.home();
    let elsewhere = elsewhere
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    repo.git(&["config", "core.worktree", elsewhere])?;
    assert!(matches!(
        GixHistory::open(&repo.root()),
        Err(GitError::Open(_))
    ));
    assert!(matches!(
        CliHistory::open(&repo.root()),
        Err(GitError::Open(_))
    ));
    Ok(())
}
