//! Adopt on first use, through the real binary: after `cortex init --global`, a hook from a git
//! repository not set up yet starts a daemon that sets it up (its `.cortex/` and the lines in the
//! exclude file git keeps, never a `.gitignore`), tells the user once and never the model, and
//! serves it. The home folder, the temporary folder, the user's own Cortex folder, a folder
//! outside git, a folder `adopt.exclude` names, a folder below `off` and a folder that is not its
//! working tree's top are never set up and keep nothing of Cortex's, whether a hook offers them or
//! the daemon is started for one directly; a probe offers nothing; a set-up that fails leaves the
//! folder as it was and is not remembered; and a second start sets nothing up.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::fs::TryLockError;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::adopting::{
    FAMILIAR, Machine, calc_repository, git_status, given, in_session, names_in, wait_within,
    welcomes_only_the_user,
};
use common::calc::{captured, prompt};
use common::hook_call::Call;
use common::payloads::payload_of;
use common::{READY_WITHIN, StopsDaemon, TestResult, graph_of, wait_for};
use cortex_core::{RepoId, canonical_root, open_private_file};
use cortex_daemon::is_running;
use serde_json::Value;

/// The Codex version whose captured payloads these tests send.
const CODEX: &str = "0.165.0";

/// A session's start from `folder`, as Claude Code sends it.
fn start_from(folder: &Path) -> TestResult<Value> {
    captured("session-start-startup.json", folder)
}

/// After `init --global`, the first hook from a new git repository below home gives nothing and
/// starts a daemon that sets the repository up: `.cortex/` appears and the exclude file git keeps
/// gets Cortex's lines after what it held, no `.gitignore` is made, and git shows nothing new. Once
/// the graph is built, the first prompt tells the user, in the message for the user alone, that
/// Cortex set the repository up; the next prompt gets a work order, and no such line again.
/// `cortex uninstall` there then gives the exclude file back as it was.
#[test]
fn a_repository_an_agent_opens_after_init_global_is_adopted() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    machine.adopting("mode = \"medium\"\n[holdout]\nshare = 0\n")?;
    let repo = home.join("code").join("calc");
    calc_repository(&repo, home)?;
    let _daemon = StopsDaemon::of(&repo, home);
    let exclude = repo.join(".git").join("info").join("exclude");
    let before = std::fs::read_to_string(&exclude)?;

    let start = machine.hook(&start_from(&repo)?)?;
    assert!(start.is_silent_success(), "{:?}", start.output);
    wait_within(READY_WITHIN, || {
        Ok(repo.join(".cortex").is_dir() && graph_of(&repo, home).is_ok_and(|g| g.is_some()))
    })?;
    assert_eq!(
        std::fs::read_to_string(&exclude)?,
        format!("{before}# Cortex\n.cortex/\n")
    );
    assert!(!repo.join(".gitignore").exists());
    assert_eq!(git_status(&repo, home)?, "");
    assert!(!machine.refusal(&repo)?.exists());

    let first = given(&machine.hook(&in_session(prompt(&repo, FAMILIAR)?, "s-first"))?)?
        .ok_or("the first prompt gave nothing")?;
    welcomes_only_the_user(&first)?;
    let second = given(&machine.hook(&in_session(prompt(&repo, FAMILIAR)?, "s-second"))?)?
        .ok_or("the second prompt gave nothing")?;
    assert!(second.get("systemMessage").is_none(), "{second}");
    let order = second["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .ok_or("no work order")?;
    assert!(order.starts_with("[cortex] Work order · "), "{order}");
    assert!(order.contains("calc/ops.py"), "{order}");

    let root = repo.to_string_lossy().into_owned();
    let undone = machine.cortex(&["uninstall", "--root", &root])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(std::fs::read_to_string(&exclude)?, before);
    Ok(())
}

/// Codex shows its user a hook's message too: a repository Codex opens is set up, and Codex's
/// first prompt there tells the user, and nothing Codex's model reads holds the line.
#[test]
fn codex_s_user_is_told_and_its_model_is_not() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    machine.adopting("[holdout]\nshare = 0\n")?;
    let repo = home.join("code").join("calc");
    calc_repository(&repo, home)?;
    let _daemon = StopsDaemon::of(&repo, home);
    let codex = |name: &str| -> TestResult<Call> {
        let payload: Value = serde_json::from_slice(&payload_of("codex", CODEX, name, &repo)?)?;
        machine.hook_of("codex", &payload, &machine.temp)
    };
    assert!(codex("session-start-startup.json")?.is_silent_success());
    wait_within(READY_WITHIN, || {
        Ok(repo.join(".cortex").is_dir() && graph_of(&repo, home).is_ok_and(|g| g.is_some()))
    })?;
    let first = given(&codex("user-prompt-submit.json")?)?.ok_or("the prompt gave nothing")?;
    welcomes_only_the_user(&first)?;
    Ok(())
}

/// None of these is ever offered by a hook, and none keeps anything of Cortex's: the home folder
/// (with the temporary folder inside it, as on Windows, and outside it, as elsewhere, so that the
/// home folder's own rule is the one that refuses it), a git repository in the temporary folder,
/// one in the user's own Cortex folder, one whose `.cortex` holds `off`, and a folder in no git
/// working tree; the one in the temporary folder carries a `.cortex` of its own, which serves
/// nothing, since no record of this machine says it is set up. A probe, `cortex doctor`'s, offers nothing either. A git repository
/// `adopt.exclude` names is refused by the daemon started for it, which remembers the refusal, so
/// that the hook starts nothing for it again, and `cortex status` and `cortex doctor` name it.
#[test]
fn excluded_folders_are_never_adopted() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    machine.adopting("[adopt]\nexclude = [\"~/skip\"]\n")?;
    let in_temp = machine.temp.join("clone");
    let in_user_folder = machine.user().path().join("clone");
    let turned_off = home.join("off");
    let probed = home.join("probed");
    for folder in [&in_temp, &in_user_folder, &turned_off, &probed] {
        calc_repository(folder, home)?;
    }
    std::fs::create_dir_all(turned_off.join(".cortex"))?;
    std::fs::write(turned_off.join(".cortex").join(cortex_core::OFF_FILE), "")?;
    // A clone carrying a `.cortex` of its own, as an archive could: no record of this machine's
    // says it is set up, so no hook serves it.
    std::fs::create_dir_all(in_temp.join(".cortex"))?;
    std::fs::write(
        in_temp.join(".cortex").join("config.toml"),
        "mode = \"tiny\"\n",
    )?;
    let loose = home.join("notes");
    std::fs::create_dir_all(&loose)?;
    std::fs::write(loose.join("todo.md"), "- write the tests\n")?;
    for folder in [home, &in_temp, &in_user_folder, &turned_off, &loose] {
        let call = machine.hook(&start_from(folder)?)?;
        assert!(call.is_silent_success(), "{:?}", call.output);
        assert!(
            !machine.start_marker(folder)?.exists(),
            "{}",
            folder.display()
        );
        assert!(!machine.refusal(folder)?.exists(), "{}", folder.display());
    }
    let mut probe = start_from(&probed)?;
    probe["cortex_probe"] = Value::Bool(true);
    assert!(machine.hook(&probe)?.is_silent_success());
    assert!(!machine.start_marker(&probed)?.exists());
    assert_eq!(names_in(&in_temp.join(".cortex"))?, ["config.toml"]);
    assert!(!probed.join(".cortex").exists());
    assert_eq!(names_in(&loose)?, ["todo.md"]);
    let temp_outside = machine.base.path().join("tmp");
    std::fs::create_dir_all(&temp_outside)?;
    let call = machine.hook_of("claude", &start_from(home)?, &temp_outside)?;
    assert!(call.is_silent_success(), "{:?}", call.output);
    assert!(!machine.start_marker(home)?.exists());

    let skipped = home.join("skip").join("calc");
    calc_repository(&skipped, home)?;
    let exclude = skipped.join(".git").join("info").join("exclude");
    let before = std::fs::read(&exclude)?;
    let call = machine.hook(&start_from(&skipped)?)?;
    assert!(call.is_silent_success(), "{:?}", call.output);
    let refusal = machine.refusal(&skipped)?;
    wait_for(|| Ok(refusal.exists()))?;
    let why = "`adopt.exclude` holds `~/skip`";
    let root = canonical_root(&skipped)?;
    assert_eq!(
        std::fs::read_to_string(&refusal)?,
        format!("{}\n{why}\n", root.display())
    );
    assert!(!skipped.join(".cortex").exists());
    assert!(!is_running(&machine.user(), &RepoId::of(&root))?);
    // Long past the gap between two starts, a hook would start one again: it does not.
    let long_ago = machine.age_start_marker(&skipped)?;
    let again = machine.hook(&start_from(&skipped)?)?;
    assert!(again.is_silent_success(), "{:?}", again.output);
    assert_eq!(
        std::fs::metadata(machine.start_marker(&skipped)?)?.modified()?,
        long_ago
    );
    assert_eq!(std::fs::read(&exclude)?, before);

    let shown = root.to_string_lossy().into_owned();
    let status = machine.cortex(&["status", "--root", &shown])?;
    assert!(status.is_success, "{}", status.stderr);
    let refused_line = format!("adoption: refused {}: {why}", root.display());
    assert!(
        status
            .stdout
            .contains("adoption: on; 1 folder refused by its rules, the newest 1 below"),
        "{}",
        status.stdout
    );
    assert!(status.stdout.contains(&refused_line), "{}", status.stdout);
    let doctor = machine.cortex(&["doctor", "--root", &shown])?;
    assert!(
        doctor.stdout.contains(&format!(
            "FAIL  adoption: Cortex refused to set this repository up on its own: {why}"
        )),
        "{}",
        doctor.stdout
    );
    assert!(!skipped.join(".cortex").exists());
    Ok(())
}

/// Started for a folder directly, as a hook would start it, the daemon checks each rule itself and
/// refuses: the home folder, a folder outside it, the temporary folder, the user's own Cortex
/// folder, a folder below `off`, a folder `adopt.exclude` names, a folder that is not its git
/// working tree's top, and a folder in no git working tree. Each refusal is remembered with why,
/// nothing is written in the folder, and no daemon runs.
#[test]
fn the_daemon_refuses_each_excluded_folder_itself() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    machine.adopting("[adopt]\nexclude = [\"~/skip\"]\n")?;
    let outside = machine.base.path().join("elsewhere");
    let in_temp = machine.temp.join("clone");
    let in_user_folder = machine.user().path().join("clone");
    let below_off = home.join("outer").join("calc");
    let skipped = home.join("skip").join("calc");
    let repo = home.join("code").join("calc");
    for folder in [
        &outside,
        &in_temp,
        &in_user_folder,
        &below_off,
        &skipped,
        &repo,
    ] {
        calc_repository(folder, home)?;
    }
    std::fs::create_dir_all(home.join("outer").join(".cortex"))?;
    std::fs::write(
        home.join("outer")
            .join(".cortex")
            .join(cortex_core::OFF_FILE),
        "",
    )?;
    let loose = home.join("notes");
    std::fs::create_dir_all(&loose)?;
    let cases: [(&Path, &str); 8] = [
        (home, "it is the home folder or outside it"),
        (&outside, "it is the home folder or outside it"),
        (
            &in_temp,
            "it is the temporary folder, inside it, or holds it",
        ),
        (&in_user_folder, "it is in the user's own Cortex folder"),
        (&below_off, "`cortex off` turned Cortex off there"),
        (&skipped, "`adopt.exclude` holds `~/skip`"),
        (
            &repo.join("calc"),
            "it is not the top of the git working tree it is in",
        ),
        (
            &loose,
            "it is in no git working tree; `cortex init` sets such a folder up",
        ),
    ];
    for (folder, why) in cases {
        let before = names_in(folder)?;
        machine.adopt_setting_nothing_up(folder)?;
        let root = canonical_root(folder)?;
        assert_eq!(
            std::fs::read_to_string(machine.refusal(folder)?)?,
            format!("{}\n{why}\n", root.display()),
            "{}",
            folder.display()
        );
        assert_eq!(names_in(folder)?, before, "{}", folder.display());
        assert!(!is_running(&machine.user(), &RepoId::of(&root))?);
    }
    assert!(!machine.failures_log().exists());
    Ok(())
}

/// A set-up that fails once writing began (here a file stands where git keeps its exclude file's
/// folder, which init would otherwise make) gives back what it wrote: no `.cortex`, no backup, nothing in the user's backups; it is
/// logged and not remembered, and no daemon runs. A configuration that cannot be read, and git
/// missing from the PATH, fail too, logged and not remembered; once the configuration is fixed, the
/// next hook sets the folder up.
#[test]
fn a_failed_set_up_leaves_the_folder_as_it_was() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    machine.adopting("[holdout]\nshare = 0\n")?;
    let user = machine.user();
    let backups_before = names_in(&user.path().join("backups"))?;
    let repo = home.join("code").join("calc");
    calc_repository(&repo, home)?;
    let _daemon = StopsDaemon::of(&repo, home);
    let info = repo.join(".git").join("info");
    std::fs::remove_dir_all(&info)?;
    std::fs::write(&info, "not a folder\n")?;
    machine.adopt_setting_nothing_up(&repo)?;
    assert!(!repo.join(".cortex").exists());
    assert_eq!(std::fs::read_to_string(&info)?, "not a folder\n");
    assert_eq!(names_in(&user.path().join("backups"))?, backups_before);
    assert!(!machine.refusal(&repo)?.exists());
    let log = std::fs::read_to_string(machine.failures_log())?;
    assert!(
        log.contains(&format!("{}: Cortex could not set it up:", repo.display())),
        "{log}"
    );
    assert!(!is_running(&user, &RepoId::of(&repo))?);
    std::fs::remove_file(&info)?;
    std::fs::create_dir_all(&info)?;

    let root = repo.to_string_lossy().into_owned();
    let no_git = machine.cortex_with(&["daemon", "--root", &root, "--adopt"], &[])?;
    assert!(no_git.is_success, "{}", no_git.stderr);
    assert!(!repo.join(".cortex").exists());
    assert!(!machine.refusal(&repo)?.exists());
    let log = std::fs::read_to_string(machine.failures_log())?;
    assert!(log.contains("git is not on the PATH"), "{log}");

    std::fs::write(user.config_path(), "[adopt]\nexclude = 3\n")?;
    let call = machine.hook(&start_from(&repo)?)?;
    assert!(call.is_silent_success(), "{:?}", call.output);
    wait_for(|| {
        Ok(std::fs::read_to_string(machine.failures_log())?
            .contains("its configuration cannot be read"))
    })?;
    assert!(!repo.join(".cortex").exists());
    assert!(!machine.refusal(&repo)?.exists());
    std::fs::write(user.config_path(), "[holdout]\nshare = 0\n")?;
    machine.age_start_marker(&repo)?;
    let call = machine.hook(&start_from(&repo)?)?;
    assert!(call.is_silent_success(), "{:?}", call.output);
    wait_within(READY_WITHIN, || {
        Ok(repo.join(".cortex").is_dir() && graph_of(&repo, home).is_ok_and(|g| g.is_some()))
    })?;
    Ok(())
}

/// While another start holds the folder's set-up, a start sets nothing up and records nothing:
/// no `.cortex`, no refusal, no failure, no daemon.
#[test]
fn a_second_start_sets_nothing_up() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    machine.adopting("[holdout]\nshare = 0\n")?;
    let repo = home.join("code").join("calc");
    calc_repository(&repo, home)?;
    let user = machine.user();
    let id = RepoId::of(&canonical_root(&repo)?);
    let lock = user.adopting_path(&id);
    std::fs::create_dir_all(user.run_folder())?;
    let held = open_private_file(&lock)?;
    match held.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err("the lock is taken already".into()),
        Err(TryLockError::Error(error)) => return Err(error.into()),
    }
    machine.adopt_setting_nothing_up(&repo)?;
    assert!(!repo.join(".cortex").exists());
    assert!(!machine.refusal(&repo)?.exists());
    assert!(!machine.failures_log().exists());
    assert!(!is_running(&user, &id)?);
    drop(held);
    Ok(())
}

/// The hook's cost, from its start to its exit, at a prompt from a git repository not set up, in
/// three cases: adoption off; adoption on and the folder refused before; adoption on and the
/// folder offered for the first time, which starts the daemon that refuses it. 50 runs each, the
/// median and the 95th percentile printed, each against the prompt's 150 ms budget. Timing under
/// load: run alone, on a release build (`cargo test --release`).
#[test]
#[ignore = "timing: run alone on a release build"]
#[allow(
    clippy::print_stdout,
    reason = "the times measured are what this test is run for"
)]
fn the_hook_s_cost_with_adoption() -> TestResult {
    const RUNS: usize = 50;
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    let repo = home.join("code").join("calc");
    calc_repository(&repo, home)?;
    let payload = prompt(&repo, FAMILIAR)?;
    let off = timed(RUNS, |_| machine.hook(&payload))?;

    machine.adopting("[adopt]\nexclude = [\"~/skip\"]\n")?;
    let skipped: Vec<PathBuf> = (0..RUNS)
        .map(|at| home.join("skip").join(format!("r{at:02}")))
        .collect();
    for folder in &skipped {
        std::fs::create_dir_all(folder.join(".git"))?;
    }
    let first = timed(RUNS, |at| {
        let folder = skipped.get(at).ok_or("no folder")?;
        machine.hook(&prompt(folder, FAMILIAR)?)
    })?;
    for folder in &skipped {
        let refusal = machine.refusal(folder)?;
        wait_for(|| Ok(refusal.exists()))?;
    }
    let remembered = timed(RUNS, |at| {
        let folder = skipped.get(at).ok_or("no folder")?;
        machine.hook(&prompt(folder, FAMILIAR)?)
    })?;
    for (case, times) in [
        ("adoption off", off),
        ("adoption on, first offer", first),
        ("adoption on, refused before", remembered),
    ] {
        let (median, p95) = percentiles(times);
        println!("{case}: median {median:?}, p95 {p95:?} ({RUNS} runs)");
        assert!(p95 < Duration::from_millis(150), "{case}: p95 {p95:?}");
    }
    Ok(())
}

/// How long each of `runs` calls of `call` took, the run's index given, each having ended as an
/// agent requires: exit code 0 and nothing printed.
fn timed(runs: usize, call: impl Fn(usize) -> TestResult<Call>) -> TestResult<Vec<Duration>> {
    let mut times = Vec::with_capacity(runs);
    for at in 0..runs {
        let done = call(at)?;
        assert!(done.is_silent_success(), "{:?}", done.output);
        times.push(done.took);
    }
    Ok(times)
}

/// The median and the 95th percentile of `times`, by the nearest rank.
fn percentiles(mut times: Vec<Duration>) -> (Duration, Duration) {
    times.sort();
    let rank = |share: usize| {
        let at = (times.len() * share).div_ceil(100).saturating_sub(1);
        times.get(at).copied().unwrap_or_default()
    };
    (rank(50), rank(95))
}
