//! Linked working trees (`git worktree add`), through the real binary as Claude Code runs it, on
//! the repository the captured payloads come from, made a git repository with one commit on
//! `main` and a linked working tree `wt` beside it on the branch `feature`. A prompt from the
//! linked working tree reaches the main tree's daemon, the one daemon of the repository, and its
//! work order shows the linked working tree's own lines; a fact noted in one working tree is given
//! in the other; a file changed in the linked working tree lends its code to neither the work
//! order there (its lines are not the index's) nor counts as uncommitted in the main tree;
//! `cortex init` there writes its ignore lines to the exclude file git names, and positions and
//! notices name the working tree and branch they come from.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::calc::{OPS, OPS_FIXED, captured, prompt, repository};
use common::git::{commit, git};
use common::hook_call::hook;
use common::init::init;
use common::{Daemon, StopsDaemon, TestResult, graph_of, isolate, printed, wait_for};
use cortex_core::ipc::{RequestBody, ResponseBody, Role, StatusReport};
use cortex_core::{Identifier, RepoId, UserFolder, WorkingTree, canonical_root};
use cortex_hook::ask;
use cortex_store::{PositionRow, Reader};
use serde_json::Value;

/// A prompt in the area of `divide`: it names `divide` and the file it is in.
const FAMILIAR: &str = "Make divide in calc/ops.py round its result to two places.";

/// A session in the main working tree.
const IN_MAIN: &str = "00000000-0000-4000-8000-00000000a001";

/// A session in the linked working tree.
const IN_LINKED: &str = "00000000-0000-4000-8000-00000000b001";

/// Another session in the linked working tree.
const IN_LINKED_AGAIN: &str = "00000000-0000-4000-8000-00000000b002";

/// The captured repository in a home folder, made a git repository with every file committed on
/// `main`, and a linked working tree `wt` beside it on the new branch `feature`: the home folder,
/// the main tree and the linked one.
fn repositories() -> TestResult<(tempfile::TempDir, PathBuf, PathBuf)> {
    let (home, repo) = repository()?;
    git(&repo, home.path(), &["init", "-q", "-b", "main"])?;
    commit(&repo, home.path(), "calc", &["."])?;
    let linked = repo.with_file_name("wt");
    let spelled = linked.to_str().ok_or("a path that is not UTF-8")?;
    git(
        &repo,
        home.path(),
        &["worktree", "add", "-q", "-b", "feature", spelled],
    )?;
    Ok((home, repo, linked))
}

/// [`repositories`], the main tree pinned to medium mode with no task held out, and its daemon,
/// once its graph is built.
fn started() -> TestResult<(tempfile::TempDir, PathBuf, PathBuf, Daemon)> {
    let (home, repo, linked) = repositories()?;
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(
        repo.join(".cortex/config.toml"),
        "mode = \"medium\"\n[holdout]\nshare = 0\n",
    )?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home.path())?.is_some()))?;
    Ok((home, repo, linked, daemon))
}

/// Runs `cortex hook` on `payload` as Claude Code does, and returns what it gave the agent.
fn given(payload: &Value, home: &Path) -> TestResult<Option<String>> {
    let event = payload["hook_event_name"]
        .as_str()
        .ok_or("no event name")?
        .to_owned();
    let call = hook(
        &["--agent", "claude", "--event", &event],
        serde_json::to_vec(payload)?,
        Duration::ZERO,
        home,
    )?;
    call.context(&event)
}

/// `payload` as session `session` sends it.
fn in_session(mut payload: Value, session: &str) -> Value {
    payload["session_id"] = Value::from(session);
    payload
}

/// The captured prompt naming `divide`, from `session` working in the working tree at `root`.
fn familiar(root: &Path, session: &str) -> TestResult<Value> {
    Ok(in_session(prompt(root, FAMILIAR)?, session))
}

/// The lines of code a work order shows, each with its file and its number: under "Current code:",
/// a line `  path:start-end` names the file and each line `  N | code` one of its lines.
fn shown_code(work_order: &str) -> TestResult<Vec<(String, usize, String)>> {
    let Some((_, code)) = work_order.split_once("Current code:\n") else {
        return Ok(Vec::new());
    };
    let mut shown = Vec::new();
    let mut file: Option<String> = None;
    for line in code.lines() {
        let Some(line) = line.strip_prefix("  ") else {
            break;
        };
        if let Some((number, text)) = line.split_once(" | ") {
            if number.trim().is_empty() && text == "..." {
                continue;
            }
            let number: usize = number.trim().parse()?;
            shown.push((
                file.clone().ok_or("code before its file")?,
                number,
                text.to_owned(),
            ));
        } else if let Some((path, _)) = line.rsplit_once(':') {
            file = Some(path.to_owned());
        }
    }
    Ok(shown)
}

/// Checks that every line `work_order` shows is the line of that number in that file of the
/// working tree at `root`, and returns the files it shows code of.
fn true_to(work_order: &str, root: &Path) -> TestResult<Vec<String>> {
    let shown = shown_code(work_order)?;
    for (file, number, text) in &shown {
        let on_disk = std::fs::read_to_string(root.join(file))?;
        let line = on_disk
            .lines()
            .nth(number - 1)
            .ok_or("past the file's end")?;
        assert_eq!(text, line, "{file}:{number} in {work_order}");
    }
    let mut files: Vec<String> = shown.into_iter().map(|(file, _, _)| file).collect();
    files.dedup();
    Ok(files)
}

/// What `cortex` printed with `args`, run in the folder `folder` with no `--root`, as a person in
/// that folder runs it, having succeeded.
fn cortex_in(folder: &Path, home: &Path, args: &[&str]) -> TestResult<String> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).current_dir(folder);
    isolate(&mut command, home);
    let printed = printed(&mut command, b"")?;
    if !printed.is_success {
        return Err(format!("cortex {args:?} failed: {}", printed.stderr).into());
    }
    Ok(printed.stdout)
}

/// The status of the daemon of the repository whose main tree is `repo`.
fn status(repo: &Path, home: &Path) -> TestResult<StatusReport> {
    let files = UserFolder::in_home(home).run_files(&RepoId::of(&canonical_root(repo)?));
    let deadline = Instant::now() + Duration::from_secs(10);
    match ask(&files, Role::Cli, RequestBody::Status, deadline)?.body {
        ResponseBody::Status(report) => Ok(report),
        other => Err(format!("the daemon answered its status with {other:?}").into()),
    }
}

/// The positions the store of the repository whose main tree is `repo` keeps for `session`.
fn positions_of(repo: &Path, session: &str) -> TestResult<Vec<PositionRow>> {
    let session = Identifier::new(session)?;
    let reader = Reader::open(&repo.join(".cortex"), 0)?;
    Ok(reader
        .queries()
        .positions_since(0)?
        .into_iter()
        .filter(|row| row.record.session_id == session)
        .collect())
}

/// Claude Code's captured read, moved to the whole of `calc/ops.py` holding `text` in the working
/// tree at `root`, in `session`.
fn read_of_ops(root: &Path, session: &str, text: &str) -> TestResult<Value> {
    let mut read = in_session(captured("post-tool-use-read.json", root)?, session);
    let file = Value::from(
        root.join("calc")
            .join("ops.py")
            .to_string_lossy()
            .into_owned(),
    );
    read["tool_input"]["file_path"] = file.clone();
    let lines = text.lines().count();
    let result = &mut read["tool_response"]["file"];
    result["filePath"] = file;
    result["content"] = Value::from(text);
    result["numLines"] = Value::from(lines);
    result["totalLines"] = Value::from(lines);
    Ok(read)
}

/// From the linked working tree, a prompt naming `divide` reaches the main tree's daemon, the only
/// daemon of the repository, and gets a work order whose every line shown is the linked working
/// tree's own line on disk; nothing of Cortex's is made in the linked working tree, and its prompt
/// is recorded in the main tree's store. A fact noted from a shell in the linked working tree is
/// given to a prompt in the main tree, and to another in the linked one.
#[test]
fn a_linked_working_tree_shares_its_main_tree_s_daemon_and_facts() -> TestResult {
    let (home, repo, linked, _daemon) = started()?;
    let home = home.path();
    let order = given(&familiar(&linked, IN_LINKED)?, home)?.ok_or("no work order")?;
    assert!(order.starts_with("[cortex] Work order · "), "{order}");
    let files = true_to(&order, &linked)?;
    assert!(files.contains(&"calc/ops.py".to_owned()), "{order}");

    let user = UserFolder::in_home(home);
    let main_id = RepoId::of(&canonical_root(&repo)?);
    let linked_id = RepoId::of(&canonical_root(&linked)?);
    assert!(user.run_files(&main_id).pid_path().exists());
    assert!(!user.run_files(&linked_id).pid_path().exists());
    assert!(!linked.join(".cortex").exists());
    let session = Identifier::new(IN_LINKED)?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        Ok(!reader.queries().tasks_of_session(&session)?.is_empty())
    })?;

    let rule = "divide rounds its result to two places.";
    assert_eq!(
        cortex_in(&linked, home, &["note", rule, "--at", "calc/ops.py:divide"])?,
        "fact 1 added\n"
    );
    let line = format!("Project facts:\n  - {rule} (anchored to calc/ops.py:divide)\n");
    let in_main = given(&familiar(&repo, IN_MAIN)?, home)?.ok_or("no work order")?;
    assert!(in_main.contains(&line), "{in_main}");
    let in_linked = given(&familiar(&linked, IN_LINKED_AGAIN)?, home)?.ok_or("no work order")?;
    assert!(in_linked.contains(&line), "{in_linked}");
    assert!(!user.run_files(&linked_id).pid_path().exists());
    Ok(())
}

/// `divide` changed in the linked working tree (true division, line 13) and the edit sent from
/// there: the linked working tree's look counts `calc/ops.py` uncommitted and the main tree's does
/// not. A prompt there naming `divide` is shown no line of `calc/ops.py`, whose bytes are not the
/// ones the index read; the same prompt in the main tree, whose file is, is shown its lines.
#[test]
fn a_linked_working_tree_s_changed_file_is_its_own() -> TestResult {
    let (home, repo, linked, _daemon) = started()?;
    let home = home.path();
    std::fs::write(linked.join("calc/ops.py"), OPS_FIXED)?;
    given(
        &in_session(captured("post-tool-use-edit.json", &linked)?, IN_LINKED),
        home,
    )?;
    let top = canonical_root(&linked)?;
    wait_for(|| {
        let report = status(&repo, home)?;
        let looked = report
            .linked_worktrees
            .iter()
            .find(|linked| linked.root == top)
            .and_then(|linked| linked.worktree.as_ref());
        let is_main_looked_at = report.worktree.is_some();
        Ok(is_main_looked_at && looked.is_some_and(|looked| looked.uncommitted_files == 1))
    })?;
    let main_tree = status(&repo, home)?
        .worktree
        .ok_or("no look at the main tree")?;
    assert_eq!(main_tree.uncommitted_files, 0, "{main_tree:?}");
    assert!(cortex_in(&repo, home, &["status"])?.contains(&format!(
        "linked work tree {}: 1 uncommitted file kept to seed retrievals",
        top.display()
    )));

    let in_linked = given(&familiar(&linked, IN_LINKED_AGAIN)?, home)?;
    let shown_there = match &in_linked {
        Some(order) => true_to(order, &linked)?,
        None => Vec::new(),
    };
    assert!(
        !shown_there.contains(&"calc/ops.py".to_owned()),
        "{in_linked:?}"
    );
    let in_main = given(&familiar(&repo, IN_MAIN)?, home)?.ok_or("no work order")?;
    assert!(
        true_to(&in_main, &repo)?.contains(&"calc/ops.py".to_owned()),
        "{in_main}"
    );

    // `cortex where`, asked in each working tree, reads that working tree's files alike.
    let located_there = cortex_in(&linked, home, &["where", FAMILIAR])?;
    assert!(
        !true_to(&located_there, &linked)?.contains(&"calc/ops.py".to_owned()),
        "{located_there}"
    );
    let located_here = cortex_in(&repo, home, &["where", FAMILIAR])?;
    assert!(
        true_to(&located_here, &repo)?.contains(&"calc/ops.py".to_owned()),
        "{located_here}"
    );
    assert_eq!(std::fs::read_to_string(repo.join("calc/ops.py"))?, OPS);
    Ok(())
}

/// `cortex init` run first in the linked working tree, before the main tree is set up, sets up
/// the main tree (its `.cortex/config.toml`) and makes no `.cortex` in the linked working tree, so
/// that a prompt there still goes to the main tree's daemon, the only one. `cortex uninstall
/// --purge` run there gives back, byte for byte, the linked working tree's Claude Code settings
/// (which held the user's own model) and the exclude file, and deletes the main tree's `.cortex`.
#[test]
fn init_first_in_a_linked_working_tree_and_uninstall_from_it() -> TestResult {
    let (home, repo, linked) = repositories()?;
    let home = home.path();
    let settings = linked.join(".claude").join("settings.local.json");
    std::fs::create_dir_all(linked.join(".claude"))?;
    let users = br#"{"model": "opus"}"#;
    std::fs::write(&settings, users)?;
    let exclude = linked.join(git_answer(
        &linked,
        home,
        &["rev-parse", "--git-path", "info/exclude"],
    )?);
    let excluded_before = std::fs::read(&exclude)?;
    let _daemon = StopsDaemon::of(&repo, home);
    let linked_init = init(&linked, home, &[])?;
    assert!(linked_init.is_success, "{}", linked_init.stderr);
    assert!(!linked.join(".cortex").exists());
    assert!(repo.join(".cortex").join("config.toml").is_file());
    assert_ne!(std::fs::read(&settings)?, users);
    assert_ne!(std::fs::read(&exclude)?, excluded_before);

    given(&familiar(&linked, IN_LINKED)?, home)?;
    let session = Identifier::new(IN_LINKED)?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        Ok(!reader.queries().tasks_of_session(&session)?.is_empty())
    })?;
    let user = UserFolder::in_home(home);
    assert!(
        !user
            .run_files(&RepoId::of(&canonical_root(&linked)?))
            .pid_path()
            .exists()
    );

    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["uninstall", "--purge"]).current_dir(&linked);
    let uninstalled = common::init::run(&mut command, home, &[])?;
    assert!(uninstalled.is_success, "{}", uninstalled.stderr);
    assert_eq!(std::fs::read(&settings)?, users);
    assert_eq!(std::fs::read(&exclude)?, excluded_before);
    assert!(!repo.join(".cortex").exists());
    assert!(!linked.join(".cortex").exists());
    Ok(())
}

/// `cortex init` in the main tree, then in the linked working tree: the second names the main tree
/// as the repository and the linked working tree as one of it, writes Claude Code's hooks in the
/// linked working tree, makes no `.cortex` there, and adds its ignore lines to the exclude file
/// `git rev-parse --git-path info/exclude` names, never a `.gitignore`: git finds nothing
/// uncommitted there. A session reading `calc/ops.py` in the main tree and one in the linked
/// working tree each keep a position naming its working tree and branch; the linked session's edit
/// of the file is told that the other, in the main working tree on `main`, read it, and the other
/// is sent that the linked session, in worktree `wt` on `feature`, edited it.
#[test]
fn init_and_positions_in_a_linked_working_tree_name_where_they_are() -> TestResult {
    let (home, repo, linked) = repositories()?;
    let home = home.path();
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), "mode = \"medium\"\n")?;
    let _daemon = StopsDaemon::of(&repo, home);
    let main_init = init(&repo, home, &[])?;
    assert!(main_init.is_success, "{}", main_init.stderr);
    let linked_init = init(&linked, home, &[])?;
    assert!(linked_init.is_success, "{}", linked_init.stderr);
    let (main, top) = (canonical_root(&repo)?, canonical_root(&linked)?);
    assert!(
        linked_init
            .stdout
            .contains(&format!("repository: {}\n", main.display())),
        "{}",
        linked_init.stdout
    );
    assert!(
        linked_init.stdout.contains(&format!(
            "note: {} is a linked working tree of this repository",
            top.display()
        )),
        "{}",
        linked_init.stdout
    );
    assert!(linked.join(".claude/settings.local.json").is_file());
    assert!(!linked.join(".cortex").exists());
    assert!(!linked.join(".gitignore").exists());
    let exclude = git_answer(&linked, home, &["rev-parse", "--git-path", "info/exclude"])?;
    let excluded = std::fs::read_to_string(linked.join(exclude))?;
    assert!(
        excluded.ends_with("# Cortex\n.cortex/\n.claude/settings.local.json\n"),
        "{excluded}"
    );
    assert_eq!(git_answer(&linked, home, &["status", "--porcelain"])?, "");

    for (root, session) in [(&repo, IN_MAIN), (&linked, IN_LINKED)] {
        given(&in_session(prompt(root, "Look at divide.")?, session), home)?;
        given(&read_of_ops(root, session, OPS)?, home)?;
    }
    wait_for(|| {
        Ok(
            !positions_of(&repo, IN_MAIN)?.is_empty()
                && !positions_of(&repo, IN_LINKED)?.is_empty(),
        )
    })?;
    let place = |session: &str| -> TestResult<Vec<(Option<String>, Option<String>)>> {
        Ok(positions_of(&repo, session)?
            .into_iter()
            .map(|row| {
                (
                    row.record.worktree.map(|tree| match tree {
                        WorkingTree::Main => "main".to_owned(),
                        WorkingTree::Linked(name) => {
                            format!("linked:{}", name.as_ref().map_or("", Identifier::as_str))
                        }
                    }),
                    row.record.branch.map(|name| name.as_str().to_owned()),
                )
            })
            .collect())
    };
    let main = Some("main".to_owned());
    assert_eq!(place(IN_MAIN)?, [(main.clone(), main)]);
    assert_eq!(
        place(IN_LINKED)?,
        [(Some("linked:wt".to_owned()), Some("feature".to_owned()))]
    );

    std::fs::write(linked.join("calc/ops.py"), OPS_FIXED)?;
    let edited = given(
        &in_session(captured("post-tool-use-edit.json", &linked)?, IN_LINKED),
        home,
    )?;
    let edited = match edited {
        Some(edited) => edited,
        None => {
            let errors = status(&repo, home)?.recent_errors;
            return Err(
                format!("the edit was told nothing; the daemon's errors: {errors:?}").into(),
            );
        }
    };
    assert!(
        edited.contains(
            "Another Claude Code session in the main working tree on branch main has read \
             calc/ops.py, the file you just edited."
        ),
        "{edited}"
    );
    let session = Identifier::new(IN_MAIN)?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        Ok(reader
            .queries()
            .notices_waiting(&session, 0)?
            .iter()
            .any(|notice| {
                notice.text.contains(
                    "Another Claude Code session in worktree wt on branch feature edited \
                     calc/ops.py",
                )
            }))
    })?;
    Ok(())
}

/// What git printed with `args` in `folder`, `home` being the home folder, without its last line
/// break.
fn git_answer(folder: &Path, home: &Path, args: &[&str]) -> TestResult<String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(folder);
    isolate(&mut command, home);
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!("git {args:?} failed: {output:?}").into());
    }
    Ok(String::from_utf8(output.stdout)?
        .trim_end_matches(['\n', '\r'])
        .to_owned())
}
