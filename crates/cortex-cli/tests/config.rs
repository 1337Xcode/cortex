//! `cortex config` and `cortex features` through the real binary, on a repository with a user file
//! and a repository file, with a home folder of the test's own: each value is shown with the file
//! it came from; `config set` writes one line of the right file, keeps a copy of the old one and
//! refuses what the configuration refuses; `features` lists every feature and switches one.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// What one run of `cortex` printed.
struct Printed {
    /// Whether it succeeded.
    is_success: bool,
    /// Its stdout.
    stdout: String,
    /// Its stderr.
    stderr: String,
}

/// A home folder holding the user's Cortex folder and a repository Cortex was set up in, with the
/// repository's root, the user's file and the repository's file (neither written yet).
fn setup() -> TestResult<(tempfile::TempDir, PathBuf, PathBuf, PathBuf)> {
    let home = tempfile::tempdir()?;
    let user = home.path().join(".cortex");
    let repo = home.path().join("repo");
    std::fs::create_dir_all(&user)?;
    std::fs::create_dir_all(repo.join(".cortex"))?;
    let repository_file = repo.join(".cortex").join("config.toml");
    Ok((home, repo, user.join("config.toml"), repository_file))
}

/// Runs `cortex <args> --root <root>` with `home` as the user's home folder, reading its output
/// through pipes.
fn cortex(home: &Path, root: &Path, args: &[&str]) -> TestResult<Printed> {
    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(args)
        .arg("--root")
        .arg(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("HOMEDRIVE")
        .env_remove("HOMEPATH")
        .output()?;
    Ok(Printed {
        is_success: output.status.success(),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

/// `cortex config` shows every key once, in the order of the keys, each with the file its value
/// came from: the repository's file over the user's, the user's over the profile, the profile the
/// user's file chose over the defaults. A key Cortex does not read is warned of as loading words it.
#[test]
fn each_value_shows_the_file_it_came_from() -> TestResult {
    let (home, repo, user_file, repository_file) = setup()?;
    std::fs::write(
        &user_file,
        "profile = \"everything\"\n[retrieval]\nmax_files = 7\nmax_tokens = 800\nflavour = 1\n",
    )?;
    std::fs::write(
        &repository_file,
        "[retrieval]\nmax_files = 9\n[hints]\nnarrow_tests = false\n",
    )?;
    let printed = cortex(home.path(), &repo, &["config"])?;
    assert!(printed.is_success, "{}", printed.stderr);
    let lines: Vec<&str> = printed.stdout.lines().collect();
    for expected in [
        "profile = \"everything\" (user)",
        "retrieval.max_files = 9 (repository)",
        "retrieval.max_tokens = 800 (user)",
        "hints.narrow_tests = false (repository)",
        "hints.runtime_grounding = true (profile)",
        "hints.redundant_commands = true (profile)",
        "compaction.keep_list = true (profile)",
        "retrieval.max_code_spans = 2 (default)",
        "mode = \"auto\" (default)",
        "holdout.share = 0.1 (default)",
    ] {
        assert!(lines.contains(&expected), "{expected}\n{}", printed.stdout);
    }
    // 18 keys of the main settings, 11 retrieval, 5 warning, 11 coordination, 3 hint and 1
    // compaction key, and the profile.
    assert_eq!(lines.len(), 50, "{}", printed.stdout);
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted);
    assert_eq!(
        printed.stderr,
        format!(
            "cortex: warning: {}: unknown key `retrieval.flavour` is ignored\n",
            user_file.display()
        )
    );
    let json = cortex(home.path(), &repo, &["config", "--json"])?;
    assert!(json.is_success, "{}", json.stderr);
    let rows: serde_json::Value = serde_json::from_str(&json.stdout)?;
    let row = rows
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["key"] == "retrieval.max_files"))
        .ok_or("no retrieval.max_files")?;
    assert_eq!(row["value"], 9);
    assert_eq!(row["source"], "repository");
    assert_eq!(row["description"], "How many files a work order lists.");
    Ok(())
}

/// `config set` changes the key's own line, its comment and the file's other lines kept; once the
/// change is made, what the file held before Cortex first changed it is kept beside it under a
/// name of Cortex's own, which a later change leaves as it is, and a file of the user's own named
/// like a backup is never touched. A key the file lacks goes in a table of its own, and the user's
/// file, made when there is none, gets a key with `--user`.
#[test]
fn a_set_key_is_written_and_the_old_file_kept() -> TestResult {
    let (home, repo, user_file, repository_file) = setup()?;
    let first = "# My settings.\n[retrieval]\nmax_files = 5 # five\n";
    std::fs::write(&repository_file, first)?;
    let users_own = repo.join(".cortex").join("config.toml.bak");
    std::fs::write(&users_own, "mine\n")?;
    let backup = repo.join(".cortex").join("config.toml.cortex-backup");
    let printed = cortex(
        home.path(),
        &repo,
        &["config", "set", "retrieval.max_files", "7"],
    )?;
    assert!(printed.is_success, "{}", printed.stderr);
    let second = "# My settings.\n[retrieval]\nmax_files = 7 # five\n";
    assert_eq!(std::fs::read_to_string(&repository_file)?, second);
    assert_eq!(std::fs::read_to_string(&backup)?, first);
    assert_eq!(
        printed.stdout,
        format!(
            "set retrieval.max_files = 7 in {}\nkept what {} held before as {}\n",
            repository_file.display(),
            repository_file.display(),
            backup.display()
        )
    );
    let printed = cortex(
        home.path(),
        &repo,
        &["config", "set", "hints.narrow_tests", "true"],
    )?;
    assert!(printed.is_success, "{}", printed.stderr);
    assert_eq!(
        std::fs::read_to_string(&repository_file)?,
        format!("{second}\n[hints]\nnarrow_tests = true\n")
    );
    assert_eq!(std::fs::read_to_string(&backup)?, first);
    assert_eq!(
        printed.stdout,
        format!(
            "set hints.narrow_tests = true in {}\n",
            repository_file.display()
        )
    );
    assert_eq!(std::fs::read_to_string(&users_own)?, "mine\n");
    let printed = cortex(
        home.path(),
        &repo,
        &["config", "set", "mode", "small", "--user"],
    )?;
    assert!(printed.is_success, "{}", printed.stderr);
    assert_eq!(std::fs::read_to_string(&user_file)?, "mode = \"small\"\n");
    assert!(
        !home
            .path()
            .join(".cortex")
            .join("config.toml.cortex-backup")
            .exists()
    );
    let shown = cortex(home.path(), &repo, &["config"])?;
    assert!(
        shown
            .stdout
            .lines()
            .any(|line| line == "mode = \"small\" (user)"),
        "{}",
        shown.stdout
    );
    Ok(())
}

/// A folder where Cortex's copy of the file goes is refused before anything is written: the file
/// keeps its text and nothing says a copy was kept.
#[test]
fn a_set_that_cannot_keep_its_copy_writes_nothing() -> TestResult {
    let (home, repo, _, repository_file) = setup()?;
    std::fs::write(&repository_file, "mode = \"auto\"\n")?;
    std::fs::create_dir(repo.join(".cortex").join("config.toml.cortex-backup"))?;
    let printed = cortex(home.path(), &repo, &["config", "set", "mode", "small"])?;
    assert!(!printed.is_success);
    assert!(
        printed.stderr.contains("is not a plain file"),
        "{}",
        printed.stderr
    );
    assert_eq!(printed.stdout, "");
    assert_eq!(
        std::fs::read_to_string(&repository_file)?,
        "mode = \"auto\"\n"
    );
    Ok(())
}

/// Makes `link` a link to the folder `target`: a junction on Windows, which needs no privilege, a
/// symbolic link elsewhere.
fn link_folder(target: &Path, link: &Path) -> TestResult {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link)?;
    #[cfg(windows)]
    {
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()?;
        if !made.status.success() {
            return Err(format!("{made:?}").into());
        }
    }
    Ok(())
}

/// A repository whose `.cortex` is a link to the user's own Cortex folder, as a cloned repository
/// could carry, is refused: `config set` writes nothing through it, and the user's own file keeps
/// its text.
#[test]
fn a_linked_cortex_folder_is_refused() -> TestResult {
    let home = tempfile::tempdir()?;
    let user = home.path().join(".cortex");
    let repo = home.path().join("repo");
    std::fs::create_dir_all(&user)?;
    std::fs::create_dir_all(&repo)?;
    let user_file = user.join("config.toml");
    std::fs::write(&user_file, "[retrieval]\nmax_files = 5\n")?;
    link_folder(&user, &repo.join(".cortex"))?;
    let printed = cortex(
        home.path(),
        &repo,
        &["config", "set", "retrieval.max_files", "7"],
    )?;
    assert!(!printed.is_success, "{}", printed.stdout);
    assert!(
        printed.stderr.contains("leads elsewhere"),
        "{}",
        printed.stderr
    );
    assert_eq!(
        std::fs::read_to_string(&user_file)?,
        "[retrieval]\nmax_files = 5\n"
    );
    assert!(!user.join("config.toml.cortex-backup").exists());
    Ok(())
}

/// `config set` refuses an unknown key, naming the nearest; a value the key does not accept, in
/// the words loading refuses it with; a value it cannot place without changing anything else the
/// file says; and a repository Cortex was not set up in. Each leaves every file as it was.
#[test]
fn what_the_configuration_refuses_is_not_written() -> TestResult {
    let (home, repo, _, repository_file) = setup()?;
    let text = "[tests]\nextra_patterns = [\n  \"^just test\",\n]\n";
    std::fs::write(&repository_file, text)?;
    let refusals = [
        (
            ["retrieval.max_filez", "7"],
            "unknown key `retrieval.max_filez`; the nearest key Cortex reads is \
             `retrieval.max_files`",
        ),
        (
            ["retrieval.max_files", "21"],
            "`retrieval.max_files` must be a whole number from 1 to 20",
        ),
        (
            ["profile", "heavy"],
            "`profile` must be one of standard, lean, everything",
        ),
        (
            ["tests.extra_patterns", "[\"tox\"]"],
            "cannot set `tests.extra_patterns` without changing anything else it says; set it \
             there by hand",
        ),
    ];
    for ([key, value], expected) in refusals {
        let printed = cortex(home.path(), &repo, &["config", "set", key, value])?;
        assert!(!printed.is_success, "{key}");
        assert!(printed.stderr.contains(expected), "{}", printed.stderr);
        assert_eq!(std::fs::read_to_string(&repository_file)?, text);
    }
    assert!(
        !repo
            .join(".cortex")
            .join("config.toml.cortex-backup")
            .exists()
    );
    let bare = home.path().join("bare");
    std::fs::create_dir_all(&bare)?;
    let printed = cortex(home.path(), &bare, &["config", "set", "mode", "small"])?;
    assert!(!printed.is_success);
    assert!(
        printed.stderr.contains("Cortex has not been set up here"),
        "{}",
        printed.stderr
    );
    assert!(!bare.join(".cortex").exists());
    Ok(())
}

/// `cortex features` lists every feature with whether the settings let it reach a task and no
/// measured effect; `features on` and `off` write its key to the repository's file, and a feature
/// with no key, or an unknown one, is refused. A feature with no key is off when the settings keep
/// it from every task: warnings with no warning allowed an edit, every feature with every task held
/// out.
#[test]
fn features_are_listed_and_switched() -> TestResult {
    let (home, repo, _, repository_file) = setup()?;
    let printed = cortex(home.path(), &repo, &["features"])?;
    assert!(printed.is_success, "{}", printed.stderr);
    let lines: Vec<&str> = printed.stdout.lines().collect();
    for expected in [
        "work_orders: on (state on, no key)",
        "warnings: on (state on, no key)",
        "runtime_grounding: off (state experimental, key hints.runtime_grounding)",
        "narrow_tests: off (state experimental, key hints.narrow_tests)",
        "redundant_commands: off (state experimental, key hints.redundant_commands)",
        "keep_list: off (state experimental, key compaction.keep_list)",
    ] {
        assert!(lines.contains(&expected), "{expected}\n{}", printed.stdout);
    }
    let unmeasured = lines
        .iter()
        .filter(|line| **line == "  effect here: not measured here yet")
        .count();
    assert_eq!(unmeasured, 6);
    let printed = cortex(home.path(), &repo, &["features", "on", "narrow_tests"])?;
    assert!(printed.is_success, "{}", printed.stderr);
    assert_eq!(
        std::fs::read_to_string(&repository_file)?,
        "[hints]\nnarrow_tests = true\n"
    );
    let listed = cortex(home.path(), &repo, &["features"])?;
    assert!(
        listed
            .stdout
            .lines()
            .any(|line| line == "narrow_tests: on (state experimental, key hints.narrow_tests)"),
        "{}",
        listed.stdout
    );
    let printed = cortex(home.path(), &repo, &["features", "off", "narrow_tests"])?;
    assert!(printed.is_success, "{}", printed.stderr);
    assert_eq!(
        std::fs::read_to_string(&repository_file)?,
        "[hints]\nnarrow_tests = false\n"
    );
    let printed = cortex(home.path(), &repo, &["features", "off", "warnings"])?;
    assert!(!printed.is_success);
    assert!(
        printed
            .stderr
            .contains("`warnings` has no key of its own to switch it"),
        "{}",
        printed.stderr
    );
    let printed = cortex(home.path(), &repo, &["features", "on", "teleport"])?;
    assert!(!printed.is_success);
    assert!(
        printed.stderr.contains(
            "unknown feature `teleport`; the features are work_orders, warnings, \
             runtime_grounding, narrow_tests, redundant_commands, keep_list"
        ),
        "{}",
        printed.stderr
    );
    std::fs::write(&repository_file, "[warnings]\nmax_per_edit = 0\n")?;
    let listed = cortex(home.path(), &repo, &["features"])?;
    assert!(
        listed
            .stdout
            .lines()
            .any(|line| line == "warnings: off (state on, no key)"),
        "{}",
        listed.stdout
    );
    std::fs::write(
        &repository_file,
        "profile = \"everything\"\nholdout.share = 1\n",
    )?;
    let listed = cortex(home.path(), &repo, &["features"])?;
    let on = listed
        .stdout
        .lines()
        .filter(|line| line.contains(": on ("))
        .count();
    assert_eq!(on, 0, "{}", listed.stdout);
    Ok(())
}
