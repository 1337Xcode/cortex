//! `cortex uninstall` after `cortex init` on temporary repositories with an empty home folder:
//! every file init changed gets its bytes back and the daemon stops; a file changed since keeps
//! the change and loses only what init added; runs are undone newest first, and another
//! repository's files are left alone; a manifest that came with the repository is never read; a
//! backup copy that does not hold what init copied, or is a link, is not restored; a daemon of a
//! newer Cortex is stopped too, and one that never answers fails uninstall; running it again
//! changes nothing. `--purge` deletes what Cortex keeps of the repository but the folder the team
//! shares, never through a link, never the home folder's, and never while a change is left;
//! `--scope user` also undoes the user's own files.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::fake_daemon::{FakeDaemon, Script, kinds};
use common::init::{git, init, run};
use common::{Printed, StopsDaemon, TestResult};
use cortex_core::ipc::CORTEX_VERSION;
use cortex_core::{ContentHash, RepoId, UserFolder, canonical_root};
use cortex_daemon::is_running;

/// Runs `cortex uninstall --root <repo>` with `extra` arguments and `home` as the home folder.
fn uninstall(repo: &Path, home: &Path, extra: &[&str]) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["uninstall", "--root"]).arg(repo).args(extra);
    run(&mut command, home, &[])
}

/// The names of the entries of `folder`, sorted.
fn names(folder: &Path) -> TestResult<Vec<String>> {
    let mut names = std::fs::read_dir(folder)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

/// The one run folder init made in `repo`'s `.cortex/backups/`, under the repository's canonical
/// path, as uninstall names it.
fn only_run(repo: &Path) -> TestResult<PathBuf> {
    let backups = canonical_root(repo)?.join(".cortex").join("backups");
    let mut runs = std::fs::read_dir(backups)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    runs.retain(|path| path.is_dir());
    let [run] = runs.as_slice() else {
        return Err(format!("{runs:?}").into());
    };
    Ok(run.clone())
}

/// The line uninstall ends with while the repository at `root` keeps its `.cortex` folder.
fn stays(root: &Path) -> String {
    format!(
        "note: {} stays, with the index and init's backups; `cortex uninstall --purge` deletes \
         it\n",
        root.join(".cortex").display()
    )
}

/// Whether the daemon of the repository at `repo` runs, with `home` as the home folder.
fn is_daemon_running(repo: &Path, home: &Path) -> TestResult<bool> {
    let repo_id = RepoId::of(&canonical_root(repo)?);
    Ok(is_running(&UserFolder::in_home(home), &repo_id)?)
}

/// A git repository whose `.gitignore` ends without a line break and whose local settings hold
/// the user's own Stop hook: init changes both and makes `.cortex/config.toml`; uninstall gives
/// both their bytes back, removes the configuration init made and stops the daemon; init never
/// made a configuration of the user's own. Run again, it finds nothing to undo and no daemon.
#[test]
fn uninstall_gives_back_every_byte_init_changed() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let gitignore = repo.path().join(".gitignore");
    std::fs::write(&gitignore, "target/")?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    let original =
        r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "notify.sh"}]}]}}"#;
    std::fs::write(&settings, original)?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    assert_ne!(std::fs::read_to_string(&settings)?, original);
    assert!(is_daemon_running(repo.path(), home.path())?);
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    let root = canonical_root(repo.path())?;
    let expected = format!(
        "repository: {}\nremoved {}, which init made\nrestored {} from its backup\n\
         restored {} from its backup\ndaemon: stopped (Cortex {CORTEX_VERSION})\n{}",
        root.display(),
        root.join(".cortex").join("config.toml").display(),
        root.join(".claude").join("settings.local.json").display(),
        root.join(".gitignore").display(),
        stays(&root),
    );
    assert_eq!(undone.stdout, expected);
    assert_eq!(std::fs::read_to_string(&gitignore)?, "target/");
    assert_eq!(std::fs::read_to_string(&settings)?, original);
    assert!(!repo.path().join(".cortex").join("config.toml").exists());
    assert!(!home.path().join(".cortex").join("config.toml").exists());
    assert!(!is_daemon_running(repo.path(), home.path())?);
    let again = uninstall(repo.path(), home.path(), &[])?;
    assert!(again.is_success, "{}", again.stderr);
    assert_eq!(
        again.stdout,
        format!(
            "repository: {}\ndaemon: not running\n{}",
            root.display(),
            stays(&root)
        )
    );
    assert_eq!(std::fs::read_to_string(&gitignore)?, "target/");
    assert_eq!(std::fs::read_to_string(&settings)?, original);
    Ok(())
}

/// Files the user changed after init keep the change: the settings init made keep the user's
/// model and lose Cortex's hooks, the `.gitignore` init made keeps the user's line and loses
/// Cortex's, and the configuration init made is kept as the user changed it.
#[test]
fn uninstall_keeps_what_the_user_changed_since() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let settings = repo.path().join(".claude").join("settings.local.json");
    let written = std::fs::read_to_string(&settings)?;
    let changed = written.replacen("{\n", "{\n  \"model\": \"opus\",\n", 1);
    assert_ne!(changed, written);
    std::fs::write(&settings, changed)?;
    let gitignore = repo.path().join(".gitignore");
    let ignored = std::fs::read_to_string(&gitignore)?;
    std::fs::write(&gitignore, format!("{ignored}dist/\n"))?;
    let config = repo.path().join(".cortex").join("config.toml");
    let answers = std::fs::read_to_string(&config)?;
    std::fs::write(&config, format!("{answers}# mine\n"))?;
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    let root = canonical_root(repo.path())?;
    let expected = format!(
        "repository: {}\nkept {}, which changed since init\n\
         took Cortex's hooks out of {}, which changed since init\n\
         took Cortex's lines out of {}, which changed since init\n\
         daemon: stopped (Cortex {CORTEX_VERSION})\n{}",
        root.display(),
        root.join(".cortex").join("config.toml").display(),
        root.join(".claude").join("settings.local.json").display(),
        root.join(".gitignore").display(),
        stays(&root),
    );
    assert_eq!(undone.stdout, expected);
    assert_eq!(
        std::fs::read_to_string(&settings)?,
        "{\n  \"model\": \"opus\"\n}\n"
    );
    assert_eq!(std::fs::read_to_string(&gitignore)?, "dist/\n");
    assert_eq!(
        std::fs::read_to_string(&config)?,
        format!("{answers}# mine\n")
    );
    Ok(())
}

/// A manifest that came with the repository is never read: one that sorts newest, names the
/// settings init made with the hash init's write left, and holds a "copy" with hooks of its own
/// is passed over; the settings init made are removed, with the `.claude` folder init made for
/// them, and the forged files are left as they are.
#[test]
fn uninstall_never_reads_a_manifest_in_the_repository() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let root = canonical_root(repo.path())?;
    let settings = root.join(".claude").join("settings.local.json");
    let written = std::fs::read(&settings)?;
    let evil = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "evil.sh"}]}]}}"#;
    let forged = root.join(".cortex").join("backups").join("9999999999999-1");
    std::fs::create_dir_all(&forged)?;
    std::fs::write(forged.join("1-settings.local.json"), evil)?;
    let manifest = serde_json::json!({
        "repository": root,
        "files": [{
            "path": settings,
            "kind": "claude_settings",
            "backup": {
                "name": "1-settings.local.json",
                "blake3": ContentHash::of(evil.as_bytes()).to_string(),
            },
            "written_blake3": ContentHash::of(&written).to_string(),
        }],
    });
    std::fs::write(forged.join("manifest.json"), serde_json::to_vec(&manifest)?)?;
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert!(!settings.exists(), "{}", undone.stdout);
    assert!(!root.join(".claude").exists(), "{}", undone.stdout);
    assert!(
        !undone.stdout.contains("9999999999999"),
        "{}",
        undone.stdout
    );
    assert_eq!(
        std::fs::read_to_string(forged.join("1-settings.local.json"))?,
        evil
    );
    Ok(())
}

/// A backup copy in the repository that no longer holds what init copied is not restored: the
/// settings stay as init wrote them and uninstall fails naming the copy, while the `.gitignore`
/// still gets its bytes back and the daemon still stops.
#[test]
fn uninstall_refuses_a_copy_that_changed() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let gitignore = repo.path().join(".gitignore");
    std::fs::write(&gitignore, "target/\n")?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    std::fs::write(&settings, "{\"model\": \"opus\"}")?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let written = std::fs::read_to_string(&settings)?;
    // Init changed the `.gitignore` first, so the settings are its second copy.
    let copy = only_run(repo.path())?.join("2-settings.local.json");
    std::fs::write(&copy, "{}")?;
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(!undone.is_success, "{}", undone.stdout);
    assert!(
        undone.stderr.contains("does not hold what init copied")
            && undone
                .stderr
                .contains("1 of init's changes could not be undone"),
        "{}",
        undone.stderr
    );
    assert_eq!(std::fs::read_to_string(&settings)?, written);
    assert_eq!(std::fs::read_to_string(&gitignore)?, "target/\n");
    assert!(
        undone
            .stdout
            .contains(&format!("daemon: stopped (Cortex {CORTEX_VERSION})\n")),
        "{}",
        undone.stdout
    );
    Ok(())
}

/// With nothing of init's to undo, uninstall says so and stops whatever daemon holds the
/// repository, here a fake one of a newer Cortex that speaks another protocol: every version
/// reads a stop.
#[test]
fn uninstall_stops_a_daemon_of_a_newer_cortex() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let newer = FakeDaemon::start(repo.path(), home.path(), Script::Mismatch("99.0.0"))?;
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    let root = canonical_root(repo.path())?;
    let expected = format!(
        "repository: {}\nnothing to restore: no manifest of init's in {} names this repository\n\
         daemon: stopped (Cortex 99.0.0)\n",
        root.display(),
        home.path().join(".cortex").join("backups").display()
    );
    assert_eq!(undone.stdout, expected);
    assert_eq!(kinds(&newer.requests()?), ["hello", "shutdown"]);
    Ok(())
}

/// Two repositories set up in one home folder: uninstalling one undoes its own files alone and
/// stops its own daemon alone.
#[test]
fn uninstall_leaves_another_repository_alone() -> TestResult {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _first_daemon = StopsDaemon::of(first.path(), home.path());
    let _second_daemon = StopsDaemon::of(second.path(), home.path());
    for repo in [&first, &second] {
        git(repo.path(), &["init", "-q"], home.path())?;
        let done = init(repo.path(), home.path(), &[])?;
        assert!(done.is_success, "{}", done.stderr);
    }
    let files = [
        second.path().join(".claude").join("settings.local.json"),
        second.path().join(".gitignore"),
        second.path().join(".cortex").join("config.toml"),
    ];
    let before = files
        .iter()
        .map(std::fs::read)
        .collect::<Result<Vec<_>, _>>()?;
    let undone = uninstall(first.path(), home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    let first_settings = first.path().join(".claude").join("settings.local.json");
    assert!(!first_settings.exists());
    let after = files
        .iter()
        .map(std::fs::read)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(after, before);
    assert!(is_daemon_running(second.path(), home.path())?);
    Ok(())
}

/// Two runs of init on one file: the second put back the hooks the user had taken out by hand,
/// the file compact as the user wrote it. Undone newest first, the second run's copy gives the
/// user's file back byte for byte, and the first run then finds none of its hooks to take out.
#[test]
fn uninstall_undoes_runs_newest_first() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let first = init(repo.path(), home.path(), &[])?;
    assert!(first.is_success, "{}", first.stderr);
    let settings = repo.path().join(".claude").join("settings.local.json");
    let compact = r#"{"model":"opus"}"#;
    std::fs::write(&settings, compact)?;
    let second = init(repo.path(), home.path(), &[])?;
    assert!(second.is_success, "{}", second.stderr);
    assert_ne!(std::fs::read_to_string(&settings)?, compact);
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(std::fs::read_to_string(&settings)?, compact);
    Ok(())
}

/// A `.gitignore` holding a block of Cortex's lines a team committed before init gets a block of
/// init's own; changed by the user after init, it loses init's block alone, keeping the team's
/// and the user's line, and a second uninstall changes nothing.
#[test]
fn uninstall_keeps_a_team_s_block_of_cortex_lines() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let gitignore = repo.path().join(".gitignore");
    std::fs::write(&gitignore, "# Cortex\n.cortex/\n")?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let both = "# Cortex\n.cortex/\n# Cortex\n.claude/settings.local.json\n";
    assert_eq!(std::fs::read_to_string(&gitignore)?, both);
    std::fs::write(&gitignore, format!("{both}dist/\n"))?;
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(
        std::fs::read_to_string(&gitignore)?,
        "# Cortex\n.cortex/\ndist/\n"
    );
    let again = uninstall(repo.path(), home.path(), &[])?;
    assert!(again.is_success, "{}", again.stderr);
    assert_eq!(
        std::fs::read_to_string(&gitignore)?,
        "# Cortex\n.cortex/\ndist/\n"
    );
    Ok(())
}

/// Settings that held a Cortex hook before init, as ones copied from a teammate, get it back, and
/// a second uninstall, finding the file as it was before init, leaves it alone.
#[test]
fn uninstall_leaves_a_file_as_it_was_before_init() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    let teammates = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/opt/cortex/bin/cortex", "args": ["hook", "--agent", "claude", "--event", "Stop"]}]}]}}"#;
    std::fs::write(&settings, teammates)?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    assert_ne!(std::fs::read_to_string(&settings)?, teammates);
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(std::fs::read_to_string(&settings)?, teammates);
    let again = uninstall(repo.path(), home.path(), &[])?;
    assert!(again.is_success, "{}", again.stderr);
    assert_eq!(std::fs::read_to_string(&settings)?, teammates);
    Ok(())
}

/// A backup copy that is a link, here to an endless file, is refused before it is read: uninstall
/// fails naming it and leaves the settings as init wrote them. Links need no privilege on Unix
/// alone.
#[cfg(unix)]
#[test]
fn uninstall_refuses_a_copy_that_is_a_link() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    std::fs::write(&settings, "{\"model\": \"opus\"}")?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let written = std::fs::read_to_string(&settings)?;
    // Init made the `.gitignore` first, so the settings are its second copy.
    let copy = only_run(repo.path())?.join("2-settings.local.json");
    std::fs::remove_file(&copy)?;
    std::os::unix::fs::symlink("/dev/zero", &copy)?;
    let undone = uninstall(repo.path(), home.path(), &[])?;
    assert!(!undone.is_success, "{}", undone.stdout);
    assert!(
        undone
            .stderr
            .contains(&format!("{} is not a plain file", copy.display())),
        "{}",
        undone.stderr
    );
    assert_eq!(std::fs::read_to_string(&settings)?, written);
    Ok(())
}

/// A daemon that holds the repository but never answers, as a stuck one, fails uninstall once the
/// 20 s it is given have passed, and keeps `--purge` from deleting anything: here the test holds
/// the repository's lock and nothing listens.
#[test]
fn uninstall_fails_when_the_daemon_never_answers() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let user = UserFolder::in_home(home.path());
    let files = user.run_files(&RepoId::of(&canonical_root(repo.path())?));
    std::fs::create_dir_all(user.run_folder())?;
    let lock = std::fs::File::create(files.lock_path())?;
    lock.lock()?;
    let kept = repo.path().join(".cortex").join("keep.txt");
    std::fs::create_dir_all(repo.path().join(".cortex"))?;
    std::fs::write(&kept, "mine")?;
    let undone = uninstall(repo.path(), home.path(), &["--purge"])?;
    assert!(!undone.is_success, "{}", undone.stdout);
    assert!(
        undone
            .stderr
            .contains("the daemon holds the repository but did not answer within 20 s")
            && undone.stderr.contains(&format!(
                "nothing purged from {}: a daemon holds the repository",
                canonical_root(repo.path())?.join(".cortex").display()
            )),
        "{}",
        undone.stderr
    );
    assert_eq!(std::fs::read_to_string(&kept)?, "mine");
    drop(lock);
    Ok(())
}

/// `--purge` deletes what Cortex keeps of the repository but the folder the team shares: its
/// `.cortex/` but `.cortex/shared/`, init's manifest for it in the user's folder (with its run
/// folder, empty then), and the daemon's log with the files it rotated to. Another repository's
/// log stays, and so do the run files and the user's own configuration.
#[test]
fn purge_deletes_what_cortex_keeps_but_the_shared_folder() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let users_config = home.path().join(".cortex").join("config.toml");
    std::fs::write(&users_config, "mode = \"auto\"\n")?;
    let root = canonical_root(repo.path())?;
    let shared = root.join(".cortex").join("shared");
    std::fs::create_dir_all(&shared)?;
    std::fs::write(shared.join("facts.json"), "[]")?;
    let logs = home.path().join(".cortex").join("logs");
    let repo_id = RepoId::of(&root);
    let log = logs.join(format!("daemon-{repo_id}.log"));
    assert!(log.exists());
    let rotated = logs.join(format!("daemon-{repo_id}.log.1"));
    std::fs::write(&rotated, "older\n")?;
    let other = logs.join("daemon-0000000000000000.log");
    std::fs::write(&other, "another repository's\n")?;
    let lock = UserFolder::in_home(home.path())
        .run_files(&repo_id)
        .lock_path();
    assert!(lock.exists());
    let backups = home.path().join(".cortex").join("backups");
    let [run] = <[String; 1]>::try_from(names(&backups)?).map_err(|names| format!("{names:?}"))?;
    let undone = uninstall(repo.path(), home.path(), &["--purge"])?;
    assert!(undone.is_success, "{}", undone.stderr);
    let purged = format!(
        "daemon: stopped (Cortex {CORTEX_VERSION})\npurged {} but {}, which the team shares \
         through git\npurged 1 of init's manifests for this repository from {}\npurged the \
         daemon's log of this repository from {}\n",
        root.join(".cortex").display(),
        shared.display(),
        backups.display(),
        logs.display()
    );
    assert!(undone.stdout.ends_with(&purged), "{}", undone.stdout);
    assert_eq!(names(&root.join(".cortex"))?, ["shared"]);
    assert_eq!(std::fs::read_to_string(shared.join("facts.json"))?, "[]");
    assert!(!log.exists() && !rotated.exists());
    assert_eq!(std::fs::read_to_string(&other)?, "another repository's\n");
    assert!(lock.exists());
    assert!(!backups.join(&run).exists());
    assert_eq!(std::fs::read_to_string(&users_config)?, "mode = \"auto\"\n");
    Ok(())
}

/// A change uninstall could not undo keeps `--purge` from deleting anything: the backups a retry
/// needs stay, and so does the rest of `.cortex/`.
#[test]
fn purge_deletes_nothing_while_a_change_is_left() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    std::fs::write(&settings, "{\"model\": \"opus\"}")?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    // Init made the `.gitignore` first, so the settings are its second copy.
    let run = only_run(repo.path())?;
    std::fs::write(run.join("2-settings.local.json"), "{}")?;
    let undone = uninstall(repo.path(), home.path(), &["--purge"])?;
    assert!(!undone.is_success, "{}", undone.stdout);
    assert!(
        undone.stderr.contains(&format!(
            "nothing purged from {}: it keeps the backups that undoing what is left needs",
            canonical_root(repo.path())?.join(".cortex").display()
        )),
        "{}",
        undone.stderr
    );
    assert!(run.join("2-settings.local.json").exists());
    Ok(())
}

/// `--purge` from the home folder is refused, its `.cortex` being the user's own, for every
/// repository, and so is `--purge` from a folder holding the home folder: each `.cortex` keeps
/// what it holds.
#[test]
fn purge_never_deletes_the_home_folder_s_cortex() -> TestResult {
    let outer = tempfile::tempdir()?;
    let home = outer.path().join("home");
    let own = home.join(".cortex");
    std::fs::create_dir_all(&own)?;
    std::fs::write(own.join("config.toml"), "mode = \"auto\"\n")?;
    let above = outer.path().join(".cortex");
    std::fs::create_dir_all(&above)?;
    std::fs::write(above.join("keep.txt"), "mine")?;
    for root in [home.as_path(), outer.path()] {
        let undone = uninstall(root, &home, &["--purge"])?;
        assert!(!undone.is_success, "{}", undone.stdout);
        assert!(
            undone.stderr.contains(&format!(
                "{} is the home folder, a folder holding it, or a file system's root, which is \
                 never a repository's; purge never deletes its `.cortex`",
                canonical_root(root)?.display()
            )),
            "{}",
            undone.stderr
        );
    }
    assert_eq!(
        std::fs::read_to_string(own.join("config.toml"))?,
        "mode = \"auto\"\n"
    );
    assert_eq!(std::fs::read_to_string(above.join("keep.txt"))?, "mine");
    Ok(())
}

/// A `.cortex` that is a link to a folder elsewhere is refused: purge deletes nothing through it,
/// and the folder it leads to keeps what it holds. A link inside a `.cortex` of the repository's
/// own goes as a link, the folder it leads to untouched. Links need no privilege on Unix alone.
#[cfg(unix)]
#[test]
fn purge_refuses_a_linked_cortex_folder() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    std::fs::write(elsewhere.path().join("keep.txt"), "mine")?;
    std::os::unix::fs::symlink(elsewhere.path(), repo.path().join(".cortex"))?;
    let undone = uninstall(repo.path(), home.path(), &["--purge"])?;
    assert!(!undone.is_success, "{}", undone.stdout);
    assert!(
        undone.stderr.contains("purge deletes nothing through it"),
        "{}",
        undone.stderr
    );
    assert_eq!(
        std::fs::read_to_string(elsewhere.path().join("keep.txt"))?,
        "mine"
    );
    let own = tempfile::tempdir()?;
    std::fs::create_dir(own.path().join(".cortex"))?;
    std::os::unix::fs::symlink(elsewhere.path(), own.path().join(".cortex").join("inside"))?;
    let purged = uninstall(own.path(), home.path(), &["--purge"])?;
    assert!(purged.is_success, "{}", purged.stderr);
    assert!(!own.path().join(".cortex").exists());
    assert_eq!(
        std::fs::read_to_string(elsewhere.path().join("keep.txt"))?,
        "mine"
    );
    Ok(())
}

/// After `init --scope user`, uninstalling the repository's files leaves the user's settings, which
/// hold Cortex's hooks for every repository, and says so last on stdout; `--scope user` then gives
/// the settings their bytes back. Init makes no configuration in the home folder.
#[test]
fn uninstall_for_the_user_undoes_the_user_s_files() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    std::fs::create_dir_all(home.path().join(".claude"))?;
    std::fs::write(
        home.path().join(".claude").join("settings.json"),
        "{\"theme\": \"dark\"}",
    )?;
    let done = init(repo.path(), home.path(), &["--scope", "user"])?;
    assert!(done.is_success, "{}", done.stderr);
    let settings = canonical_root(&home.path().join(".claude"))?.join("settings.json");
    let config = home.path().join(".cortex").join("config.toml");
    assert!(settings.exists() && !config.exists());
    let project = uninstall(repo.path(), home.path(), &[])?;
    assert!(project.is_success, "{}", project.stderr);
    assert!(settings.exists());
    let note = format!(
        "note: {} holds Cortex's Claude Code hooks, which run in every repository; `cortex \
         uninstall --scope user` takes them out\n",
        home.path().join(".claude").join("settings.json").display()
    );
    assert!(project.stdout.ends_with(&note), "{}", project.stdout);
    let user = uninstall(repo.path(), home.path(), &["--scope", "user"])?;
    assert!(user.is_success, "{}", user.stderr);
    let root = canonical_root(repo.path())?;
    let expected = format!(
        "repository: {}\nrestored {} from its backup\ndaemon: not running\n{}",
        root.display(),
        settings.display(),
        stays(&root)
    );
    assert_eq!(user.stdout, expected);
    assert_eq!(std::fs::read_to_string(&settings)?, "{\"theme\": \"dark\"}");
    assert!(!config.exists());
    Ok(())
}
