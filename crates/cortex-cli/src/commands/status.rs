//! `cortex status`: whether the repository's daemon runs and, from the daemon itself, its queue,
//! what its co-change miner is doing, how long it takes to handle each hook and its latest errors;
//! then whether the index is ready, the scale mode, how many files are stored, parsed and waiting
//! to be parsed, how far the history is mined, what the gate asks before it hands a work order
//! over (the cold-start rule's bars, or the chance a kept confidence model must give, and whether a
//! kept model cannot be used), and what the self-check last found of the work orders given; last,
//! once the user has turned it on, whether Cortex sets repositories up on its own and the folders
//! it refused. The store is only read.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use cortex_core::ipc::{
    GitReport, GitState, GraphReport, HistoryReport, HookLatency, LinkedWorktreeReport,
    RecentError, RequestBody, ResponseBody, StatusReport, UpkeepReport, WorktreeReport,
    WorktreeState,
};
use cortex_core::{Config, FileFlags, ModeSetting, ScaleMode, UserFolder, unix_millis};
use cortex_daemon::settings;
use cortex_git::mine::Mark;
use cortex_hook::{Answer, ClientError};
use cortex_index::waits_to_be_parsed;
use cortex_learn::{MAX_RAISE_STEPS, ModelInUse, model_in_use, raise_of, recent_precision};
use cortex_retrieve::{LEXICAL_BAR, STRONG_AGREEING_KINDS, STRONG_LEXICAL_BAR, confidence_bar};
use cortex_store::{
    FactHealth, MetaKey, Queries, Reader, SelfCheckRecord, StoreError, WarningCounts,
    is_writer_running,
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
    for line in index_lines(&root, &loaded.config)? {
        output::line(&line)?;
    }
    // Adoption is said once the user has used it: on, or with refusals still remembered.
    if let Some(user) = UserFolder::of_current_user()
        && (user.adopt_path().is_file() || crate::adoption::latest(&user)?.0 > 0)
    {
        for line in crate::adoption::lines(&user)? {
            output::line(&line)?;
        }
    }
    Ok(())
}

/// The lines saying how far the index of the repository at `root` is, with the `config` in effect,
/// or that it has none, or that its writer is still setting up its store.
///
/// # Errors
///
/// The store cannot be read.
pub(crate) fn index_lines(root: &Path, config: &Config) -> anyhow::Result<Vec<String>> {
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
    status_lines(&folder, reader.queries(), config)
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

/// What a running daemon reported, as lines: itself, its queue, its miner, its history worker
/// when it runs, each hook's handling time, and its latest errors, each with how long before
/// `now_ms` it last happened.
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
        upkeep_line(report.upkeep.as_ref(), now_ms),
    ];
    lines.extend(report.history.as_ref().map(history_line));
    lines.extend(report.linked_worktrees.iter().map(linked_line));
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

/// What the daemon read of a linked working tree's uncommitted changes, as a line naming its top,
/// as the work tree's line says them.
fn linked_line(linked: &LinkedWorktreeReport) -> String {
    let said = linked
        .worktree
        .as_ref()
        .map_or_else(|| "not looked at yet".to_owned(), worktree_said);
    format!("linked work tree {}: {said}", linked.root.display())
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
        WorktreeState::Includes => {
            "not read, as the repository's own git configuration includes another file".to_owned()
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
pub(crate) fn counted(count: u64, (one, many): (&str, &str)) -> String {
    match count {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    }
}

/// What the last run of upkeep removed, how long before `now_ms` it ended and whether it finished,
/// as a line; that none ran when none did.
fn upkeep_line(report: Option<&UpkeepReport>, now_ms: i64) -> String {
    let Some(report) = report else {
        return "upkeep: not run since the daemon started".to_owned();
    };
    let removed = [
        (report.links, ("link of use", "links of use")),
        (report.interest, ("interest score", "interest scores")),
        (report.facts, ("retired fact", "retired facts")),
        (report.events, ("old event", "old events")),
        (report.episodes, ("episode", "episodes")),
        (report.positions, ("position", "positions")),
        (report.pages, ("free page", "free pages")),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, names)| counted(count, names))
    .collect::<Vec<_>>();
    let removed = if removed.is_empty() {
        "nothing to remove".to_owned()
    } else {
        format!("removed {}", removed.join(", "))
    };
    let finished = if report.is_done {
        "done"
    } else {
        "not finished, carries on at its next run"
    };
    format!(
        "upkeep: last ran {} ago, {removed}; {finished}",
        ago(report.at_ms, now_ms)
    )
}

/// What the daemon's history worker holds, as a line: the cards and blames the store keeps, the
/// jobs waiting, the blames that failed when any did, and why git is refused when it is.
fn history_line(report: &HistoryReport) -> String {
    let mut line = format!(
        "history: {}, {}, {}",
        counted(report.cards, ("commit carded", "commits carded")),
        counted(report.cached_blames, ("blame kept", "blames kept")),
        counted(report.jobs_waiting, ("job waiting", "jobs waiting")),
    );
    if report.failed_blames > 0 {
        line.push_str(&format!(
            ", {}",
            counted(report.failed_blames, ("blame failed", "blames failed"))
        ));
    }
    if let Some(why) = &report.refused {
        line.push_str(&format!("; git is not run for history: {why}"));
    }
    line
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

/// The status of the store in `folder` that `queries` reads, with the `config` in effect.
fn status_lines(
    folder: &Path,
    queries: Queries<'_>,
    config: &Config,
) -> anyhow::Result<Vec<String>> {
    let index = if queries.meta(MetaKey::CortexVersion)?.is_some() {
        "ready"
    } else if is_writer_running(folder)? {
        "not ready, an index is under way"
    } else {
        "not ready, the last index did not finish; the next one reads every file again"
    };
    let stored_mode = queries.meta(MetaKey::ScaleMode)?;
    let mode_now = match config.mode {
        ModeSetting::Pinned(pinned) => Some(pinned),
        ModeSetting::Auto => stored_mode.as_deref().and_then(ScaleMode::from_name),
    };
    let mut mode = stored_mode.unwrap_or_else(|| "not counted yet".to_owned());
    if let ModeSetting::Pinned(pinned) = config.mode {
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
        Ok(record) => {
            let in_use = model_in_use(queries)?;
            let bar = (config.retrieval.min_confidence, mode_now);
            lines.extend(gate_lines(&record, &in_use, bar));
            lines.push(self_check_line(&record, in_use.model().is_some()));
        }
        Err(StoreError::CorruptMeta { .. }) => {
            lines.push("gate: the self-check's record cannot be read".to_owned());
        }
        Err(error) => return Err(error.into()),
    }
    lines.push(warnings_line(queries.warning_counts()?));
    lines.extend(next_file_line(queries.next_file_use()?));
    lines.extend(health_line(queries.fact_health()?));
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

/// How many lines naming the file earlier tasks read next were given and counted at their task's
/// end, and how many named a file the task then read or edited; none before any was counted.
fn next_file_line((named, used): (u64, u64)) -> Option<String> {
    (named > 0).then(|| {
        #[expect(
            clippy::cast_precision_loss,
            reason = "counts of lines given stay far below the 2^52 a float holds exactly"
        )]
        let share = used as f64 / named as f64;
        format!(
            "next files: {} named; {used} then read or edited ({share:.2})",
            counted(named, ("file", "files"))
        )
    })
}

/// The line naming the facts that need a person, none when no fact does: those anchored to code
/// the index no longer holds, and those suspect since their code changed.
fn health_line(health: FactHealth) -> Option<String> {
    let mut found = Vec::new();
    if health.gone > 0 {
        found.push(format!(
            "{} anchored to code that is gone",
            counted(health.gone, ("fact", "facts"))
        ));
    }
    if health.changed > 0 {
        found.push(format!(
            "{} whose code changed since",
            counted(health.changed, ("fact", "facts"))
        ));
    }
    (!found.is_empty()).then(|| {
        format!(
            "health: {}; `cortex facts` lists them to confirm or retire",
            found.join(", ")
        )
    })
}

/// What the gate asks before it hands a work order over, with the self-check's `record` and the
/// confidence model `in_use`. Once a model is kept: which, what it was fitted on and how well it
/// ranked later work, and the chance it must give, from `retrieval.min_confidence` and the mode now
/// (both chances, the other modes' and small mode's, while the mode is not known yet), raised as
/// the record says. Before then: the cold-start rule's bar for the prompt's words, raised as the
/// record says, and the lower bar that holds when the most kinds of evidence agree; and, when the
/// newest model kept cannot be used, that the cold-start rule decides until upkeep fits another.
fn gate_lines(
    record: &SelfCheckRecord,
    in_use: &ModelInUse,
    (min_confidence, mode): (f64, Option<ScaleMode>),
) -> Vec<String> {
    let raise = raise_of(record);
    if let ModelInUse::Kept {
        version,
        samples,
        auc,
        ..
    } = in_use
    {
        let chance = match mode {
            Some(mode) => format!("{:.2}", confidence_bar(min_confidence, mode, raise)),
            None => format!(
                "{:.2} ({:.2} in small mode)",
                confidence_bar(min_confidence, ScaleMode::Medium, raise),
                confidence_bar(min_confidence, ScaleMode::Small, raise)
            ),
        };
        let fitted = counted(
            u64::from(*samples),
            ("labelled retrieval", "labelled retrievals"),
        );
        return vec![format!(
            "gate: confidence model {version}, fitted on {fitted} (AUC {auc:.2} on later ones); a \
             work order needs a chance of {chance}"
        )];
    }
    let (bar, strong_bar) = (LEXICAL_BAR + raise, STRONG_LEXICAL_BAR + raise);
    let mut lines = vec![format!(
        "gate: bar {bar:.2}, {strong_bar:.2} when {STRONG_AGREEING_KINDS} kinds agree"
    )];
    if let ModelInUse::Unusable { version } = in_use {
        lines.push(format!(
            "gate: confidence model {version} cannot be used by this build, so the bars above \
             decide until upkeep fits a new one"
        ));
    }
    lines
}

/// What the self-check last found, as its `record` keeps it: how often the latest work orders
/// given were used, and how far it raised the gate's bar: by how much, while the cold-start rule
/// decides (`is_model` false), since a kept model's bar stops at 0.9 and the gate line gives it.
fn self_check_line(record: &SelfCheckRecord, is_model: bool) -> String {
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
        steps => {
            let by = if is_model {
                String::new()
            } else {
                format!(", by {:.2}", raise_of(record))
            };
            format!(
                "self-check: {found}; the bar is raised {steps} of {MAX_RAISE_STEPS} steps{by}, \
                 last when {} had been labelled",
                counted(record.raised_at, ("work order", "work orders")),
            )
        }
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
    use cortex_learn::{ConfidenceModel, MODEL_INPUTS};

    use super::*;

    /// No line before a next-file line is counted; 4 named and 3 used read 0.75; 1 named reads in
    /// the singular.
    #[test]
    fn the_next_file_line_says_how_often_the_file_was_read() {
        assert_eq!(next_file_line((0, 0)), None);
        assert_eq!(
            next_file_line((4, 3)).as_deref(),
            Some("next files: 4 files named; 3 then read or edited (0.75)")
        );
        assert_eq!(
            next_file_line((1, 0)).as_deref(),
            Some("next files: 1 file named; 0 then read or edited (0.00)")
        );
    }

    /// The gate's lines, case by case. With no model: the cold-start bars, 0.50 and 0.10, lifted by
    /// a raise of two steps (0.10) to 0.60 and 0.20. With model 3, fitted on 1600 retrievals at an
    /// AUC of 0.834: in medium mode the setting's 0.70; with two steps of raise, 0.80; in small
    /// mode at least 0.85, and 0.85 + 0.10 stops at 0.90; with the mode not known yet, both. With
    /// model 2 kept but not usable: the cold-start bars, and a line saying so.
    #[test]
    fn the_gate_lines_say_which_rule_decides_and_its_bar() {
        let none = SelfCheckRecord::default();
        let raised = SelfCheckRecord {
            raise_steps: 2,
            raised_at: 300,
            ..none
        };
        let kept = ModelInUse::Kept {
            model: ConfidenceModel {
                intercept: 0.0,
                weights: [0.0; MODEL_INPUTS],
            },
            version: 3,
            samples: 1_600,
            auc: 0.834,
        };
        let model_line = |chance: &str| {
            vec![format!(
                "gate: confidence model 3, fitted on 1600 labelled retrievals (AUC 0.83 on later \
                 ones); a work order needs a chance of {chance}"
            )]
        };
        let medium = Some(ScaleMode::Medium);
        assert_eq!(
            gate_lines(&none, &ModelInUse::None, (0.7, medium)),
            ["gate: bar 0.50, 0.10 when 3 kinds agree"]
        );
        assert_eq!(
            gate_lines(&raised, &ModelInUse::None, (0.7, medium)),
            ["gate: bar 0.60, 0.20 when 3 kinds agree"]
        );
        assert_eq!(gate_lines(&none, &kept, (0.7, medium)), model_line("0.70"));
        assert_eq!(
            gate_lines(&raised, &kept, (0.7, medium)),
            model_line("0.80")
        );
        assert_eq!(
            gate_lines(&raised, &kept, (0.7, Some(ScaleMode::Small))),
            model_line("0.90")
        );
        assert_eq!(
            gate_lines(&none, &kept, (0.7, None)),
            model_line("0.70 (0.85 in small mode)")
        );
        assert_eq!(
            gate_lines(&none, &ModelInUse::Unusable { version: 2 }, (0.7, medium)),
            [
                "gate: bar 0.50, 0.10 when 3 kinds agree",
                "gate: confidence model 2 cannot be used by this build, so the bars above decide \
                 until upkeep fits a new one"
            ]
        );
    }

    /// With the bar raised two steps, the self-check says by how much while the cold-start rule
    /// decides, and not once a model does, whose bar the gate line gives.
    #[test]
    fn the_self_check_line_says_the_raise_where_it_is_the_bar() {
        let raised = SelfCheckRecord {
            raise_steps: 2,
            raised_at: 300,
            recent_labelled: 100,
            recent_used: 52,
        };
        assert_eq!(
            self_check_line(&raised, false),
            "self-check: precision 0.52 over the latest 100 work orders given; the bar is raised 2 \
             of 4 steps, by 0.10, last when 300 work orders had been labelled"
        );
        assert_eq!(
            self_check_line(&raised, true),
            "self-check: precision 0.52 over the latest 100 work orders given; the bar is raised 2 \
             of 4 steps, last when 300 work orders had been labelled"
        );
    }

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
            linked_worktrees: Vec::new(),
            upkeep: None,
            history: None,
        }
    }

    /// The history worker's line names what the store keeps and what waits, then the failures
    /// and the refusal only when there are any; a daemon with history off has no such line.
    #[test]
    fn the_history_worker_reads_as_one_line() {
        let mut history = HistoryReport {
            cards: 12,
            cached_blames: 1,
            jobs_waiting: 0,
            failed_blames: 0,
            refused: None,
        };
        assert_eq!(
            history_line(&history),
            "history: 12 commits carded, 1 blame kept, 0 jobs waiting"
        );
        history.failed_blames = 2;
        history.refused = Some("the repository's own git configuration names a filter".to_owned());
        assert_eq!(
            history_line(&history),
            "history: 12 commits carded, 1 blame kept, 0 jobs waiting, 2 blames failed; git is \
             not run for history: the repository's own git configuration names a filter"
        );
        let with_history = StatusReport {
            history: Some(history),
            ..report()
        };
        let lines = report_lines(&with_history, 0);
        assert!(
            lines
                .get(6)
                .is_some_and(|line| line.starts_with("history: "))
        );
        assert!(
            !report_lines(&report(), 0)
                .iter()
                .any(|line| line.starts_with("history"))
        );
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
                "upkeep: not run since the daemon started",
                "daemon latency: no hook request handled yet",
                "errors: none",
            ]
        );
    }

    /// Worked by hand: a run that ended 180 s before now, having removed one link of use and two
    /// old events, done, is said in minutes with what it removed; one that removed nothing and
    /// yielded says so and that it carries on.
    #[test]
    fn the_last_upkeep_run_says_what_it_removed() {
        let ran = |links, events, is_done| UpkeepReport {
            at_ms: 1_000,
            links,
            interest: 0,
            facts: 0,
            events,
            episodes: 0,
            positions: 0,
            pages: 0,
            is_done,
        };
        assert_eq!(
            upkeep_line(Some(&ran(1, 2, true)), 181_000),
            "upkeep: last ran 3 min ago, removed 1 link of use, 2 old events; done"
        );
        assert_eq!(
            upkeep_line(Some(&ran(0, 0, false)), 181_000),
            "upkeep: last ran 3 min ago, nothing to remove; not finished, carries on at its next \
             run"
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
        assert_eq!(lines.get(6..), Some(expected.map(str::to_owned).as_slice()));
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
                WorktreeState::Includes,
                "work tree: not read, as the repository's own git configuration includes another file",
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

    /// Each linked working tree an agent worked in gets a line of its own, naming its top, before
    /// its first look and after.
    #[test]
    fn each_linked_working_tree_is_reported_in_a_line() {
        let top = std::path::PathBuf::from("wt");
        let waiting = LinkedWorktreeReport {
            root: top.clone(),
            worktree: None,
        };
        assert_eq!(
            linked_line(&waiting),
            "linked work tree wt: not looked at yet"
        );
        let read = LinkedWorktreeReport {
            root: top,
            worktree: Some(WorktreeReport {
                state: WorktreeState::Read,
                uncommitted_files: 1,
                listed_files: 1,
                looks: 2,
                failed_looks: 0,
            }),
        };
        assert_eq!(
            linked_line(&read),
            "linked work tree wt: 1 uncommitted file kept to seed retrievals, as of the latest of \
             2 looks"
        );
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

    /// Worked by hand: nothing to look at, no line; one fact on code that is gone; two whose code
    /// changed; both, those on code that is gone first.
    #[test]
    fn health_is_named_only_when_a_fact_needs_a_person() {
        let line = |gone, changed| health_line(FactHealth { gone, changed });
        let tail = "; `cortex facts` lists them to confirm or retire";
        assert_eq!(line(0, 0), None);
        assert_eq!(
            line(1, 0),
            Some(format!(
                "health: 1 fact anchored to code that is gone{tail}"
            ))
        );
        assert_eq!(
            line(0, 2),
            Some(format!("health: 2 facts whose code changed since{tail}"))
        );
        assert_eq!(
            line(2, 1),
            Some(format!(
                "health: 2 facts anchored to code that is gone, 1 fact whose code changed \
                 since{tail}"
            ))
        );
    }
}
