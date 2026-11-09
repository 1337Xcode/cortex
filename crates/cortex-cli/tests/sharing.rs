//! `cortex export` and `cortex import` through the binary when the shared folder is not the
//! repository's own: a `.cortex/shared` that is a link to a folder elsewhere on the machine is
//! refused by both, and nothing is read or written through it.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Daemon, Printed, TestResult, graph_of, isolate, printed, wait_for};

/// Runs `cortex` with `args` in the repository at `root`, `home` being the home folder.
fn cortex(root: &Path, home: &Path, args: &[&str]) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).current_dir(root);
    isolate(&mut command, home);
    printed(&mut command, b"")
}

/// Makes `link` a link to the folder `target`: a symbolic link, or on Windows a junction, which a
/// user may make without privileges and which git checks out from no repository, but a script in
/// one can make.
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

/// A set-up repository whose `.cortex/shared` leads to a folder elsewhere holding a shared facts
/// file: `cortex import` refuses it, naming the link, and notes no fact; `cortex export` refuses
/// to write through it, and the file elsewhere keeps its bytes.
#[test]
fn a_shared_folder_that_is_a_link_is_refused() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(root.join("src/money.py"), "class Money:\n    cents = 0\n")?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some()))?;
    let home = home.path();
    let file = "{\n  \"version\": 1,\n  \"facts\": [\n    {\n      \"text\": \"Money is cents.\",\n      \
                \"kind\": \"convention\",\n      \"at\": [\n        \"src/money.py\"\n      ]\n    }\n  \
                ]\n}\n";
    std::fs::write(elsewhere.path().join("facts.json"), file)?;
    link_folder(elsewhere.path(), &root.join(".cortex").join("shared"))?;

    let imported = cortex(root, home, &["import"])?;
    assert!(!imported.is_success);
    assert!(imported.stderr.contains("is a link"), "{}", imported.stderr);
    let listed = cortex(root, home, &["facts", "list"])?;
    assert_eq!(listed.stdout, "no facts\n");

    let noted = cortex(
        root,
        home,
        &["note", "Money is cents.", "--at", "src/money.py"],
    )?;
    assert!(noted.is_success, "{}", noted.stderr);
    let exported = cortex(root, home, &["export"])?;
    assert!(!exported.is_success);
    assert!(
        exported.stderr.contains("leads elsewhere") || exported.stderr.contains("is a link"),
        "{}",
        exported.stderr
    );
    assert_eq!(
        std::fs::read_to_string(elsewhere.path().join("facts.json"))?,
        file
    );
    Ok(())
}
