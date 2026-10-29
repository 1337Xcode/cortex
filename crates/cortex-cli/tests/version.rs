//! `cortex version` and `cortex --version` run the built binary and print its version.

use std::error::Error;
use std::process::{Command, ExitStatus};

/// The version the workspace manifest declares; a version bump updates this literal on purpose.
const EXPECTED_VERSION_LINE: &str = "cortex 2.0.0-dev\n";

/// Result type for these tests: any I/O or decoding failure fails the test.
type TestResult<T> = Result<T, Box<dyn Error>>;

/// Runs the built `cortex` binary with `args` and returns its exit status and stdout.
fn run_cortex(args: &[&str]) -> TestResult<(ExitStatus, String)> {
    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(args)
        .output()?;
    Ok((output.status, String::from_utf8(output.stdout)?))
}

/// The `version` command prints exactly one line with the name and version.
#[test]
fn version_command_prints_name_and_version() -> TestResult<()> {
    let (status, stdout) = run_cortex(&["version"])?;
    assert!(status.success());
    assert_eq!(stdout, EXPECTED_VERSION_LINE);
    Ok(())
}

/// The `--version` flag prints the same line as the `version` command.
#[test]
fn version_flag_prints_name_and_version() -> TestResult<()> {
    let (status, stdout) = run_cortex(&["--version"])?;
    assert!(status.success());
    assert_eq!(stdout, EXPECTED_VERSION_LINE);
    Ok(())
}

/// An unknown command is a usage error: non-zero exit and nothing on stdout.
#[test]
fn unknown_command_fails_with_usage_error() -> TestResult<()> {
    let (status, stdout) = run_cortex(&["no-such-command"])?;
    assert!(!status.success());
    assert_eq!(stdout, "");
    Ok(())
}
