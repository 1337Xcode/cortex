//! The keep-list a compaction's summary is given, from a store built here: the same working set the
//! recap after it gives (the files the open task edited, the files it was warned of and has not
//! read or edited since, its prompt), under a header asking the summary to keep it. Nothing without
//! an open task or anything to keep, and never past its cap of 600 characters, counted as Claude
//! Code counts them: lines past it go from the end, and the edited files' line names fewer files.
//! Every expected line and every length is worked out by hand.

mod common;

use common::task::{SESSION, acted, file, task, warned};
use common::{Repo, TestResult};
use cortex_core::{Identifier, ToolKind, WarningKind};
use cortex_retrieve::{MAX_KEEP_LIST_CHARS, keep_list};

/// The first line of every keep-list: 56 characters, its line break included.
const HEADER: &str = "[cortex] Keep in the summary that the task in progress:\n";

/// The keep-list the session's compaction gets.
fn kept(repo: &Repo) -> TestResult<Option<String>> {
    let reader = repo.reader()?;
    let queries = reader.queries();
    let Some(task) = queries.open_task(&Identifier::new(SESSION)?)? else {
        return Ok(None);
    };
    Ok(keep_list(queries, &task, (repo.root.path(), 1_048_576))?)
}

/// How long `text` is as Claude Code counts it: in UTF-16 code units.
fn length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A task that edited `main.py` at 8 and 20 and `calc/ops.py` at 10 and 30 keeps them, the latest
/// edited first; of its three warnings at 10, the one on `main.py` was followed by its edit at 20,
/// so the two others are kept, the latest first; then its prompt, on one line. 321 characters.
#[test]
fn a_task_s_working_set_is_kept() -> TestResult {
    let mut repo = Repo::new()?;
    let (ops, add) = file(&mut repo, "calc/ops.py")?;
    let (main, _) = file(&mut repo, "main.py")?;
    let (tests, _) = file(&mut repo, "tests/test_ops.py")?;
    let (docs, _) = file(&mut repo, "docs/ops.md")?;
    let task = task(&mut repo, Some("Let add\ntake a third number."))?;
    let edit = |path, node| (ToolKind::Edit, path, node);
    acted(&mut repo, task, edit("main.py", main), (8, true))?;
    acted(&mut repo, task, edit("calc/ops.py", ops), (10, true))?;
    warned(&mut repo, task, (WarningKind::CallSite, add, main), 10)?;
    warned(&mut repo, task, (WarningKind::Test, ops, tests), 10)?;
    warned(&mut repo, task, (WarningKind::CoChange, ops, docs), 10)?;
    acted(&mut repo, task, edit("main.py", main), (20, true))?;
    acted(&mut repo, task, edit("calc/ops.py", ops), (30, true))?;
    let expected = "\
[cortex] Keep in the summary that the task in progress:
- edited calc/ops.py, main.py
- was warned of docs/ops.md, which usually changes with calc/ops.py; not read or edited since
- was warned of tests/test_ops.py, the tests of calc/ops.py; not read or edited since
- began with the prompt \"Let add take a third number.\"
";
    assert_eq!(kept(&repo)?.as_deref(), Some(expected));
    assert_eq!(length(expected), 321);
    Ok(())
}

/// A task that edited nothing, was warned of nothing and kept no prompt has no keep-list, and
/// neither has one whose only edit failed: a failed edit is not an edit. (Before the task opens
/// there is no task to ask for one; the daemon gives nothing then, as the CLI's test shows.)
#[test]
fn nothing_to_keep_gives_no_keep_list() -> TestResult {
    let mut repo = Repo::new()?;
    let (ops, _) = file(&mut repo, "calc/ops.py")?;
    assert_eq!(kept(&repo)?, None);
    let task = task(&mut repo, None)?;
    assert_eq!(kept(&repo)?, None);
    acted(
        &mut repo,
        task,
        (ToolKind::Edit, "calc/ops.py", ops),
        (10, false),
    )?;
    assert_eq!(kept(&repo)?, None);
    Ok(())
}

/// Past the cap, lines go from the end. By hand: the header is 56 characters and the edited line
/// 21 (`- edited calc/ops.py` and its line break), 77 together. A warning of a test file whose path
/// is 100 characters is 16 + 100 + 15 + 11 + 26 + 1 = 169: three make 77 + 507 = 584, four would
/// make 753. So the three latest of the four warnings are kept, and the prompt is not.
#[test]
fn a_keep_list_past_its_cap_drops_its_last_lines() -> TestResult {
    let mut repo = Repo::new()?;
    let (ops, _) = file(&mut repo, "calc/ops.py")?;
    let task = task(&mut repo, Some("Let add take a third number."))?;
    acted(
        &mut repo,
        task,
        (ToolKind::Edit, "calc/ops.py", ops),
        (10, true),
    )?;
    let mut paths = Vec::new();
    for at in 0..4_u8 {
        // 50 + 1 + 1 + 45 + 3 = 100 characters.
        let path = format!(
            "{}/{}{}.py",
            "a".repeat(50),
            char::from(b'k' + at),
            "t".repeat(45)
        );
        let (test, _) = file(&mut repo, &path)?;
        warned(&mut repo, task, (WarningKind::Test, ops, test), 10)?;
        paths.push(path);
    }
    let warning = |path: &String| {
        format!("- was warned of {path}, the tests of calc/ops.py; not read or edited since\n")
    };
    let expected = format!(
        "{HEADER}- edited calc/ops.py\n{}{}{}",
        warning(&paths[3]),
        warning(&paths[2]),
        warning(&paths[1])
    );
    assert_eq!(kept(&repo)?.as_deref(), Some(expected.as_str()));
    assert_eq!(length(&expected), 584);
    Ok(())
}

/// The edited files' line names as many files as fit beside the header, and how many more there
/// are. By hand: twelve files of 60 characters each, edited one after the other; naming n of them
/// with the 11 characters of ` and k more` takes 56 + 9 + 60n + 2(n - 1) + 11 + 1 characters: 571
/// for eight, 633 for nine. So the eight latest are named and 4 more counted.
#[test]
fn the_edited_files_line_names_what_fits() -> TestResult {
    let mut repo = Repo::new()?;
    let task = task(&mut repo, None)?;
    let mut paths = Vec::new();
    for at in 0..12_i64 {
        // 55 + 2 + 3 = 60 characters.
        let path = format!("{}{at:02}.py", "p".repeat(55));
        let (node, _) = file(&mut repo, &path)?;
        acted(
            &mut repo,
            task,
            (ToolKind::Edit, &path, node),
            (10 + at, true),
        )?;
        paths.push(path);
    }
    let named: Vec<&str> = paths.iter().rev().take(8).map(String::as_str).collect();
    let expected = format!("{HEADER}- edited {} and 4 more\n", named.join(", "));
    assert_eq!(kept(&repo)?.as_deref(), Some(expected.as_str()));
    assert_eq!(length(&expected), 571);
    Ok(())
}

/// The cap counts as Claude Code counts: a character past the basic plane (U+1D11E) is two. By
/// hand: the header and `- edited <path>` with its line break are 56 + 10 + p; the prompt of 200
/// such characters is 25 + 400 + 2 = 427. With a path of 107 characters that is 600, kept; of 108,
/// 601, and the prompt goes, though in characters the whole would be 401.
#[test]
fn the_cap_counts_as_claude_code_counts() -> TestResult {
    for (letters, is_prompt_kept) in [(53, true), (54, false)] {
        let mut repo = Repo::new()?;
        let task = task(&mut repo, Some(&"\u{1d11e}".repeat(200)))?;
        // 50 + 1 + letters + 3 characters.
        let path = format!("{}/{}.py", "a".repeat(50), "b".repeat(letters));
        let (node, _) = file(&mut repo, &path)?;
        acted(&mut repo, task, (ToolKind::Edit, &path, node), (10, true))?;
        let text = kept(&repo)?.ok_or("a keep-list")?;
        assert_eq!(
            text.contains("- began with the prompt"),
            is_prompt_kept,
            "{letters}"
        );
        assert!(length(&text) <= MAX_KEEP_LIST_CHARS, "{letters}");
        if is_prompt_kept {
            assert_eq!(length(&text), 600);
        }
    }
    Ok(())
}

/// Whatever a task holds, its keep-list stays within the cap, opens with the header, is made of
/// whole lines and keeps them in order: the prompt, the last line, only when every warning is kept,
/// and the warnings kept are the latest. Each case is a number of warnings, the length of their
/// paths and whether the task kept its prompt.
#[test]
fn every_keep_list_stays_within_its_cap() -> TestResult {
    for warnings in [1_u8, 3, 6] {
        for path_chars in [20, 90, 200] {
            for prompt in [None, Some("x".repeat(200))] {
                let case = format!("{warnings} warnings of {path_chars}, prompt {prompt:?}");
                let mut repo = Repo::new()?;
                let (ops, _) = file(&mut repo, "calc/ops.py")?;
                let task = task(&mut repo, prompt.as_deref())?;
                acted(
                    &mut repo,
                    task,
                    (ToolKind::Edit, "calc/ops.py", ops),
                    (10, true),
                )?;
                let mut paths = Vec::new();
                for at in 0..warnings {
                    let path =
                        format!("{}{}.py", char::from(b'k' + at), "t".repeat(path_chars - 4));
                    let (test, _) = file(&mut repo, &path)?;
                    warned(&mut repo, task, (WarningKind::Test, ops, test), 10)?;
                    paths.push(path);
                }
                let text = kept(&repo)?.ok_or_else(|| format!("{case}: a keep-list"))?;
                assert!(length(&text) <= MAX_KEEP_LIST_CHARS, "{case}");
                assert!(text.starts_with(HEADER), "{case}");
                assert!(text.ends_with('\n'), "{case}");
                assert!(
                    text.lines().skip(1).all(|line| line.starts_with("- ")),
                    "{case}"
                );
                let named: Vec<bool> = paths
                    .iter()
                    .map(|path| text.contains(path.as_str()))
                    .collect();
                // The latest warnings come first, so the ones named are the last paths here.
                let first_named = named.iter().position(|is_named| *is_named);
                assert!(
                    first_named.is_none_or(|first| named[first..].iter().all(|is_named| *is_named)),
                    "{case}"
                );
                if text.contains("- began with the prompt") {
                    assert!(named.iter().all(|is_named| *is_named), "{case}");
                }
            }
        }
    }
    Ok(())
}
