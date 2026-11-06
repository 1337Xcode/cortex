//! The reason strict mode gives a stop to go on, from a store built here: the files that call a
//! signature the task changed and that it has not read or edited since the warning, the latest
//! warned of first, each once; nothing once they are all followed up, or when no warning is of a
//! call site; never past its cap. Every expected line is typed out by hand.

mod common;

use common::task::{acted, file, task, warned};
use common::{Repo, TestResult};
use cortex_core::{ToolKind, WarningKind, estimate_tokens};
use cortex_retrieve::{MAX_STOP_REASON_TOKENS, stop_reason};
use cortex_store::NodeId;

/// The reason task `task` gets.
fn reason(repo: &Repo, task: cortex_core::Ulid) -> TestResult<Option<String>> {
    let reader = repo.reader()?;
    Ok(stop_reason(reader.queries(), task)?)
}

/// The edit of `calc/ops.py` at 10 changed `add`, called from `main.py` and the tests: two
/// call-site warnings at 10, then the tests' own warning. Both callers are open, the later warned
/// of first, the tests named once though warned of twice. Read at 20, `main.py` is followed up;
/// edited at 30, the tests are too, and there is no reason left. A read at 5, before the
/// warnings, and a failed edit follow nothing up.
#[test]
fn a_stop_is_given_the_callers_not_followed_up() -> TestResult {
    let mut repo = Repo::new()?;
    let task = task(&mut repo, None)?;
    let (ops, add) = file(&mut repo, "calc/ops.py")?;
    let (main, _) = file(&mut repo, "main.py")?;
    let (tests, _) = file(&mut repo, "tests/test_ops.py")?;
    acted(
        &mut repo,
        task,
        (ToolKind::Read, "main.py", main),
        (5, true),
    )?;
    acted(
        &mut repo,
        task,
        (ToolKind::Edit, "calc/ops.py", ops),
        (10, true),
    )?;
    warned(&mut repo, task, (WarningKind::CallSite, add, main), 10)?;
    warned(&mut repo, task, (WarningKind::CallSite, add, tests), 10)?;
    warned(&mut repo, task, (WarningKind::Test, ops, tests), 10)?;
    acted(
        &mut repo,
        task,
        (ToolKind::Edit, "tests/test_ops.py", tests),
        (15, false),
    )?;
    assert_eq!(
        reason(&repo, task)?.as_deref(),
        Some(
            "[cortex] Strict mode: these files call a signature this task changed and were not \
             read or edited since the warning: tests/test_ops.py, main.py.\n"
        )
    );
    acted(
        &mut repo,
        task,
        (ToolKind::Read, "main.py", main),
        (20, true),
    )?;
    assert_eq!(
        reason(&repo, task)?.as_deref(),
        Some(
            "[cortex] Strict mode: these files call a signature this task changed and were not \
             read or edited since the warning: tests/test_ops.py.\n"
        )
    );
    acted(
        &mut repo,
        task,
        (ToolKind::Edit, "tests/test_ops.py", tests),
        (30, true),
    )?;
    assert_eq!(reason(&repo, task)?, None);
    Ok(())
}

/// A task warned only of its tests and of a file that changes with it gives no reason: strict mode
/// holds a stop on call sites alone.
#[test]
fn only_call_sites_hold_a_stop() -> TestResult {
    let mut repo = Repo::new()?;
    let task = task(&mut repo, None)?;
    let (ops, _) = file(&mut repo, "calc/ops.py")?;
    let (tests, _) = file(&mut repo, "tests/test_ops.py")?;
    let (docs, _) = file(&mut repo, "docs/ops.md")?;
    acted(
        &mut repo,
        task,
        (ToolKind::Edit, "calc/ops.py", ops),
        (10, true),
    )?;
    warned(&mut repo, task, (WarningKind::Test, ops, tests), 10)?;
    warned(&mut repo, task, (WarningKind::CoChange, ops, docs), 10)?;
    assert_eq!(reason(&repo, task)?, None);
    Ok(())
}

/// A reason is held to its cap of 250 tokens, 1,000 bytes at four bytes a token: it names fewer
/// files and counts the rest. By hand: the text around the files is 118 bytes (116 before them,
/// the full stop and line break after) and " and 1 more" 11; each caller's path is 200 bytes, and
/// each ", " between two of them 2. All five: 118 + 1,000 + 8 = 1,126 bytes, 282 tokens, too
/// many; four and one more: 118 + 800 + 6 + 11 = 935 bytes, 234 tokens. So four are named, the
/// latest warned of first, and one more counted.
#[test]
fn a_reason_is_held_to_its_cap() -> TestResult {
    let mut repo = Repo::new()?;
    let task = task(&mut repo, None)?;
    let (ops, add) = file(&mut repo, "calc/ops.py")?;
    acted(
        &mut repo,
        task,
        (ToolKind::Edit, "calc/ops.py", ops),
        (10, true),
    )?;
    let mut paths: Vec<String> = Vec::new();
    let mut callers: Vec<NodeId> = Vec::new();
    for at in 0..5_u8 {
        let path = format!("{}{}.py", char::from(b'k' + at), "c".repeat(196));
        let (caller, _) = file(&mut repo, &path)?;
        warned(&mut repo, task, (WarningKind::CallSite, add, caller), 10)?;
        paths.push(path);
        callers.push(caller);
    }
    let given = reason(&repo, task)?.ok_or("a reason")?;
    assert!(estimate_tokens(&given) <= MAX_STOP_REASON_TOKENS);
    assert_eq!(given.len(), 935, "{given}");
    let latest_four: Vec<&str> = paths.iter().rev().take(4).map(String::as_str).collect();
    assert!(given.contains(&latest_four.join(", ")), "{given}");
    assert!(given.ends_with(" and 1 more.\n"), "{given}");
    Ok(())
}
