//! Git repositories the tests script: git run with the test's home folder as the user's, and
//! commits made by a fixed author.

use std::path::Path;
use std::process::Command;

use super::{TestResult, isolate};

/// Runs git with `args` in `folder`, with `home` as the user's home folder.
pub fn git(folder: &Path, home: &Path, args: &[&str]) -> TestResult {
    let mut command = Command::new("git");
    command.args(args).current_dir(folder);
    isolate(&mut command, home);
    let status = command.status()?;
    if !status.success() {
        return Err(format!("git {args:?} failed: {status}").into());
    }
    Ok(())
}

/// Commits what `paths` hold in the repository at `root` as `message`, by a fixed author, with
/// `home` as the user's home folder; returns the new HEAD.
pub fn commit(root: &Path, home: &Path, message: &str, paths: &[&str]) -> TestResult<String> {
    let mut add = vec!["add", "--"];
    add.extend_from_slice(paths);
    git(root, home, &add)?;
    let mut command = Command::new("git");
    command
        .args(["commit", "-q", "-m", message])
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "Cortex Test")
        .env("GIT_AUTHOR_EMAIL", "test")
        .env("GIT_COMMITTER_NAME", "Cortex Test")
        .env("GIT_COMMITTER_EMAIL", "test");
    isolate(&mut command, home);
    if !command.status()?.success() {
        return Err(format!("committing {message} failed").into());
    }
    let mut head = Command::new("git");
    head.args(["rev-parse", "HEAD"]).current_dir(root);
    isolate(&mut head, home);
    Ok(String::from_utf8(head.output()?.stdout)?.trim().to_owned())
}
