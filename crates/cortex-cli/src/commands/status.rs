//! `cortex status`: whether the repository's daemon runs and, from the daemon itself, its queue,
//! what its co-change miner is doing, how long it takes to handle each hook and its latest errors;
//! then whether the index is ready, the scale mode, how many files are stored, parsed and waiting
//! to be parsed, and how far the history is mined. The store is only read.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use cortex_core::ipc::{
    GitReport, GitState, HookLatency, RecentError, RequestBody, ResponseBody, StatusReport,
};
use cortex_core::{FileFlags, ModeSetting, UserFolder, unix_millis};
use cortex_daemon::settings;
use cortex_git::mine::Mark;
use cortex_hook::{Answer, ClientError};
use cortex_index::waits_to_be_parsed;
use cortex_store::{MetaKey, Queries, Reader, StoreError, is_writer_running};

use crate::{output, repository, upgrade};

/// How long the daemon has to answer. It answers status from memory, so a daemon that takes longer
/// is stuck, not busy.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);

/// The arguments of `cortex status`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Prints the repository's status, one fact per line, or that it has no index.
///
/// # Errors
///
/// A root that is not a folder, an unusable configuration, a store that cannot be read, or
/// stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let loaded = settings::load(&root)?;
    for warning in &loaded.warnings {
        output::note(&format!("warning: {warning}"))?;
    }
    for line in daemon_lines(&root) {
        output::line(&line)?;
    }
    for line in index_lines(&root, loaded.config.mode)? {
        output::line(&line)?;
    }
    Ok(())
}

/// The lines saying how far the index of the repository at `root` is, with the mode `setting` the
/// configuration gives, or that it has none, or that its writer is still setting up its store.
///
/// # Errors
///
/// The store cannot be read.
pub(crate) fn index_lines(root: &Path, setting: ModeSetting) -> anyhow::Result<Vec<String>> {
    let folder = root.join(".cortex");
    let reader = match Reader::open(&folder, 0) {
        Ok(reader) => reader,
        Err(StoreError::Missing { .. }) => {
            return Ok(vec![format!(
                "no index in {}; `cortex daemon` builds one",
                root.display()
            )]);
        }
        // A daemon answers as soon as it listens, before its writer has created the store or
        // brought an older one up to this build's schema, so a status asked then finds the store
        // part made. The writer takes its lock before it touches the database file.
        Err(StoreError::SchemaNotCurrent { .. }) if is_writer_running(&folder)? => {
            return Ok(vec![
                "index: not ready, the store is being set up".to_owned(),
            ]);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading the store in {}", folder.display()));
        }
    };
    status_lines(&folder, reader.queries(), setting)
}

/// The lines saying whether the daemon of the repository at `root` runs and, from its own answer,
/// what waits for it, how long it takes to handle each hook, and its latest errors.
fn daemon_lines(root: &Path) -> Vec<String> {
    let Some(user) = UserFolder::of_current_user() else {
        return vec!["daemon: unknown, no home folder to find it from".to_owned()];
    };
    let deadline = Instant::now() + ANSWER_WITHIN;
    let asked = upgrade::ask_current(root, &user, &RequestBody::Status, deadline);
    let mut lines: Vec<String> = asked
        .replaced
        .iter()
        .map(|old| format!("daemon: replaced a daemon of Cortex {old}"))
        .collect();
    let line = match asked.answer {
        Ok(Answer {
            body: ResponseBody::Status(report),
            ..
        }) => {
            lines.extend(report_lines(&report, unix_millis(SystemTime::now())));
            return lines;
        }
        Ok(_) => "daemon: running, but it did not answer with its status".to_owned(),
        Err(ClientError::NotRunning) => "daemon: not running".to_owned(),
        Err(ClientError::TimedOut) => "daemon: not answering".to_owned(),
        Err(ClientError::Mismatch(theirs)) => format!(
            "daemon: running Cortex {}, which speaks another protocol",
            theirs.cortex_version
        ),
        Err(other) => format!("daemon: unreachable ({other})"),
    };
    lines.push(line);
    lines
}

/// What a running daemon reported, as lines: itself, its queue, its miner, each hook's handling
/// time, and its latest errors, each with how long before `now_ms` it last happened.
fn report_lines(report: &StatusReport, now_ms: i64) -> Vec<String> {
    let mut lines = vec![
        format!(
            "daemon: running (process {}, Cortex {}, up {} s)",
            report.pid,
            report.cortex_version,
            report.uptime_ms / 1000
        ),
        format!(
            "queue: {} events and {} changes waiting; {} events dropped, {} batches not \
             recorded, {} checkpoints failed",
            report.events_waiting,
            report.changes_waiting,
            report.events_dropped,
            report.failed_batches,
            report.failed_checkpoints
        ),
        git_line(report.git.as_ref()),
    ];
    if report.latencies.is_empty() {
        lines.push("daemon latency: no hook request handled yet".to_owned());
    }
    lines.extend(report.latencies.iter().map(latency_line));
    if report.recent_errors.is_empty() {
        lines.push("errors: none".to_owned());
    }
    lines.extend(
        report
            .recent_errors
            .iter()
            .map(|error| error_line(error, now_ms)),
    );
    lines
}

/// What the daemon's co-change miner is doing, as a line, or that the daemon did not say.
fn git_line(report: Option<&GitReport>) -> String {
    let Some(report) = report else {
        return "git: not reported by this daemon".to_owned();
    };
    let state = match report.state {
        GitState::Waiting => "not ready, waiting for the first index",
        GitState::NoRepository => "no repository at the root, so no history is mined",
        GitState::Mining => "not ready, mining the history",
        GitState::Ready => "ready",
        GitState::Failed => "not ready, mining failed and is tried again later",
        GitState::Other => "in a state this build does not know",
    };
    match &report.detail {
        Some(detail) => format!("git: {state}: {detail}"),
        None => format!("git: {state}"),
    }
}

/// How long the daemon took to handle one hook's requests, not counting the hook's own process
/// and connection: the percentiles in milliseconds and how many requests they cover.
fn latency_line(latency: &HookLatency) -> String {
    let requests = match latency.requests {
        1 => "1 request".to_owned(),
        count => format!("{count} requests"),
    };
    format!(
        "daemon latency: {} p50 {}, p95 {}, p99 {} (last {requests})",
        latency.hook,
        millis(latency.p50_us),
        millis(latency.p95_us),
        millis(latency.p99_us)
    )
}

/// One of the daemon's latest errors, how long before `now_ms` it last happened, and how many
/// times it did when more than once.
fn error_line(error: &RecentError, now_ms: i64) -> String {
    let when = ago(error.at_ms, now_ms);
    match error.count {
        1 => format!("error: {} ({when} ago)", error.message),
        count => format!("error: {} ({when} ago, {count} times)", error.message),
    }
}

/// `micros` microseconds as milliseconds to the microsecond: 1,235 µs reads `1.235 ms`, 4 µs
/// `0.004 ms`. The daemon handles a hook's event in microseconds, which two decimals would show as
/// nothing.
fn millis(micros: u64) -> String {
    format!("{}.{:03} ms", micros / 1_000, micros % 1_000)
}

/// How long before `now_ms` the time `at_ms` was: whole seconds under two minutes, whole minutes
/// under two hours, whole hours beyond. A time after `now_ms`, from a clock set back, reads as
/// `0 s`.
fn ago(at_ms: i64, now_ms: i64) -> String {
    let seconds = now_ms.saturating_sub(at_ms).max(0) / 1_000;
    match seconds {
        ..120 => format!("{seconds} s"),
        120..7_200 => format!("{} min", seconds / 60),
        _ => format!("{} h", seconds / 3_600),
    }
}

/// The status of the store in `folder` that `queries` reads, with the mode `setting`.
fn status_lines(
    folder: &Path,
    queries: Queries<'_>,
    setting: ModeSetting,
) -> anyhow::Result<Vec<String>> {
    let index = if queries.meta(MetaKey::CortexVersion)?.is_some() {
        "ready"
    } else if is_writer_running(folder)? {
        "not ready, an index is under way"
    } else {
        "not ready, the last index did not finish; the next one reads every file again"
    };
    let mut mode = queries
        .meta(MetaKey::ScaleMode)?
        .unwrap_or_else(|| "not counted yet".to_owned());
    if let ModeSetting::Pinned(pinned) = setting {
        mode.push_str(&format!(
            "; pinned to {} in the configuration",
            pinned.name()
        ));
    }
    let rows = queries.file_rows()?;
    let parsed = rows
        .iter()
        .filter(|row| row.file.flags.contains(FileFlags::PARSED))
        .count();
    let mut files = format!("{} stored, {parsed} parsed", rows.len());
    let waiting = rows
        .iter()
        .filter(|row| waits_to_be_parsed(&row.file))
        .count();
    if waiting > 0 {
        files.push_str(&format!(", {waiting} waiting to be parsed"));
    }
    let mark = Mark::read(queries)?;
    Ok(vec![
        format!("index: {index}"),
        format!("mode: {mode}"),
        format!("files: {files}"),
        cochange_line(mark.as_ref(), unix_millis(SystemTime::now())),
    ])
}

/// How far the history is mined, from the `mark` the last mining left, with how long before
/// `now_ms` the last full mining was.
fn cochange_line(mark: Option<&Mark>, now_ms: i64) -> String {
    let Some(mark) = mark else {
        return "co-change: nothing mined yet".to_owned();
    };
    if mark.last_commit.is_empty() {
        return "co-change: nothing to mine, the branch has no commit yet".to_owned();
    }
    let commit = mark.last_commit.get(..12).unwrap_or(&mark.last_commit);
    format!(
        "co-change: {} commits mined, up to {commit}; the last full mining {} ago",
        mark.last_seq,
        ago(mark.full_at_ms, now_ms)
    )
}

#[cfg(test)]
mod tests {
    use cortex_core::ipc::{GitReport, Protocol};

    use super::*;

    /// A report of a daemon of process 7, Cortex 1.2.3, up 61.5 s, with nothing waiting or lost.
    fn report() -> StatusReport {
        StatusReport {
            cortex_version: "1.2.3".to_owned(),
            protocol: Protocol::CURRENT,
            pid: 7,
            uptime_ms: 61_500,
            events_waiting: 0,
            changes_waiting: 0,
            events_dropped: 0,
            failed_batches: 0,
            failed_checkpoints: 0,
            probes_answered: 0,
            is_indexed: false,
            latencies: Vec::new(),
            recent_errors: Vec::new(),
            git: Some(GitReport::default()),
        }
    }

    /// A daemon that handled no hook and met no error says so, its miner still waiting for the
    /// first index.
    #[test]
    fn a_quiet_daemon_says_so() {
        assert_eq!(
            report_lines(&report(), 0),
            [
                "daemon: running (process 7, Cortex 1.2.3, up 61 s)",
                "queue: 0 events and 0 changes waiting; 0 events dropped, 0 batches not \
                 recorded, 0 checkpoints failed",
                "git: not ready, waiting for the first index",
                "daemon latency: no hook request handled yet",
                "errors: none",
            ]
        );
    }

    /// Each hook's percentiles print in milliseconds to the microsecond (125 µs is 0.125 ms, 4 µs
    /// 0.004 ms, 1,999,995 µs 1999.995 ms), in the report's order; each error prints with how long
    /// ago it last happened (12 s, 3 min from 180 s, 2 h from 7,200 s) and, when it happened more
    /// than once, how many times.
    #[test]
    fn latencies_and_errors_print_one_a_line() {
        let now_ms = 10_000_000;
        let report = StatusReport {
            latencies: vec![
                HookLatency {
                    hook: "session_start".to_owned(),
                    requests: 1,
                    p50_us: 125,
                    p95_us: 125,
                    p99_us: 125,
                },
                HookLatency {
                    hook: "post_tool".to_owned(),
                    requests: 1_000,
                    p50_us: 4,
                    p95_us: 2_004,
                    p99_us: 1_999_995,
                },
            ],
            recent_errors: vec![
                RecentError {
                    at_ms: now_ms - 7_200_000,
                    count: 1,
                    message: "first".to_owned(),
                },
                RecentError {
                    at_ms: now_ms - 180_000,
                    count: 3,
                    message: "second".to_owned(),
                },
                RecentError {
                    at_ms: now_ms - 12_400,
                    count: 1,
                    message: "third".to_owned(),
                },
            ],
            ..report()
        };
        let lines = report_lines(&report, now_ms);
        let expected = [
            "daemon latency: session_start p50 0.125 ms, p95 0.125 ms, p99 0.125 ms (last 1 \
             request)",
            "daemon latency: post_tool p50 0.004 ms, p95 2.004 ms, p99 1999.995 ms (last 1000 \
             requests)",
            "error: first (2 h ago)",
            "error: second (3 min ago, 3 times)",
            "error: third (12 s ago)",
        ];
        assert_eq!(lines.get(3..), Some(expected.map(str::to_owned).as_slice()));
    }

    /// Ages cross from seconds to minutes at two minutes and to hours at two hours; a time ahead
    /// of the clock reads as 0 s.
    #[test]
    fn ages_change_unit_at_two_of_the_next() {
        let cases = [
            (119_999, "119 s"),
            (120_000, "2 min"),
            (7_199_999, "119 min"),
            (7_200_000, "2 h"),
            (-5_000, "0 s"),
        ];
        for (age_ms, expected) in cases {
            assert_eq!(
                ago(1_000_000_000 - age_ms, 1_000_000_000),
                expected,
                "{age_ms}"
            );
        }
    }

    /// Each of the miner's states reads as its own line, with the daemon's words after it when it
    /// gave some, and a daemon that did not say is said not to.
    #[test]
    fn the_miner_reads_as_one_line() {
        let line = |state, detail: Option<&str>| {
            git_line(Some(&GitReport {
                state,
                detail: detail.map(str::to_owned),
            }))
        };
        assert_eq!(git_line(None), "git: not reported by this daemon");
        assert_eq!(
            line(GitState::Waiting, None),
            "git: not ready, waiting for the first index"
        );
        assert_eq!(
            line(GitState::NoRepository, None),
            "git: no repository at the root, so no history is mined"
        );
        assert_eq!(
            line(GitState::Mining, None),
            "git: not ready, mining the history"
        );
        assert_eq!(line(GitState::Ready, None), "git: ready");
        assert_eq!(
            line(GitState::Ready, Some("read with the git program")),
            "git: ready: read with the git program"
        );
        assert_eq!(
            line(GitState::Failed, Some("disk full")),
            "git: not ready, mining failed and is tried again later: disk full"
        );
        assert_eq!(
            line(GitState::Other, None),
            "git: in a state this build does not know"
        );
    }

    /// The co-change line says nothing was mined before a mark exists, that there is nothing to
    /// mine on a branch with no commit, and otherwise how many commits, up to which (its first 12
    /// characters), and how long ago the last full mining was: 3 min from 180 s.
    #[test]
    fn the_mark_reads_as_one_line() {
        assert_eq!(cochange_line(None, 0), "co-change: nothing mined yet");
        let mark = |commit: &str| Mark {
            last_commit: commit.to_owned(),
            last_seq: 42,
            full_at_ms: 1_000_000,
        };
        assert_eq!(
            cochange_line(Some(&mark("")), 1_180_000),
            "co-change: nothing to mine, the branch has no commit yet"
        );
        assert_eq!(
            cochange_line(
                Some(&mark("0123456789abcdef0123456789abcdef01234567")),
                1_180_000
            ),
            "co-change: 42 commits mined, up to 0123456789ab; the last full mining 3 min ago"
        );
    }
}
