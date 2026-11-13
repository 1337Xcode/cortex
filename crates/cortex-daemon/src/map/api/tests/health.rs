//! The health endpoint's daemon part read through the site: what it takes from the daemon's status
//! report, and none when no server gives one or the server is gone.

use std::sync::Arc;

use cortex_core::ipc::{
    CORTEX_VERSION, GitReport, GitState, GraphReport, HistoryReport, HookLatency, Protocol,
    RecentError, StatusReport, UpkeepReport,
};
use serde_json::Value;

use super::{get, site_reporting};
use crate::map::Status;
use crate::test_store::TestResult;

/// A daemon whose status report is fixed.
#[derive(Debug)]
struct Fixed(StatusReport);

impl Status for Fixed {
    fn status_report(&self) -> StatusReport {
        self.0.clone()
    }
}

/// An error the daemon met at `at_ms`, `count` times, saying `message`.
fn error(at_ms: i64, count: u64, message: &str) -> RecentError {
    RecentError {
        at_ms,
        count,
        message: message.to_owned(),
    }
}

/// A daemon up for 90 s that handled 12 prompts, kept 12 errors (the oldest at 1,000 ms, one more
/// every 1,000 ms), dropped 3 events and failed 1 batch, whose miner is ready, whose graph holds 40
/// nodes and 55 links, whose upkeep last ran at 5,000 ms and whose history worker runs.
fn report() -> StatusReport {
    StatusReport {
        cortex_version: CORTEX_VERSION.to_owned(),
        protocol: Protocol::CURRENT,
        pid: 7,
        uptime_ms: 90_000,
        events_waiting: 0,
        changes_waiting: 0,
        events_dropped: 3,
        failed_batches: 1,
        failed_checkpoints: 0,
        probes_answered: 0,
        is_indexed: true,
        latencies: vec![HookLatency {
            hook: "prompt".to_owned(),
            requests: 12,
            p50_us: 900,
            p95_us: 4_000,
            p99_us: 6_500,
        }],
        recent_errors: (1..=12)
            .map(|n| error(n * 1_000, 1, &format!("error {n}")))
            .collect(),
        git: Some(GitReport {
            state: GitState::Ready,
            detail: None,
        }),
        graph: Some(GraphReport {
            nodes: 40,
            links: 55,
            has_cochange: true,
            builds: 2,
            build_ms: 30,
            failed_builds: 0,
            is_stale: false,
        }),
        worktree: None,
        linked_worktrees: Vec::new(),
        upkeep: Some(UpkeepReport {
            at_ms: 5_000,
            links: 0,
            interest: 0,
            facts: 0,
            events: 0,
            episodes: 0,
            positions: 0,
            pages: 0,
            is_done: true,
        }),
        history: Some(HistoryReport::default()),
    }
}

/// The health answer of a site whose status comes from `status`, as JSON.
fn health(status: Option<&Arc<dyn Status>>) -> TestResult<Value> {
    let root = tempfile::tempdir()?;
    let fixture = site_reporting(
        root,
        (Arc::default(), Arc::default()),
        status.map(Arc::downgrade),
    )?;
    let (code, body) = get(&fixture, "api/health")?;
    assert_eq!(code, 200, "{body}");
    Ok(serde_json::from_str(&body)?)
}

/// The daemon part is the report's: its uptime, the prompt's handling times, the ten newest of its
/// twelve errors newest first (12 down to 3), the drops and failed batches, the miner ready, the
/// graph's 40 nodes and 55 links, upkeep at 5,000 ms and the history worker on.
#[test]
fn the_health_shows_the_daemon_s_status() -> TestResult {
    let status: Arc<dyn Status> = Arc::new(Fixed(report()));
    let daemon = &health(Some(&status))?["daemon"];
    let errors: Vec<String> = (3..=12).rev().map(|n| format!("error {n}")).collect();
    let expected = serde_json::json!({
        "uptime_ms": 90_000,
        "hooks": [{"hook": "prompt", "requests": 12, "p50_us": 900, "p95_us": 4_000, "p99_us": 6_500}],
        "errors": errors
            .iter()
            .zip((3..=12_i64).rev())
            .map(|(message, n)| serde_json::json!({"at_ms": n * 1_000, "count": 1, "message": message}))
            .collect::<Vec<_>>(),
        "events_dropped": 3,
        "failed_batches": 1,
        "git": "ready",
        "git_detail": null,
        "graph_nodes": 40,
        "graph_links": 55,
        "upkeep_at_ms": 5_000,
        "history": "on",
    });
    assert_eq!(daemon, &expected);
    Ok(())
}

/// With no server, or once the server is gone, the daemon part is null and the rest still answers.
#[test]
fn the_health_says_nothing_of_a_daemon_it_cannot_ask() -> TestResult {
    assert_eq!(health(None)?["daemon"], Value::Null);
    let status: Arc<dyn Status> = Arc::new(Fixed(report()));
    let root = tempfile::tempdir()?;
    let fixture = site_reporting(
        root,
        (Arc::default(), Arc::default()),
        Some(Arc::downgrade(&status)),
    )?;
    drop(status);
    let (code, body) = get(&fixture, "api/health")?;
    assert_eq!(code, 200, "{body}");
    let value: Value = serde_json::from_str(&body)?;
    assert_eq!(value["daemon"], Value::Null);
    assert_eq!(value["index"], "reading");
    Ok(())
}
