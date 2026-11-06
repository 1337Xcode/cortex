//! The writer lock across processes: a writer held by another process keeps this one from opening
//! a writer until that process lets it go.
//!
//! In a test binary of its own because it starts a process. While a process is being started, it
//! holds a copy of every file its parent has open until it runs its program, the writer locks of
//! other tests among them on some systems (macOS was seen to): a test of the same binary that
//! closed its writer and opened another at that moment would find its own lock still held.

mod common;

use common::{Scratch, TestResult};
use cortex_core::RepoId;
use cortex_store::StoreError;

/// The variable that makes [`hold_the_writer_for_another_test`] hold a writer: the `.cortex`
/// folder to open.
const HOLD_WRITER_VARIABLE: &str = "CORTEX_STORE_TEST_HOLD_WRITER";

/// The line the holding process prints once it has the writer.
const HOLDING_LINE: &str = "holding the writer";

/// Run as a child process by [`a_writer_in_another_process_is_refused`]: opens the writer, says so
/// on stdout and holds it until stdin closes. In an ordinary test run it does nothing.
#[test]
fn hold_the_writer_for_another_test() -> TestResult {
    use std::io::{Read, Write};
    let Some(folder) = std::env::var_os(HOLD_WRITER_VARIABLE) else {
        return Ok(());
    };
    let repo_id = RepoId::of(std::path::Path::new(&folder));
    let writer = cortex_store::Writer::open(&cortex_store::WriterOptions {
        folder: std::path::Path::new(&folder),
        repo_id: &repo_id,
        mmap_bytes: common::MMAP_BYTES,
        now_ms: 1,
    })?;
    writeln!(std::io::stdout(), "{HOLDING_LINE}")?;
    std::io::stdout().flush()?;
    let mut rest = Vec::new();
    std::io::stdin().read_to_end(&mut rest)?;
    drop(writer);
    Ok(())
}

/// A writer held by another process keeps this process from opening one, until that process lets
/// it go.
#[test]
fn a_writer_in_another_process_is_refused() -> TestResult {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    use std::time::Duration;
    let scratch = Scratch::new()?;
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", "hold_the_writer_for_another_test", "--nocapture"])
        .env(HOLD_WRITER_VARIABLE, &scratch.folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line == HOLDING_LINE && sender.send(()).is_err() {
                break;
            }
        }
    });
    receiver.recv_timeout(Duration::from_secs(60))?;
    let refused = scratch.writer(2).err();
    assert!(
        matches!(refused, Some(StoreError::WriterBusy { .. })),
        "{refused:?}"
    );
    drop(child.stdin.take());
    assert!(child.wait()?.success());
    scratch.writer(3)?;
    Ok(())
}
