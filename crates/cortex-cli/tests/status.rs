//! `cortex status` against stores the real writer wrote, with an empty home folder so that no user
//! configuration applies.

use std::error::Error;
use std::num::NonZeroUsize;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use cortex_core::{ModeSetting, RepoId, canonical_root};
use cortex_index::{DiscoverOptions, Grammars, IndexOptions, index_repository};
use cortex_store::{SelfCheckRecord, Writer, WriterOptions};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// What one run of `cortex status` printed.
struct Status {
    /// The exit code.
    code: Option<i32>,
    /// Everything written to stdout.
    stdout: String,
    /// Everything written to stderr.
    stderr: String,
}

/// Runs `cortex status --root <root>` with `home` as the user's home folder.
fn status(root: &Path, home: &Path) -> TestResult<Status> {
    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(["status", "--root"])
        .arg(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .output()?;
    Ok(Status {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

/// Opens the writer of the repository at `root`.
fn writer(root: &Path) -> TestResult<Writer> {
    Ok(Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(root),
        mmap_bytes: 0,
        now_ms: 1_700_000_000_000,
    })?)
}

/// Indexes the repository at `root` with `writer`, the mode left to the size.
fn index(root: &Path, writer: &mut Writer) -> TestResult {
    let options = IndexOptions {
        discover: DiscoverOptions {
            max_file_bytes: 1_048_576,
            extra_ignores: &[],
            global_gitignore: None,
        },
        parse_timeout: Duration::from_secs(60),
        threads: NonZeroUsize::MIN,
        now_ms: 1_700_000_000_000,
        mode: ModeSetting::Auto,
        pause: None,
    };
    index_repository(root, &Arc::new(Grammars::load()?), writer, &options)?;
    Ok(())
}

/// With no daemon running: a repository without a store says so. A store whose first index has
/// not finished, its writer held open as an index holds it, is not ready, with the index under
/// way, and has no mode. After an index of two Python files and a text file the index is ready,
/// two files of three were parsed and the repository is tiny. With the writer gone and the mode
/// pinned to small, the mode is the one the last index recorded, with the pinned one beside it.
#[test]
fn status_reports_the_daemon_the_index_and_the_mode() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    let none = status(root, home.path())?;
    assert_eq!(none.code, Some(0));
    assert_eq!(
        none.stdout,
        format!(
            "daemon: not running\nno index in {}; `cortex daemon` builds one\n",
            canonical_root(root)?.display()
        )
    );
    std::fs::write(root.join("a.py"), "def a():\n    pass\n")?;
    std::fs::write(root.join("b.py"), "def b():\n    pass\n")?;
    std::fs::write(root.join("notes.txt"), "notes\n")?;
    let mut writer = writer(root)?;
    let empty = status(root, home.path())?;
    assert_eq!(
        empty.stdout,
        "daemon: not running\nindex: not ready, an index is under way\n\
         mode: not counted yet\nfiles: 0 stored, 0 parsed\nco-change: nothing mined yet\n\
         gate: a work order when the prompt's words match a file at 0.50 or more and 2 kinds of \
         evidence agree, or when a session goes on with its own work\n\
         self-check: no work order given is labelled yet; the bar is not raised\n"
    );
    index(root, &mut writer)?;
    let ready = status(root, home.path())?;
    assert_eq!(
        ready.stdout,
        "daemon: not running\nindex: ready\nmode: tiny\nfiles: 3 stored, 2 parsed\n\
         co-change: nothing mined yet\n\
         gate: a work order when the prompt's words match a file at 0.50 or more and 2 kinds of \
         evidence agree, or when a session goes on with its own work\n\
         self-check: no work order given is labelled yet; the bar is not raised\n"
    );
    drop(writer);
    std::fs::write(root.join(".cortex/config.toml"), "mode = \"small\"\n")?;
    let stopped = status(root, home.path())?;
    assert_eq!(
        stopped.stdout,
        "daemon: not running\nindex: ready\nmode: tiny; pinned to small in the configuration\n\
         files: 3 stored, 2 parsed\nco-change: nothing mined yet\n\
         gate: a work order when the prompt's words match a file at 0.50 or more and 2 kinds of \
         evidence agree, or when a session goes on with its own work\n\
         self-check: no work order given is labelled yet; the bar is not raised\n"
    );
    Ok(())
}

/// A store whose writer is still setting it up is not ready, rather than unreadable: a daemon
/// answers before it has made its store, so a status asked then meets one. Here the database file
/// is empty, an SQLite database without tables, as the writer's is between its creation and its
/// first migration, and the test holds the writer's lock as the writer does. With no writer, the
/// same store is an error that names its schema.
#[test]
fn a_store_being_set_up_is_not_ready() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    let folder = root.join(".cortex");
    std::fs::create_dir(&folder)?;
    std::fs::write(folder.join("cortex.db"), "")?;
    let lock = std::fs::File::create(folder.join("writer.lock"))?;
    lock.lock()?;
    let setting_up = status(root, home.path())?;
    assert_eq!(setting_up.code, Some(0), "{}", setting_up.stderr);
    assert_eq!(
        setting_up.stdout,
        "daemon: not running\nindex: not ready, the store is being set up\n"
    );
    drop(lock);
    let unwritten = status(root, home.path())?;
    assert_eq!(unwritten.code, Some(1), "{}", unwritten.stdout);
    assert!(
        unwritten
            .stderr
            .contains("the database schema is version 0, not "),
        "{}",
        unwritten.stderr
    );
    Ok(())
}

/// From a folder inside a repository Cortex set up, with no `--root`, status reports that
/// repository, found as a hook finds it: here its index of one Python file, from `src/deep`.
#[test]
fn status_finds_the_repository_from_a_folder_inside_it() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::create_dir_all(root.join("src").join("deep"))?;
    std::fs::write(root.join("src").join("a.py"), "def a():\n    pass\n")?;
    let mut writer = writer(root)?;
    index(root, &mut writer)?;
    drop(writer);
    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .arg("status")
        .current_dir(root.join("src").join("deep"))
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()?;
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "daemon: not running\nindex: ready\nmode: tiny\nfiles: 1 stored, 1 parsed\n\
         co-change: nothing mined yet\n\
         gate: a work order when the prompt's words match a file at 0.50 or more and 2 kinds of \
         evidence agree, or when a session goes on with its own work\n\
         self-check: no work order given is labelled yet; the bar is not raised\n"
    );
    Ok(())
}

/// What the self-check last found shows in the gate's bar and the self-check's line. Its record
/// here: 52 of the latest 100 work orders given used, so a precision of 0.52; the bar raised 2
/// steps of 0.05, so by 0.10, to 0.50 + 0.10 = 0.60; raised last when 130 had been labelled. A
/// record that cannot be read is said to be so, the rest of the status standing.
#[test]
fn status_shows_the_gate_as_the_self_check_left_it() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::write(root.join("a.py"), "def a():\n    pass\n")?;
    let mut writer = writer(root)?;
    index(root, &mut writer)?;
    let mut transaction = writer.transaction()?;
    transaction.set_self_check(&SelfCheckRecord {
        raise_steps: 2,
        raised_at: 130,
        recent_labelled: 100,
        recent_used: 52,
    })?;
    transaction.commit()?;
    drop(writer);
    let raised = status(root, home.path())?;
    assert_eq!(
        raised.stdout,
        "daemon: not running\nindex: ready\nmode: tiny\nfiles: 1 stored, 1 parsed\n\
         co-change: nothing mined yet\n\
         gate: a work order when the prompt's words match a file at 0.60 or more and 2 kinds of \
         evidence agree, or when a session goes on with its own work\n\
         self-check: precision 0.52 over the latest 100 work orders given; the bar is raised 2 \
         of 4 steps, by 0.10, last when 130 work orders had been labelled\n"
    );
    let store = rusqlite::Connection::open(root.join(".cortex/cortex.db"))?;
    store.execute(
        "UPDATE meta SET value = '{not a record' WHERE key = 'self_check'",
        [],
    )?;
    drop(store);
    let unreadable = status(root, home.path())?;
    assert_eq!(unreadable.code, Some(0), "{}", unreadable.stderr);
    assert_eq!(
        unreadable.stdout,
        "daemon: not running\nindex: ready\nmode: tiny\nfiles: 1 stored, 1 parsed\n\
         co-change: nothing mined yet\ngate: the self-check's record cannot be read\n"
    );
    Ok(())
}
