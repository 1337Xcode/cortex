//! `cortex note` and `cortex facts` through the binary, against a real daemon: facts noted on a
//! file and on a symbol, a note said again, the list, a correction, a retirement, a fact turned
//! suspect by an edit of its file and confirmed again, and each refusal in the user's words.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Daemon, Printed, TestResult, graph_of, isolate, printed, wait_for};

/// `src/money.py`: the class `Money`.
const MONEY: &str = "class Money:\n    cents = 0\n";

/// `src/tax.py`: the function `rate`.
const TAX: &str = "def rate():\n    return 1\n";

/// Runs `cortex` with `args` in the repository at `root`, `home` being the home folder.
fn cortex(root: &Path, home: &Path, args: &[&str]) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).current_dir(root);
    isolate(&mut command, home);
    printed(&mut command, b"")
}

/// What `cortex` printed with `args`, having succeeded.
fn said(root: &Path, home: &Path, args: &[&str]) -> TestResult<String> {
    let printed = cortex(root, home, args)?;
    if !printed.is_success {
        return Err(format!("cortex {args:?} failed: {}", printed.stderr).into());
    }
    Ok(printed.stdout)
}

/// The error `cortex` printed with `args`, having failed.
fn refused(root: &Path, home: &Path, args: &[&str]) -> TestResult<String> {
    let printed = cortex(root, home, args)?;
    if printed.is_success {
        return Err(format!("cortex {args:?} succeeded: {}", printed.stdout).into());
    }
    Ok(printed.stderr)
}

/// Worked by hand. Fact 1 on `Money`, fact 2 a pitfall on `tax.py`; the first said again is
/// fact 1 again. The list: both active, in number order, each with its kind, text and anchor.
/// Fact 2 corrected becomes fact 3; fact 1 retired. The list: fact 3 alone; with `--all`, fact 1
/// retired too. Fact 4 is about the whole repository, anchored to its root (`.`). `tax.py` changes:
/// fact 3 turns suspect; confirmed, it is active again.
#[test]
fn facts_are_noted_listed_corrected_retired_and_confirmed() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(root.join("src/money.py"), MONEY)?;
    std::fs::write(root.join("src/tax.py"), TAX)?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some()))?;
    let home = home.path();

    let cents = [
        "note",
        "Money is kept in cents.",
        "--at",
        "src/money.py:Money",
    ];
    assert_eq!(said(root, home, &cents)?, "fact 1 added\n");
    let half_up = [
        "note",
        "Tax rounds half up.",
        "--at",
        "src/tax.py",
        "--kind",
        "pitfall",
    ];
    assert_eq!(said(root, home, &half_up)?, "fact 2 added\n");
    assert_eq!(said(root, home, &cents)?, "fact 1 says this already\n");
    assert_eq!(
        said(root, home, &["facts", "list"])?,
        "   1  active     convention  Money is kept in cents.  (at src/money.py:Money)\n   \
         2  active     pitfall     Tax rounds half up.  (at src/tax.py)\n"
    );

    let edit = ["facts", "edit", "2", "Tax rounds half to even."];
    assert_eq!(
        said(root, home, &edit)?,
        "fact 2 corrected; the correction is fact 3\n"
    );
    assert_eq!(
        said(root, home, &["facts", "retire", "1"])?,
        "fact 1 retired\n"
    );
    let even = "   3  active     pitfall     Tax rounds half to even.  (at src/tax.py)\n";
    assert_eq!(said(root, home, &["facts", "list"])?, even);
    assert_eq!(
        said(root, home, &["facts", "list", "--all"])?,
        format!(
            "   1  retired    convention  Money is kept in cents.  (at src/money.py:Money)\n{even}"
        )
    );

    let whole = ["note", "Every module has tests.", "--at", "."];
    assert_eq!(said(root, home, &whole)?, "fact 4 added\n");
    let whole_line = "   4  active     convention  Every module has tests.  (at .)\n";
    assert_eq!(
        said(root, home, &["facts", "list"])?,
        format!("{even}{whole_line}")
    );
    std::fs::write(root.join("src/tax.py"), "def rate():\n    return 2\n")?;
    wait_for(|| Ok(said(root, home, &["facts", "list"])?.contains("3  suspect")))?;
    assert_eq!(
        said(root, home, &["facts", "confirm", "3"])?,
        "fact 3 confirmed\n"
    );
    assert_eq!(
        said(root, home, &["facts", "list"])?,
        format!("{even}{whole_line}")
    );
    Ok(())
}

/// Each refusal says what is wrong: a path that is not a file, a symbol the file does not define,
/// a file outside the repository, a text one character too long, a number no fact has, a fact
/// corrected since, a kind Cortex does not know, a folder other than the root, and the root named
/// with a file. None of them adds a fact.
#[test]
fn a_note_or_a_word_that_names_something_wrongly_is_refused() -> TestResult {
    let outside = tempfile::tempdir()?;
    std::fs::write(outside.path().join("elsewhere.py"), TAX)?;
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(root.join("src/money.py"), MONEY)?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some()))?;
    let home = home.path();

    let note = |at: &str, text: &str| -> TestResult<String> {
        refused(root, home, &["note", text, "--at", at])
    };
    let rule = "Money is kept in cents.";
    assert!(note("src/missing.py", rule)?.contains(
        "src/missing.py: a fact is about a file, or a symbol in one, and this is neither"
    ));
    assert!(note("src/money.py:Nope", rule)?.contains("src/money.py defines no symbol Nope"));
    let elsewhere = outside.path().join("elsewhere.py");
    let elsewhere = elsewhere.to_str().ok_or("a path that is not text")?;
    assert!(note(elsewhere, rule)?.contains("is not inside the repository"));
    let longer = "e".repeat(241);
    assert!(
        note("src/money.py", &longer)?
            .contains("a fact holds at most 240 characters; this one holds 241")
    );
    assert!(refused(root, home, &["facts", "confirm", "9"])?.contains("there is no fact 9"));
    assert_eq!(
        said(root, home, &["note", rule, "--at", "src/money.py"])?,
        "fact 1 added\n"
    );
    said(
        root,
        home,
        &["facts", "edit", "1", "Money is kept in whole cents."],
    )?;
    assert!(
        refused(root, home, &["facts", "retire", "1"])?.contains("fact 1 was corrected by fact 2")
    );
    assert!(
        refused(
            root,
            home,
            &["note", rule, "--at", "src/money.py", "--kind", "rule"]
        )?
        .contains("rule is not a kind of fact")
    );
    assert!(
        refused(root, home, &["note", rule, "--at", "src"])?.contains("this is another folder")
    );
    assert!(
        refused(
            root,
            home,
            &["note", rule, "--at", ".", "--at", "src/money.py"]
        )?
        .contains("a fact about the whole repository is anchored to it alone")
    );
    assert_eq!(
        said(root, home, &["facts", "list", "--all"])?,
        "   2  active     convention  Money is kept in whole cents.  (at src/money.py)\n"
    );
    Ok(())
}
