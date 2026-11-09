//! `cortex status`: whether the repository's daemon runs and, from the daemon itself, its queue,
//! what its co-change miner is doing, how long it takes to handle each hook and its latest errors;
//! then whether the index is ready, the scale mode, how many files are stored, parsed and waiting
//! to be parsed, how far the history is mined, what the gate asks before it hands a work order
//! over, and what the self-check last found of the work orders given. The store is only read.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use cortex_core::ipc::{
    GitReport, GitState, GraphReport, HookLatency, RecentError, RequestBody, ResponseBody,
    StatusReport, WorktreeReport, WorktreeState,
};
use cortex_core::{FileFlags, ModeSetting, UserFolder, unix_millis};
use cortex_daemon::settings;
use cortex_git::mine::Mark;
use cortex_hook::{Answer, ClientError};
use cortex_index::waits_to_be_parsed;
use cortex_learn::{MAX_RAISE_STEPS, raise_of, recent_precision};
use cortex_retrieve::{LEXICAL_BAR, STRONG_AGREEING_KINDS, STRONG_LEXICAL_BAR};
use cortex_store::{
    MetaKey, Queries, Reader, SelfCheckRecord, StoreError, WarningCounts, is_writer_running,
};

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
        queue_line(report),
        git_line(report.git.as_ref()),
        graph_line(report.graph.as_ref()),
        worktree_line(report.worktree.as_ref()),
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

/// What waits for the daemon, events and changes together, then each loss the daemon counted
/// (events dropped, batches not recorded, checkpoints failed) only when it is not zero.
fn queue_line(report: &StatusReport) -> String {
    let waiting = report.events_waiting.saturating_add(report.changes_waiting);
    let mut line = format!("queue: {waiting} waiting");
    let losses = [
        (report.events_dropped, ("event dropped", "events dropped")),
        (
            report.failed_batches,
            ("batch not recorded", "batches not recorded"),
        ),
        (
            report.failed_checkpoints,
            ("checkpoint failed", "checkpoints failed"),
        ),
    ];
    for (count, words) in losses {
        if count > 0 {
            line.push_str(&format!("; {}", counted(count, words)));
        }
    }
    line
}

/// What the daemon read of the work tree's uncommitted changes, as a line: the files kept to seed
/// retrievals, of how many git listed when fewer, and its looks; why the changes are not known; or
/// that it has not looked yet. The failed looks follow when any failed.
fn worktree_line(report: Option<&WorktreeReport>) -> String {
    let Some(report) = report else {
        return "work tree: not looked at yet".to_owned();
    };
    let failed = match report.failed_looks {
        0 => String::new(),
        failed => format!("; {} failed", counted(failed, ("look", "looks"))),
    };
    format!("work tree: {}{failed}", worktree_said(report))
}

/// What the work tree's line says of `report`'s state.
fn worktree_said(report: &WorktreeReport) -> String {
    let files = ("uncommitted file", "uncommitted files");
    match report.state {
        WorktreeState::Read => {
            let kept = if report.listed_files > report.uncommitted_files {
                format!(
                    "{} of {}",
                    report.uncommitted_files,
                    counted(report.listed_files, files)
                )
            } else {
                counted(report.uncommitted_files, files)
            };
            format!(
                "{kept} kept to seed retrievals, as of the latest of {}",
                counted(report.looks, ("look", "looks"))
            )
        }
        WorktreeState::NoGit => "unknown, as no git program is on the PATH".to_owned(),
        WorktreeState::NoRepository => "no repository at the root, so no change is read".to_owned(),
        WorktreeState::Refused => {
            "not read, as the repository's own git configuration names a filter a look would run"
                .to_owned()
        }
        WorktreeState::WouldFetch => "not read; update git to 2.45 or later".to_owned(),
        WorktreeState::TooMany => {
            let many = match report.listed_files {
                0 => "more uncommitted files than a look reads".to_owned(),
                listed => counted(listed, files),
            };
            format!("{many}, a mass change, so none seeds retrievals")
        }
        WorktreeState::HeadMoved => "looking again, as HEAD moved".to_owned(),
        WorktreeState::Failed => "unknown, as the latest look failed".to_owned(),
        WorktreeState::Other => "in a state this build does not know".to_owned(),
    }
}

/// `count` things, named `one` when there is one and `many` otherwise.
fn counted(count: u64, (one, many): (&str, &str)) -> String {
    match count {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    }
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

/// What the daemon's graph holds, as a line, with how many of its rebuilds failed when any did,
/// or that it has built none yet.
fn graph_line(report: Option<&GraphReport>) -> String {
    let failed = match report.map_or(0, |report| report.failed_builds) {
        0 => String::new(),
        failed => format!("; {} failed", counted(failed, ("build", "builds"))),
    };
    let Some(report) = report.filter(|report| report.builds > 0) else {
        return format!("graph: none built yet{failed}");
    };
    let cochange = if report.has_cochange {
        "with"
    } else {
        "without"
    };
    let builds = match report.builds {
        1 => "once".to_owned(),
        builds => format!("{builds} times"),
    };
    format!(
        "graph: {}, {}, {cochange} co-change links; built {builds}, the latest in {} ms{failed}",
        counted(report.nodes, ("node", "nodes")),
        counted(report.links, ("link", "links")),
        report.build_ms
    )
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
    let mut lines = vec![
        format!("index: {index}"),
        format!("mode: {mode}"),
        format!("files: {files}"),
        cochange_line(mark.as_ref(), unix_millis(SystemTime::now())),
    ];
    match queries.self_check() {
        Ok(record) => lines.extend([gate_line(&record), self_check_line(&record)]),
        Err(StoreError::CorruptMeta { .. }) => {
            lines.push("gate: the self-check's record cannot be read".to_owned());
        }
        Err(error) => return Err(error.into()),
    }
    lines.push(warnings_line(queries.warning_counts()?));
    Ok(lines)
}

/// How many files warnings named, and how many of those marked at their task's end were then read
/// or edited.
fn warnings_line(counts: WarningCounts) -> String {
    let named = counted(counts.given, ("file named", "files named"));
    match (counts.given, counts.labelled) {
        (0, _) => "warnings: none given yet".to_owned(),
        (_, 0) => format!("warnings: {named}, none marked yet"),
        (_, labelled) => {
            #[expect(
                clippy::cast_precision_loss,
                reason = "counts of warnings stay far below the 2^52 a float holds exactly"
            )]
            let share = counts.resolved as f64 / labelled as f64;
            let verb = if counts.resolved == 1 { "was" } else { "were" };
            format!(
                "warnings: {named}; {} of {labelled} marked {verb} then read or edited ({share:.2})",
                counts.resolved
            )
        }
    }
}

/// What the gate asks before it hands a work order over: its bar for the prompt's words, raised as
/// far as the self-check's `record` says, and the lower bar that holds when the most kinds of
/// evidence agree.
fn gate_line(record: &SelfCheckRecord) -> String {
    let raise = raise_of(record);
    let (bar, strong_bar) = (LEXICAL_BAR + raise, STRONG_LEXICAL_BAR + raise);
    format!("gate: bar {bar:.2}, {strong_bar:.2} when {STRONG_AGREEING_KINDS} kinds agree")
}

/// What the self-check last found, as its `record` keeps it: how often the latest work orders
/// given were used, and how far it raised the gate's bar.
fn self_check_line(record: &SelfCheckRecord) -> String {
    let found = match recent_precision(record) {
        None => "no work order given is labelled yet".to_owned(),
        Some(precision) => format!(
            "precision {precision:.2} over the latest {}",
            counted(
                u64::from(record.recent_labelled),
                ("work order given", "work orders given")
            ),
        ),
    };
    match record.raise_steps {
        0 => format!("self-check: {found}"),
        steps => format!(
            "self-check: {found}; the bar is raised {steps} of {MAX_RAISE_STEPS} steps, by {:.2}, \
             last when {} had been labelled",
            raise_of(record),
            counted(record.raised_at, ("work order", "work orders")),
        ),
    }
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
            graph: None,
            worktree: None,
        }
    }

    /// A daemon that handled no hook and met no error says so, its miner still waiting for the
    /// first index, its graph not built yet and its work tree not looked at.
    #[test]
    fn a_quiet_daemon_says_so() {
        assert_eq!(
            report_lines(&report(), 0),
            [
                "daemon: running (process 7, Cortex 1.2.3, up 61 s)",
                "queue: 0 waiting",
                "git: not ready, waiting for the first index",
                "graph: none built yet",
                "work tree: not looked at yet",
                "daemon latency: no hook request handled yet",
                "errors: none",
            ]
        );
    }

    /// The queue counts events and changes together (3 and 4 are 7 waiting) and names a loss only
    /// when it is not zero, singular for one.
    #[test]
    fn the_queue_names_only_the_losses_that_happened() {
        let queue = |events_dropped, failed_batches, failed_checkpoints| {
            queue_line(&StatusReport {
                events_waiting: 3,
                changes_waiting: 4,
                events_dropped,
                failed_batches,
                failed_checkpoints,
                ..report()
            })
        };
        assert_eq!(queue(0, 0, 0), "queue: 7 waiting");
        assert_eq!(queue(2, 0, 0), "queue: 7 waiting; 2 events dropped");
        assert_eq!(
            queue(1, 1, 1),
            "queue: 7 waiting; 1 event dropped; 1 batch not recorded; 1 checkpoint failed"
        );
        assert_eq!(queue(0, 5, 0), "queue: 7 waiting; 5 batches not recorded");
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
        assert_eq!(lines.get(5..), Some(expected.map(str::to_owned).as_slice()));
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

    /// The work tree's line, as typed out here: the files kept and the looks, one of a kind in the
    /// singular, of how many git listed when fewer are kept, the failed looks when any failed; each
    /// reason the changes are not known, a mass change counted or too large to count; no look yet.
    #[test]
    fn the_work_tree_is_reported_in_one_line() {
        let report = WorktreeReport {
            state: WorktreeState::Read,
            uncommitted_files: 2,
            listed_files: 2,
            looks: 3,
            failed_looks: 0,
        };
        let line = |report: &WorktreeReport| worktree_line(Some(report));
        assert_eq!(
            line(&report),
            "work tree: 2 uncommitted files kept to seed retrievals, as of the latest of 3 looks"
        );
        let one = WorktreeReport {
            uncommitted_files: 1,
            listed_files: 1,
            looks: 1,
            failed_looks: 2,
            ..report.clone()
        };
        assert_eq!(
            line(&one),
            "work tree: 1 uncommitted file kept to seed retrievals, as of the latest of 1 look; 2 \
             looks failed"
        );
        let fewer = WorktreeReport {
            uncommitted_files: 10,
            listed_files: 37,
            ..report.clone()
        };
        assert_eq!(
            line(&fewer),
            "work tree: 10 of 37 uncommitted files kept to seed retrievals, as of the latest of 3 \
             looks"
        );
        let unknown = |state| WorktreeReport {
            state,
            uncommitted_files: 0,
            listed_files: 0,
            ..report.clone()
        };
        let cases = [
            (
                WorktreeState::NoGit,
                "work tree: unknown, as no git program is on the PATH",
            ),
            (
                WorktreeState::NoRepository,
                "work tree: no repository at the root, so no change is read",
            ),
            (
                WorktreeState::Refused,
                "work tree: not read, as the repository's own git configuration names a filter a \
                 look would run",
            ),
            (
                WorktreeState::WouldFetch,
                "work tree: not read; update git to 2.45 or later",
            ),
            (
                WorktreeState::TooMany,
                "work tree: more uncommitted files than a look reads, a mass change, so none seeds \
                 retrievals",
            ),
            (
                WorktreeState::HeadMoved,
                "work tree: looking again, as HEAD moved",
            ),
            (
                WorktreeState::Other,
                "work tree: in a state this build does not know",
            ),
        ];
        for (state, expected) in cases {
            assert_eq!(line(&unknown(state)), expected);
        }
        let mass = WorktreeReport {
            listed_files: 5_001,
            ..unknown(WorktreeState::TooMany)
        };
        assert_eq!(
            line(&mass),
            "work tree: 5001 uncommitted files, a mass change, so none seeds retrievals"
        );
        let failing = WorktreeReport {
            failed_looks: 4,
            ..unknown(WorktreeState::Failed)
        };
        assert_eq!(
            line(&failing),
            "work tree: unknown, as the latest look failed; 4 looks failed"
        );
        assert_eq!(worktree_line(None), "work tree: not looked at yet");
    }

    /// The graph's line, as typed out here: its size, one of a kind in the singular, whether it
    /// holds co-change links, its builds, one said as once, and the failed ones when any failed;
    /// none before the first build, with the failures when the first builds failed.
    #[test]
    fn the_graph_is_reported_in_one_line() {
        let report = GraphReport {
            nodes: 1200,
            links: 3400,
            has_cochange: true,
            builds: 3,
            build_ms: 45,
            failed_builds: 0,
        };
        assert_eq!(
            graph_line(Some(&report)),
            "graph: 1200 nodes, 3400 links, with co-change links; built 3 times, the latest in 45 ms"
        );
        let failing = GraphReport {
            failed_builds: 2,
            ..report.clone()
        };
        assert_eq!(
            graph_line(Some(&failing)),
            "graph: 1200 nodes, 3400 links, with co-change links; built 3 times, the latest in 45 \
             ms; 2 builds failed"
        );
        let never = GraphReport {
            nodes: 0,
            links: 0,
            has_cochange: false,
            builds: 0,
            build_ms: 0,
            failed_builds: 1,
        };
        assert_eq!(
            graph_line(Some(&never)),
            "graph: none built yet; 1 build failed"
        );
        let first = GraphReport {
            nodes: 1,
            links: 1,
            has_cochange: false,
            builds: 1,
            build_ms: 45,
            failed_builds: 0,
        };
        assert_eq!(
            graph_line(Some(&first)),
            "graph: 1 node, 1 link, without co-change links; built once, the latest in 45 ms"
        );
        assert_eq!(graph_line(None), "graph: none built yet");
    }

    /// The warnings read as one line: none given; given and none marked; 7 of 10 marked resolved,
    /// 0.70; and one of one, in the singular.
    #[test]
    fn the_warnings_read_as_one_line() {
        let line = |given, labelled, resolved| {
            warnings_line(WarningCounts {
                given,
                labelled,
                resolved,
            })
        };
        assert_eq!(line(0, 0, 0), "warnings: none given yet");
        assert_eq!(line(3, 0, 0), "warnings: 3 files named, none marked yet");
        assert_eq!(
            line(12, 10, 7),
            "warnings: 12 files named; 7 of 10 marked were then read or edited (0.70)"
        );
        assert_eq!(
            line(1, 1, 1),
            "warnings: 1 file named; 1 of 1 marked was then read or edited (1.00)"
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
            anchor_seq: 40,
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
