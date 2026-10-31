//! `cortex daemon` on temporary repositories, with an empty home folder so that no user
//! configuration applies: it indexes and says so, indexes a file again after it changes, refuses a
//! second daemon on the same repository, and in huge mode parses what it took inventory of.

use std::error::Error;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cortex_core::Identifier;
use cortex_store::Reader;

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Long enough for a debug build to load every grammar and index a few files.
const READY_WITHIN: Duration = Duration::from_secs(120);

/// Long enough for the watcher's 200 ms of quiet, or the crawler's first batch, and an index of a
/// few files.
const SEEN_WITHIN: Duration = Duration::from_secs(60);

/// How often a condition is checked while waiting for it.
const POLL_EVERY: Duration = Duration::from_millis(50);

/// A running daemon, stopped when dropped, and the lines it writes to stderr.
struct Daemon {
    /// The process.
    child: Child,
    /// Its stderr, a line at a time.
    lines: mpsc::Receiver<String>,
}

impl Daemon {
    /// Starts `cortex daemon` on `root`, with `home` as the user's home folder.
    fn start(root: &Path, home: &Path) -> TestResult<Self> {
        let mut child = daemon_command(root, home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child.stderr.take().ok_or("stderr is not piped")?;
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self { child, lines })
    }

    /// The first line the daemon writes, waited for.
    fn first_line(&self) -> TestResult<String> {
        Ok(self.lines.recv_timeout(READY_WITHIN)?)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // The test is over: a daemon that already stopped cannot be killed, and that is fine.
        drop(self.child.kill());
        drop(self.child.wait());
    }
}

/// `cortex daemon --root <root>` with `home` as the user's home folder.
fn daemon_command(root: &Path, home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["daemon", "--root"])
        .arg(root)
        .env("HOME", home)
        .env("USERPROFILE", home);
    command
}

/// Whether the store of the repository at `root` holds a definition named `name`.
fn has_definition(root: &Path, name: &str) -> TestResult<bool> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    Ok(!reader
        .queries()
        .symbols_named(&Identifier::new(name)?)?
        .is_empty())
}

/// Waits until `condition` holds, for at most [`SEEN_WITHIN`].
fn wait_for(condition: impl Fn() -> TestResult<bool>) -> TestResult {
    let start = Instant::now();
    while !condition()? {
        if start.elapsed() > SEEN_WITHIN {
            return Err("the condition did not hold in time".into());
        }
        std::thread::sleep(POLL_EVERY);
    }
    Ok(())
}

/// The daemon indexes the one file, says it is ready, and indexes the file again once it changes;
/// meanwhile a second daemon on the repository is refused, since the first holds the writer.
#[test]
fn indexes_watches_and_holds_the_writer() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let app = repo.path().join("app.py");
    std::fs::write(&app, "def first():\n    pass\n")?;
    let daemon = Daemon::start(repo.path(), home.path())?;
    assert_eq!(
        daemon.first_line()?,
        "cortex: daemon ready: mode tiny; files: 1 written, 0 unchanged"
    );
    assert!(has_definition(repo.path(), "first")?);
    std::fs::write(&app, "def first():\n    pass\n\ndef second():\n    pass\n")?;
    wait_for(|| has_definition(repo.path(), "second"))?;
    let refused = daemon_command(repo.path(), home.path()).output()?;
    let stderr = String::from_utf8(refused.stderr)?;
    assert_eq!(refused.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("cortex: error: opening the store in ")
            && stderr.contains("another writer holds"),
        "{stderr}"
    );
    Ok(())
}

/// With the mode pinned to huge in the repository's configuration, the first index takes
/// inventory of both files and parses neither; the crawler parses them after the ready line.
#[test]
fn huge_mode_crawls_what_it_took_inventory_of() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::create_dir(repo.path().join(".cortex"))?;
    std::fs::write(repo.path().join(".cortex/config.toml"), "mode = \"huge\"\n")?;
    std::fs::write(repo.path().join("a.py"), "def alpha():\n    pass\n")?;
    std::fs::write(repo.path().join("b.py"), "def beta():\n    pass\n")?;
    let daemon = Daemon::start(repo.path(), home.path())?;
    assert_eq!(
        daemon.first_line()?,
        "cortex: daemon ready: mode huge; files: 2 written, 0 unchanged, 2 waiting to be parsed"
    );
    wait_for(|| Ok(has_definition(repo.path(), "alpha")? && has_definition(repo.path(), "beta")?))?;
    Ok(())
}

/// Writes 3,000 Python files into `root`, each defining one function named `prefix` and its
/// number.
fn write_modules(root: &Path, prefix: &str) -> TestResult {
    for number in 0..3000 {
        let source = format!(
            "def {prefix}{number}():
    pass
"
        );
        std::fs::write(root.join(format!("m{number}.py")), source)?;
    }
    Ok(())
}

/// An index killed partway, after a complete one, leaves the index marked unfinished, and the next
/// daemon writes every file again; were the earlier index's mark kept, the files the killed index
/// wrote would be kept as they are. The first daemon completes an index of 3,000 files and stops.
/// All of them change; the second daemon is killed once its index has written its first batch, a
/// dozen batches before the end.
#[test]
fn an_index_killed_partway_is_redone_whole() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    write_modules(repo.path(), "f")?;
    let mut complete = Daemon::start(repo.path(), home.path())?;
    assert_eq!(
        complete.first_line()?,
        "cortex: daemon ready: mode medium; files: 3000 written, 0 unchanged"
    );
    complete.child.kill()?;
    complete.child.wait()?;
    write_modules(repo.path(), "g")?;
    let mut killed = Daemon::start(repo.path(), home.path())?;
    wait_for(|| has_definition(repo.path(), "g0"))?;
    killed.child.kill()?;
    killed.child.wait()?;
    let status = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(["status", "--root"])
        .arg(repo.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()?;
    let stdout = String::from_utf8(status.stdout)?;
    assert!(
        stdout.contains(
            "
index: not ready, the last index did not finish; the next one reads every file again
"
        ),
        "{stdout}"
    );
    let redone = Daemon::start(repo.path(), home.path())?;
    assert_eq!(
        redone.first_line()?,
        "cortex: daemon ready: mode medium; files: 3000 written, 0 unchanged"
    );
    Ok(())
}
