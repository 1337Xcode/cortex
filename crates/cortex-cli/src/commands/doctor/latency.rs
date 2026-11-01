//! The check on hook latency, through probes: this binary run as Claude Code runs a hook, 20 times
//! for each event a budget names, on a payload marked as a probe, which the daemon answers and
//! counts and never records; each event's 95th percentile, from the process's start to its exit,
//! against its budget. A probe starts and stops no daemon, so the probes run only while a daemon
//! of this build answers for the repository, and the daemon's count of the probes it answered
//! shows that they reached it.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use cortex_core::ipc::{CORTEX_VERSION, RequestBody, ResponseBody, Role, StatusReport};
use cortex_core::{RepoId, UserFolder, one_line};
use serde_json::{Map, Value, json};

use super::{Context, Finding};

/// Probes per event: twenty make the 95th percentile the second slowest, not the slowest.
const PROBES: usize = 20;

/// How long one probe may take before it is counted as failing: past any hook's hard timeout.
const PROBE_WITHIN: Duration = Duration::from_secs(5);

/// How long the daemon has to answer the status doctor asks before and after the probes.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);

/// The fix when the daemon stopped or changed while the probes ran.
const SEE_DAEMON_LOG: &str = "see why in the daemon's log in ~/.cortex/logs, then run doctor again";

/// One event the probes run for.
struct Probe {
    /// What it is, for the printed line.
    what: &'static str,
    /// The event's name, as Claude Code and `cortex hook --event` give it.
    event: &'static str,
    /// The tool, and the file it aims at in the repository's root folder, for a tool's event.
    tool: Option<(&'static str, &'static str)>,
    /// The most its 95th percentile may take.
    budget: Duration,
}

/// The events the probes run for, with their budgets from start to exit.
const PROBED: [Probe; 5] = [
    Probe {
        what: "SessionStart",
        event: "SessionStart",
        tool: None,
        budget: Duration::from_millis(120),
    },
    Probe {
        what: "UserPromptSubmit",
        event: "UserPromptSubmit",
        tool: None,
        budget: Duration::from_millis(150),
    },
    Probe {
        what: "PreToolUse on a read",
        event: "PreToolUse",
        tool: Some(("Read", "cortex-doctor-probe.txt")),
        budget: Duration::from_millis(40),
    },
    Probe {
        what: "PostToolUse on an edit",
        event: "PostToolUse",
        tool: Some(("Edit", "cortex-doctor-probe.txt")),
        budget: Duration::from_millis(80),
    },
    Probe {
        what: "Stop",
        event: "Stop",
        tool: None,
        budget: Duration::from_millis(60),
    },
];

/// Each probed event's 95th percentile is within its budget, and every probe reached the daemon.
pub(super) fn probes(context: &Context) -> Vec<Finding> {
    let (before, root) = match measurable(context) {
        Ok(measurable) => measurable,
        Err(why) => return vec![Finding::Note(format!("not measured: {why}"))],
    };
    let mut findings = Vec::new();
    let mut sent = 0;
    for probe in &PROBED {
        let payload = payload(probe, root).to_string();
        let mut times = Vec::with_capacity(PROBES);
        let ran = timed(&context.program, probe.event, &payload, &mut times);
        sent += times.len();
        findings.push(match ran {
            Ok(()) => judged(probe, &mut times),
            Err(why) => Finding::Fail {
                what: format!("a {} probe failed: {why}", probe.what),
                fix: "run `cortex hook` by hand to see it fail, and see \
                      ~/.cortex/logs/hook.log"
                    .to_owned(),
            },
        });
    }
    findings.extend(unreached(&before, status(&context.root).as_ref(), sent));
    findings
}

/// The daemon's status before the probes and the repository's root as a payload carries it, when
/// the probes can measure: a daemon of the repository answers, it runs this build, and the root is
/// text. Otherwise, why not.
fn measurable(context: &Context) -> Result<(StatusReport, &str), String> {
    let before = status(&context.root)
        .ok_or_else(|| "no daemon answers, and probes start none".to_owned())?;
    if before.cortex_version != CORTEX_VERSION {
        return Err(format!(
            "the daemon runs Cortex {}, not this build's {CORTEX_VERSION}",
            one_line(&before.cortex_version, 40)
        ));
    }
    let root = context.root.to_str().ok_or_else(|| {
        "the repository's path is not UTF-8, which no hook payload can carry".to_owned()
    })?;
    Ok((before, root))
}

/// Runs [`PROBES`] probes of `event` on `payload` with `program`, keeping how long each took in
/// `times`, and stops at the first that fails, saying why.
fn timed(
    program: &str,
    event: &str,
    payload: &str,
    times: &mut Vec<Duration>,
) -> Result<(), String> {
    for _ in 0..PROBES {
        times.push(run(program, event, payload)?);
    }
    Ok(())
}

/// The finding for `probe` from the `times` its runs took.
fn judged(probe: &Probe, times: &mut [Duration]) -> Finding {
    let measured = p95(times);
    let line = format!(
        "{}: p95 {:.1} ms over {} probes, the budget {} ms",
        probe.what,
        measured.as_secs_f64() * 1_000.0,
        times.len(),
        probe.budget.as_millis()
    );
    if measured <= probe.budget {
        Finding::Pass(line)
    } else {
        Finding::Fail {
            what: line,
            fix: "see `cortex status` for the daemon's own time per hook, and its log in \
                  ~/.cortex/logs; a slow disk or a busy machine slows every hook"
                .to_owned(),
        }
    }
}

/// The 95th percentile of `times` by nearest rank; zero for none.
fn p95(times: &mut [Duration]) -> Duration {
    times.sort_unstable();
    let rank = (times.len() * 95).div_ceil(100).max(1);
    times.get(rank - 1).copied().unwrap_or_default()
}

/// A failure when the daemon that answered `before` the probes did not take all `sent` of them,
/// as its status `after` them shows: it stopped answering, another took its place, or fewer
/// probes arrived than were sent.
fn unreached(before: &StatusReport, after: Option<&StatusReport>, sent: usize) -> Option<Finding> {
    let Some(after) = after else {
        return Some(Finding::Fail {
            what: "the daemon stopped answering while the probes ran".to_owned(),
            fix: SEE_DAEMON_LOG.to_owned(),
        });
    };
    if after.pid != before.pid {
        return Some(Finding::Fail {
            what: format!(
                "the daemon was replaced while the probes ran (process {} became {})",
                before.pid, after.pid
            ),
            fix: SEE_DAEMON_LOG.to_owned(),
        });
    }
    let arrived = after.probes_answered.saturating_sub(before.probes_answered);
    let sent = u64::try_from(sent).unwrap_or(u64::MAX);
    (arrived < sent).then(|| Finding::Fail {
        what: format!("{arrived} of the {sent} probes reached the daemon"),
        fix: "the hook did not find this repository's daemon: see ~/.cortex/logs/hook.log"
            .to_owned(),
    })
}

/// The daemon's status, when a daemon of the repository at `root` answers it.
fn status(root: &Path) -> Option<StatusReport> {
    let user = UserFolder::of_current_user()?;
    let files = user.run_files(&RepoId::of(root));
    let answer = cortex_hook::ask(
        &files,
        Role::Cli,
        RequestBody::Status,
        Instant::now() + ANSWER_WITHIN,
    )
    .ok()?;
    match answer.body {
        ResponseBody::Status(report) => Some(report),
        _ => None,
    }
}

/// The payload of `probe` from the repository whose root is `root`, marked as a probe.
fn payload(probe: &Probe, root: &str) -> Value {
    let mut fields = Map::new();
    fields.insert("session_id".to_owned(), json!("cortex-doctor-probe"));
    fields.insert("cwd".to_owned(), json!(root));
    fields.insert("hook_event_name".to_owned(), json!(probe.event));
    fields.insert("cortex_probe".to_owned(), json!(true));
    if probe.event == "UserPromptSubmit" {
        fields.insert("prompt".to_owned(), json!("cortex doctor's probe"));
    }
    if let Some((tool, file)) = probe.tool {
        let file = Path::new(root).join(file);
        fields.insert("tool_name".to_owned(), json!(tool));
        fields.insert(
            "tool_input".to_owned(),
            json!({ "file_path": file.to_string_lossy() }),
        );
    }
    Value::Object(fields)
}

/// Runs `program hook --agent claude --event <event>` on `payload` and returns how long it took
/// from its start to its exit.
fn run(program: &str, event: &str, payload: &str) -> Result<Duration, String> {
    let mut command = Command::new(program);
    command.args(["hook", "--agent", "claude", "--event", event]);
    let (status, took) = cortex_daemon::time_fed(&mut command, payload.as_bytes(), PROBE_WITHIN)
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(took)
    } else {
        Err(format!("the hook ended with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::ipc::Protocol;

    use super::*;

    /// Twenty times of 1 to 20 ms, in any order: the 95th percentile by nearest rank is the 19th,
    /// 19 ms.
    #[test]
    fn the_95th_percentile_of_twenty_is_the_19th() {
        let mut times: Vec<Duration> = (1..=20).rev().map(Duration::from_millis).collect();
        assert_eq!(p95(&mut times), Duration::from_millis(19));
    }

    /// Of twenty reads, nineteen of 10 ms and one of 50 ms leave the 95th percentile, the 19th, at
    /// 10 ms, within the 40 ms budget; a second of 50 ms puts it at 50 ms, over the budget.
    #[test]
    fn a_95th_percentile_over_its_budget_fails() {
        let [_, _, read, ..] = &PROBED;
        let slow = |count: usize| {
            let mut times = vec![Duration::from_millis(10); PROBES - count];
            times.extend(vec![Duration::from_millis(50); count]);
            times
        };
        assert_eq!(
            judged(read, &mut slow(1)),
            Finding::Pass(
                "PreToolUse on a read: p95 10.0 ms over 20 probes, the budget 40 ms".to_owned()
            )
        );
        let over = judged(read, &mut slow(2));
        assert!(
            matches!(&over, Finding::Fail { what, .. }
                if what == "PreToolUse on a read: p95 50.0 ms over 20 probes, the budget 40 ms"),
            "{over:?}"
        );
    }

    /// A probe's payload names the repository, the event and the probe marker, and a tool's names
    /// the tool and a file in the repository's root folder.
    #[test]
    fn payloads_are_marked_probes() {
        let [_, prompt, read, ..] = &PROBED;
        assert_eq!(
            payload(prompt, "/repo"),
            json!({
                "session_id": "cortex-doctor-probe",
                "cwd": "/repo",
                "hook_event_name": "UserPromptSubmit",
                "cortex_probe": true,
                "prompt": "cortex doctor's probe",
            })
        );
        let read = payload(read, "/repo");
        assert_eq!(read.get("tool_name"), Some(&json!("Read")));
        let file = read
            .get("tool_input")
            .and_then(|input| input.get("file_path"))
            .and_then(Value::as_str)
            .map(Path::new);
        assert_eq!(file.and_then(Path::parent), Some(Path::new("/repo")));
        assert_eq!(
            file.and_then(Path::file_name),
            Some("cortex-doctor-probe.txt".as_ref())
        );
    }

    /// A report from daemon process `pid`, which answered `probes` probes.
    fn report(pid: u32, probes: u64) -> StatusReport {
        StatusReport {
            cortex_version: CORTEX_VERSION.to_owned(),
            protocol: Protocol::CURRENT,
            pid,
            uptime_ms: 0,
            events_waiting: 0,
            changes_waiting: 0,
            events_dropped: 0,
            failed_batches: 0,
            failed_checkpoints: 0,
            probes_answered: probes,
            latencies: Vec::new(),
            recent_errors: Vec::new(),
        }
    }

    /// What a finding says is wrong, when it is a failure.
    fn failure(finding: Option<Finding>) -> Option<String> {
        match finding {
            Some(Finding::Fail { what, .. }) => Some(what),
            _ => None,
        }
    }

    /// 100 probes sent to a daemon whose count went from 3 to 103 all arrived; to 98, five did
    /// not; a daemon that stopped answering, or another process in its place, fails whatever the
    /// counts say.
    #[test]
    fn probes_that_did_not_reach_the_daemon_fail() {
        assert_eq!(unreached(&report(7, 3), Some(&report(7, 103)), 100), None);
        assert_eq!(
            failure(unreached(&report(7, 3), Some(&report(7, 98)), 100)).as_deref(),
            Some("95 of the 100 probes reached the daemon")
        );
        assert_eq!(
            failure(unreached(&report(7, 3), None, 100)).as_deref(),
            Some("the daemon stopped answering while the probes ran")
        );
        assert_eq!(
            failure(unreached(&report(7, 3), Some(&report(8, 103)), 100)).as_deref(),
            Some("the daemon was replaced while the probes ran (process 7 became 8)")
        );
    }
}
