//! A file panel's history read through the site: how many commits it names when the blames name
//! more than it shows, and whether it says the history worker runs, is off, or is refused.

use std::sync::Arc;

use cortex_core::{ContentHash, RepoPath};
use cortex_store::{BlameRecord, BlameRunRecord};
use serde_json::Value;

use super::script::{card, script};
use super::{get, site_over};
use crate::test_store::{TestResult, writer};

/// How many commits the blame of `src/app.rs` names: two more than the panel lists.
const BLAMED: u32 = 12;

/// How the history worker stands when the panel is read.
#[derive(Debug, Clone, Copy)]
enum Worker {
    /// It never started.
    Off,
    /// It started.
    On,
    /// It started, and git was refused for this reason.
    Refused(&'static str),
}

/// The panel of `src/app.rs` (id 1) as JSON, its store the scripted one with a blame of that file
/// naming [`BLAMED`] commits, each of 10 lines, the commit of lines 1 to 10 the oldest, each held
/// with the subject "Change n", n its number from 1; the worker as `worker` says.
fn app_panel(worker: Worker) -> TestResult<Value> {
    let root = tempfile::tempdir()?;
    let texts = script(root.path())?;
    let oid = |n: u32| format!("{n:040}");
    let mut writer = writer(root.path())?;
    let mut transaction = writer.transaction()?;
    let runs = (1..=BLAMED)
        .map(|n| BlameRunRecord {
            start: (n - 1) * 10 + 1,
            end: n * 10,
            source_start: 1,
            oid: oid(n),
        })
        .collect();
    transaction.put_blame(&BlameRecord {
        path: RepoPath::new("src/app.rs")?,
        file_hash: ContentHash::of(b"src/app.rs"),
        first_line: 1,
        last_line: 120,
        runs,
        computed_at_ms: 1,
    })?;
    for n in 1..=BLAMED {
        transaction
            .put_change_card(&card(&oid(n), 1_700_000_000 + i64::from(n), None, &[]), &[])?;
        texts.put_subject(&oid(n), &format!("Change {n}"));
    }
    transaction.commit()?;
    drop(writer);
    match worker {
        Worker::Off => {}
        Worker::On => texts.start_for_tests(None),
        Worker::Refused(why) => texts.start_for_tests(Some(why)),
    }
    let fixture = site_over(root, Arc::clone(&texts))?;
    let (status, body) = get(&fixture, "api/file/1")?;
    assert_eq!(status, 200, "{body}");
    Ok(serde_json::from_str(&body)?)
}

/// The subjects `panel` lists, in order.
fn subjects(panel: &Value) -> Vec<&str> {
    panel["history"]
        .as_array()
        .map(|commits| {
            commits
                .iter()
                .filter_map(|commit| commit["subject"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// Of the 12 commits the blame names, the panel lists the 10 newest, 12 down to 3, and says 12.
#[test]
fn a_file_s_history_lists_the_ten_newest_and_counts_them_all() -> TestResult {
    let panel = app_panel(Worker::Off)?;
    let expected: Vec<String> = (3..=12).rev().map(|n| format!("Change {n}")).collect();
    assert_eq!(subjects(&panel), expected);
    assert_eq!(panel["history_total"], 12);
    Ok(())
}

/// The reader is `off` when the worker never started, `on` once it has, and `refused` when git
/// was refused in the repository; what is stored is listed in each case.
#[test]
fn a_file_s_panel_says_whether_history_is_read() -> TestResult {
    for (worker, reader) in [
        (Worker::Off, "off"),
        (Worker::On, "on"),
        (
            Worker::Refused("its configuration names a filter"),
            "refused",
        ),
    ] {
        let panel = app_panel(worker)?;
        assert_eq!(panel["history_reader"], reader, "{worker:?}");
        assert_eq!(panel["history_total"], 12, "{worker:?}");
    }
    Ok(())
}
