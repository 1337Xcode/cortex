//! The built `cortex` binary: its version output and its usage text.

use std::error::Error;
use std::process::{Command, ExitStatus};

/// The version the workspace manifest declares; a version bump updates this literal on purpose.
const EXPECTED_VERSION_LINE: &str = "cortex 2.0.0-dev\n";

/// The usage line clap prints; the binary is named `cortex` whatever its file is called.
const EXPECTED_USAGE_LINE: &str = "Usage: cortex <COMMAND>";

/// Result type for these tests: any I/O or decoding failure fails the test.
type TestResult<T> = Result<T, Box<dyn Error>>;

/// What one run of the binary produced.
struct Run {
    /// How the process exited.
    status: ExitStatus,
    /// Everything written to stdout.
    stdout: String,
    /// Everything written to stderr.
    stderr: String,
}

/// Runs the built `cortex` binary with `args`.
fn run_cortex(args: &[&str]) -> TestResult<Run> {
    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(args)
        .output()?;
    Ok(Run {
        status: output.status,
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

/// The `version` command prints exactly one line with the name and version.
#[test]
fn version_command_prints_name_and_version() -> TestResult<()> {
    let run = run_cortex(&["version"])?;
    assert!(run.status.success());
    assert_eq!(run.stdout, EXPECTED_VERSION_LINE);
    Ok(())
}

/// The `--version` flag prints the same line as the `version` command.
#[test]
fn version_flag_prints_name_and_version() -> TestResult<()> {
    let run = run_cortex(&["--version"])?;
    assert!(run.status.success());
    assert_eq!(run.stdout, EXPECTED_VERSION_LINE);
    Ok(())
}

/// An unknown command is a usage error: non-zero exit, nothing on stdout, the usage on stderr.
#[test]
fn unknown_command_fails_with_usage_error() -> TestResult<()> {
    let run = run_cortex(&["no-such-command"])?;
    assert!(!run.status.success());
    assert_eq!(run.stdout, "");
    assert!(
        run.stderr.lines().any(|line| line == EXPECTED_USAGE_LINE),
        "{}",
        run.stderr
    );
    Ok(())
}

/// Help names the binary `cortex` on every platform, never the file name (`cortex.exe`).
#[test]
fn help_names_the_binary_cortex() -> TestResult<()> {
    let run = run_cortex(&["--help"])?;
    assert!(run.status.success());
    assert!(
        run.stdout.lines().any(|line| line == EXPECTED_USAGE_LINE),
        "{}",
        run.stdout
    );
    Ok(())
}
