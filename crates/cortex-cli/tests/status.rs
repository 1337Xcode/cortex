//! `cortex status` against stores the real writer wrote, with an empty home folder so that no user
//! configuration applies.

use std::error::Error;
use std::num::NonZeroUsize;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use cortex_core::{ModeSetting, RepoId};
use cortex_index::{DiscoverOptions, Grammars, IndexOptions, index_repository};
use cortex_store::{Writer, WriterOptions};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// What one run of `cortex status` printed.
struct Status {
    /// The exit code.
    code: Option<i32>,
    /// Everything written to stdout.
    stdout: String,
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
    };
    index_repository(root, &Arc::new(Grammars::load()?), writer, &options)?;
    Ok(())
}

/// A repository without a store says so. A store whose first index has not finished is not ready
/// and has no mode. After an index of two Python files and a text file, held open by a writer as
/// a daemon holds it: the daemon runs, the index is ready, two files of three were parsed and the
/// repository is tiny. With the writer gone and the mode pinned to small, the daemon no longer
/// runs, and the mode is the one the last index recorded, with the pinned one beside it.
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
            "no index in {}; `cortex daemon` builds one\n",
            root.display()
        )
    );
    std::fs::write(root.join("a.py"), "def a():\n    pass\n")?;
    std::fs::write(root.join("b.py"), "def b():\n    pass\n")?;
    std::fs::write(root.join("notes.txt"), "notes\n")?;
    let mut writer = writer(root)?;
    let empty = status(root, home.path())?;
    assert_eq!(
        empty.stdout,
        "daemon: running\nindex: not ready, the first index has not finished\n\
         mode: not counted yet\nfiles: 0 stored, 0 parsed\n"
    );
    index(root, &mut writer)?;
    let ready = status(root, home.path())?;
    assert_eq!(
        ready.stdout,
        "daemon: running\nindex: ready\nmode: tiny\nfiles: 3 stored, 2 parsed\n"
    );
    drop(writer);
    std::fs::write(root.join(".cortex/config.toml"), "mode = \"small\"\n")?;
    let stopped = status(root, home.path())?;
    assert_eq!(
        stopped.stdout,
        "daemon: not running\nindex: ready\nmode: tiny; pinned to small in the configuration\n\
         files: 3 stored, 2 parsed\n"
    );
    Ok(())
}
