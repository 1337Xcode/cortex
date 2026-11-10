//! Upkeep: the daemon's housekeeping, run on the writer's own thread (one core, and never a second
//! writer) once the repository has been quiet for [`QUIET_BEFORE_UPKEEP`] or a session ended. Its
//! jobs run in order, each a batch at a time in a transaction of its own: links of use that have
//! faded below 0.01 are removed, then the ways tasks read on from file to file that have, then
//! interest that has; the facts are reviewed and those retired
//! for 90 days deleted; the gate's confidence model is fitted when one is due (see
//! [`cortex_learn::calibrate`]); events and injections older than 90 days are deleted, episodes
//! kept to 50,000 (unknown outcomes first) and positions that can no longer hold removed; last, the
//! query planner's statistics are refreshed and up to [`VACUUM_PAGES`] free pages given back to the
//! file system. Each batch keeps, in the store, the step to carry on from, so that a daemon started
//! later finds it; a step whose batch fails is left for the next pass and the steps after it run.
//! Between batches a run stops when a hook event or a request waits, after
//! [`UPKEEP_RUN_BUDGET`], or when the write-ahead log holds more than its limit; the next run
//! carries on where it stopped, [`PACE_WAIT`] later after the last, by when the checkpointer has
//! copied the log and the next commit starts it afresh, so that upkeep's commits leave the
//! copying to the checkpointer. A reader that holds the log, or a writer whose commits keep
//! coming, can keep it from starting afresh; each run after a pace then writes one batch and
//! waits again.

use std::time::{Duration, Instant};

use cortex_learn::{Calibrated, calibrate, has_faded_out, review_facts};
use cortex_store::{MetaKey, NodeId, Queries, StoreError, Writer};

/// How long the repository must be quiet, no hook event recorded, before upkeep runs.
pub(crate) const QUIET_BEFORE_UPKEEP: Duration = Duration::from_secs(10 * 60);

/// The longest one run of upkeep goes on before it stops and leaves the rest for the next.
pub(crate) const UPKEEP_RUN_BUDGET: Duration = Duration::from_secs(120);

/// How much the write-ahead log may hold before upkeep waits: 32 MiB, half the 64 MiB at which
/// the writer checkpoints it inside a commit, which in the upkeep benchmark kept a hook event
/// waiting 65 to 204 ms, and seconds while the disk was busy. A batch writes a few MiB at most.
pub const LOG_LIMIT_BYTES: u64 = 32 * 1_024 * 1_024;

/// How long upkeep waits once the log holds too much: two of the checkpointer's one-second
/// periods, so that it has copied every page upkeep wrote.
pub const PACE_WAIT: Duration = Duration::from_secs(2);

/// Rows weighed or deleted in one batch: a transaction short enough that a hook event waiting
/// behind it is recorded within milliseconds.
const BATCH_ROWS: u64 = 500;

/// How long events, injections and facts retired are kept: 90 days, in milliseconds.
const KEPT_FOR_MS: i64 = 90 * 24 * 60 * 60 * 1_000;

/// The most episodes kept.
const MAX_EPISODES: u64 = 50_000;

/// The most free pages of the database upkeep gives back each time through its jobs: 2,000
/// pages, 8 MiB at SQLite's usual page size, however much was deleted; the rest goes next time.
const VACUUM_PAGES: u64 = 2_000;

/// Free pages given back in one batch, a quarter of the most each time: a hook event waiting behind
/// the vacuum waits for a quarter of it (8 to 83 ms a batch on the upkeep benchmark's large store,
/// where refreshing the statistics and giving back 2,000 pages in one batch took up to 215 ms).
const VACUUM_BATCH_PAGES: u32 = 500;

/// What upkeep weighs with: the settings that say how links and interest fade, how long a
/// position can hold at the longest, and how much the write-ahead log may hold.
#[derive(Debug, Clone, Copy)]
pub struct UpkeepSettings {
    /// `learning.link_decay`.
    pub link_decay: f64,
    /// `learning.interest_decay`.
    pub interest_decay: f64,
    /// The longest any position holds, in milliseconds.
    pub longest_hold_ms: i64,
    /// How much the write-ahead log may hold before a run stops, [`LOG_LIMIT_BYTES`].
    pub log_limit_bytes: u64,
}

/// What a run removed or changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Swept {
    /// Links of use removed.
    pub links: u64,
    /// Ways tasks read on from file to file removed.
    pub transitions: u64,
    /// Nodes whose interest was removed.
    pub interest: u64,
    /// Facts deleted.
    pub facts: u64,
    /// Events deleted.
    pub events: u64,
    /// Injections deleted.
    pub injections: u64,
    /// Episodes deleted.
    pub episodes: u64,
    /// Positions removed.
    pub positions: u64,
    /// Free pages given back to the file system.
    pub pages: u64,
}

/// Why a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// Every job is done.
    Done,
    /// Other work waits: the run carries on once it is done.
    Yielded,
    /// The run used its budget: the next quiet spell carries on.
    OutOfTime,
    /// The write-ahead log holds more than its limit: the next run carries on [`PACE_WAIT`]
    /// later.
    Paced,
}

/// When upkeep is due: once the repository has been quiet for [`QUIET_BEFORE_UPKEEP`] since the
/// last hook event, again only after another event; at once after a session ended or when asked;
/// again at once after a run that yielded to other work, and [`PACE_WAIT`] after one that paced.
#[derive(Debug)]
pub(crate) struct UpkeepClock {
    /// When the last hook event was recorded.
    quiet_since: Instant,
    /// Whether a run ended, done or out of time, since then.
    has_run: bool,
    /// Whether a run is asked for now: a session ended, or a person asked.
    is_asked: bool,
    /// Not before then: the last run paced.
    paced_until: Option<Instant>,
    /// When the run under way began, through the pieces a pace or other work cut it into; none
    /// between runs.
    run_since: Option<Instant>,
}

impl UpkeepClock {
    /// A clock whose quiet spell starts at `now`.
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            quiet_since: now,
            has_run: false,
            is_asked: false,
            paced_until: None,
            run_since: None,
        }
    }

    /// When the run under way, or the one starting at `now`, must stop: [`UPKEEP_RUN_BUDGET`] after
    /// it began, however many pieces it runs in.
    pub(crate) fn run_deadline(&mut self, now: Instant) -> Instant {
        *self.run_since.get_or_insert(now) + UPKEEP_RUN_BUDGET
    }

    /// Notes hook events recorded at `now`, among them a session's end when `has_session_end`.
    pub(crate) fn heard(&mut self, now: Instant, has_session_end: bool) {
        self.quiet_since = now;
        self.has_run = false;
        self.is_asked |= has_session_end;
    }

    /// When upkeep is due next, at `now` or later; none until the next hook event.
    pub(crate) fn due_at(&self, now: Instant) -> Option<Instant> {
        let due = if self.is_asked {
            now
        } else if self.has_run {
            return None;
        } else {
            self.quiet_since + QUIET_BEFORE_UPKEEP
        };
        Some(self.paced_until.map_or(due, |until| due.max(until)))
    }

    /// Notes that a person asked for upkeep now.
    pub(crate) fn ask(&mut self) {
        self.is_asked = true;
    }

    /// Notes how a run that ended at `now` ended.
    pub(crate) fn ran(&mut self, ended: Ended, now: Instant) {
        self.paced_until = (ended == Ended::Paced).then(|| now + PACE_WAIT);
        if matches!(ended, Ended::Done | Ended::OutOfTime) {
            self.has_run = true;
            self.is_asked = false;
            self.run_since = None;
        }
    }
}

/// Where upkeep is: the job it is on and how far into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Links of use, after this pair.
    Links(Option<(NodeId, NodeId)>),
    /// What tasks read next, after this pair.
    Transitions(Option<(NodeId, NodeId)>),
    /// Interest, after this node.
    Interest(Option<NodeId>),
    /// The facts.
    Facts,
    /// The confidence model, fitted when one is due.
    Calibration,
    /// Old events.
    Events,
    /// Old injections.
    Injections,
    /// Episodes past the cap.
    Episodes,
    /// Positions that no longer hold.
    Positions,
    /// The planner's statistics.
    Optimize,
    /// The free pages, this many given back so far.
    Vacuum(u64),
}

/// Upkeep's place between runs.
#[derive(Debug)]
pub struct Upkeep {
    /// The next batch to run.
    step: Step,
    /// Whether the last run paced and no batch has run since: the next run writes a batch before
    /// it looks at the log, since SQLite cuts the log's file back only on a commit after the log
    /// starts afresh, and a run that looked first would pace again without writing.
    has_paced: bool,
}

impl Step {
    /// The first step of a pass.
    const FIRST: Self = Self::Links(None);

    /// The step's name, as the store keeps it: where in a step a run stopped is not kept, so a
    /// daemon started later carries on from the step's start.
    fn name(self) -> &'static str {
        match self {
            Self::Links(_) => "links",
            Self::Transitions(_) => "transitions",
            Self::Interest(_) => "interest",
            Self::Facts => "facts",
            Self::Calibration => "calibration",
            Self::Events => "events",
            Self::Injections => "injections",
            Self::Episodes => "episodes",
            Self::Positions => "positions",
            Self::Optimize => "optimize",
            Self::Vacuum(_) => "vacuum",
        }
    }

    /// The start of the step named `name`; none for a name this build does not give a step.
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "links" => Self::Links(None),
            "transitions" => Self::Transitions(None),
            "interest" => Self::Interest(None),
            "facts" => Self::Facts,
            "calibration" => Self::Calibration,
            "events" => Self::Events,
            "injections" => Self::Injections,
            "episodes" => Self::Episodes,
            "positions" => Self::Positions,
            "optimize" => Self::Optimize,
            "vacuum" => Self::Vacuum(0),
            _ => return None,
        })
    }

    /// The start of the step after this one; none after the last.
    fn following(self) -> Option<Self> {
        Some(match self {
            Self::Links(_) => Self::Transitions(None),
            Self::Transitions(_) => Self::Interest(None),
            Self::Interest(_) => Self::Facts,
            Self::Facts => Self::Calibration,
            Self::Calibration => Self::Events,
            Self::Events => Self::Injections,
            Self::Injections => Self::Episodes,
            Self::Episodes => Self::Positions,
            Self::Positions => Self::Optimize,
            Self::Optimize => Self::Vacuum(0),
            Self::Vacuum(_) => return None,
        })
    }
}

impl Default for Upkeep {
    /// Upkeep at its first job.
    fn default() -> Self {
        Self {
            step: Step::FIRST,
            has_paced: false,
        }
    }
}

impl Upkeep {
    /// Upkeep where an earlier daemon left it, by the step `queries` reads kept: at that step's
    /// start; at the first step when none is kept, or the name kept is one this build does not
    /// give a step (another version's).
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn resumed(queries: Queries<'_>) -> Result<Self, StoreError> {
        let kept = queries.meta(MetaKey::UpkeepStep)?;
        Ok(Self {
            step: kept
                .as_deref()
                .and_then(Step::from_name)
                .unwrap_or(Step::FIRST),
            has_paced: false,
        })
    }

    /// Runs upkeep at `now_ms` with `writer`, from where the last run stopped, until every job is
    /// done, `deadline` passes, `should_stop` says other work waits or the write-ahead log holds
    /// more than `settings` lets it: what it removed, and which of these ended it.
    ///
    /// # Errors
    ///
    /// The store's errors; the batch that failed is rolled back, and the next run carries on from
    /// the step after it, so that one step that keeps failing leaves the others to run: the failed
    /// step is tried again on the next pass.
    pub fn run(
        &mut self,
        writer: &mut Writer,
        (now_ms, settings): (i64, &UpkeepSettings),
        (deadline, should_stop): (Instant, &dyn Fn() -> bool),
    ) -> Result<(Swept, Ended), StoreError> {
        let mut swept = Swept::default();
        loop {
            if should_stop() {
                return Ok((swept, Ended::Yielded));
            }
            if Instant::now() >= deadline {
                return Ok((swept, Ended::OutOfTime));
            }
            if !self.has_paced && writer.log_bytes()? > settings.log_limit_bytes {
                self.has_paced = true;
                return Ok((swept, Ended::Paced));
            }
            let next = match self.batch(writer, (now_ms, settings), &mut swept) {
                Ok(next) => next,
                Err(error) => {
                    self.step = self.step.following().unwrap_or(Step::FIRST);
                    self.has_paced = false;
                    return Err(error);
                }
            };
            self.has_paced = false;
            match next {
                Some(next) => self.step = next,
                None => {
                    self.step = Step::FIRST;
                    return Ok((swept, Ended::Done));
                }
            }
        }
    }

    /// Runs the batch upkeep is at, adding what it removed to `swept`, and keeps in the store the
    /// step to carry on from: the step after it, none once the last job is done.
    fn batch(
        &self,
        writer: &mut Writer,
        (now_ms, settings): (i64, &UpkeepSettings),
        swept: &mut Swept,
    ) -> Result<Option<Step>, StoreError> {
        let mut transaction = writer.transaction()?;
        let next = match self.step {
            Step::Links(after) => {
                let page = transaction
                    .queries()
                    .coaccess_links_after(after, BATCH_ROWS)?;
                let latest = transaction.queries().latest_task_seq()?;
                let faded: Vec<(NodeId, NodeId)> = page
                    .iter()
                    .filter(|link| {
                        has_faded_out(link.weight, link.last_seq, latest, settings.link_decay)
                    })
                    .map(|link| (link.src, link.dst))
                    .collect();
                swept.links += transaction.remove_coaccess_links(&faded)?;
                match page.last() {
                    Some(last) if is_full(page.len()) => {
                        Some(Step::Links(Some((last.src, last.dst))))
                    }
                    _ => self.step.following(),
                }
            }
            Step::Transitions(after) => {
                let page = transaction.queries().transitions_after(after, BATCH_ROWS)?;
                let latest = transaction.queries().latest_task_seq()?;
                let faded: Vec<(NodeId, NodeId)> = page
                    .iter()
                    .filter(|way| {
                        has_faded_out(way.weight, way.last_seq, latest, settings.link_decay)
                    })
                    .map(|way| (way.src, way.dst))
                    .collect();
                swept.transitions += transaction.remove_transitions(&faded)?;
                match page.last() {
                    Some(last) if is_full(page.len()) => {
                        Some(Step::Transitions(Some((last.src, last.dst))))
                    }
                    _ => self.step.following(),
                }
            }
            Step::Interest(after) => {
                let page = transaction.queries().interest_after(after, BATCH_ROWS)?;
                let latest = transaction.queries().latest_task_seq()?;
                let faded: Vec<NodeId> = page
                    .iter()
                    .filter(|row| {
                        has_faded_out(row.score, row.last_seq, latest, settings.interest_decay)
                    })
                    .map(|row| row.node)
                    .collect();
                swept.interest += transaction.remove_interest(&faded)?;
                match page.last() {
                    Some(last) if is_full(page.len()) => Some(Step::Interest(Some(last.node))),
                    _ => self.step.following(),
                }
            }
            Step::Facts => {
                review_facts(&mut transaction, now_ms)?;
                let deleted = transaction
                    .delete_retired_facts_before(now_ms.saturating_sub(KEPT_FOR_MS), BATCH_ROWS)?;
                swept.facts += deleted;
                if deleted == BATCH_ROWS {
                    Some(Step::Facts)
                } else {
                    self.step.following()
                }
            }
            Step::Calibration => {
                match calibrate(&mut transaction, now_ms)? {
                    Some(Calibrated::Kept(fitted)) => tracing::info!(
                        auc = fitted.auc,
                        samples = fitted.samples,
                        "confidence model kept"
                    ),
                    Some(Calibrated::Refused(refusal)) => {
                        tracing::info!(?refusal, "confidence model not kept");
                    }
                    None => {}
                }
                self.step.following()
            }
            // A batch looks at the oldest rows alone, so a row newer than the rest (a clock that
            // went back) keeps a batch from filling: the step goes on while a batch deletes any.
            Step::Events => {
                let deleted = transaction
                    .delete_events_before(now_ms.saturating_sub(KEPT_FOR_MS), BATCH_ROWS)?;
                swept.events += deleted;
                if deleted > 0 {
                    Some(Step::Events)
                } else {
                    self.step.following()
                }
            }
            Step::Injections => {
                let deleted = transaction
                    .delete_injections_before(now_ms.saturating_sub(KEPT_FOR_MS), BATCH_ROWS)?;
                swept.injections += deleted;
                if deleted > 0 {
                    Some(Step::Injections)
                } else {
                    self.step.following()
                }
            }
            Step::Episodes => {
                let deleted = transaction.cap_episodes(MAX_EPISODES, BATCH_ROWS)?;
                swept.episodes += deleted;
                if deleted == BATCH_ROWS {
                    Some(Step::Episodes)
                } else {
                    self.step.following()
                }
            }
            Step::Positions => {
                swept.positions += transaction
                    .delete_positions_before(now_ms.saturating_sub(settings.longest_hold_ms))?;
                self.step.following()
            }
            Step::Optimize => {
                transaction.optimize()?;
                self.step.following()
            }
            Step::Vacuum(given) => {
                let freed = transaction.vacuum(VACUUM_BATCH_PAGES)?;
                swept.pages += freed;
                let given = given + freed;
                if freed < u64::from(VACUUM_BATCH_PAGES) || given >= VACUUM_PAGES {
                    None
                } else {
                    Some(Step::Vacuum(given))
                }
            }
        };
        transaction.set_upkeep_step(next.unwrap_or(Step::FIRST).name())?;
        transaction.commit()?;
        Ok(next)
    }
}

/// Whether a page of `rows` was full, so that more may follow it.
fn is_full(rows: usize) -> bool {
    u64::try_from(rows).is_ok_and(|rows| rows >= BATCH_ROWS)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use cortex_core::{
        Agent, ContentHash, EdgeType, EventKind, EventMeta, GateFeatures, HookKind, Identifier,
        RepoPath,
    };
    use cortex_store::{EdgeRecord, EventRecord, InjectionRecord, SessionRecord, TransitionRow};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// A day in milliseconds.
    const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

    /// Links fade by half for each task; interest too; positions hold a minute at the longest;
    /// the log may hold 32 MiB.
    const SETTINGS: UpkeepSettings = UpkeepSettings {
        link_decay: 0.5,
        interest_decay: 0.5,
        longest_hold_ms: 60_000,
        log_limit_bytes: LOG_LIMIT_BYTES,
    };

    /// A store holding `a.py`, `b.py` and `c.py`, links of use `a`-`b` 0.004 and `a`-`c` 0.5 (no
    /// task has run, so neither has faded since), and events 100 days and 1 day before day 200.
    fn store(root: &std::path::Path) -> TestResult<Writer> {
        let mut writer = writer(root)?;
        let a = add_file(&mut writer, "a.py", 1)?;
        let b = add_file(&mut writer, "b.py", 1)?;
        let c = add_file(&mut writer, "c.py", 1)?;
        let mut transaction = writer.transaction()?;
        let link = |dst, weight| EdgeRecord {
            dst,
            weight,
            count: 1,
            last_seq: 0,
            is_directed: false,
        };
        transaction.set_edges(
            EdgeType::CoAccess,
            &[(a, link(b, 0.004)), (a, link(c, 0.5))],
        )?;
        for day in [100, 199] {
            transaction.append_event(&EventRecord {
                task_id: None,
                session_id: None,
                ts_ms: day * DAY_MS,
                kind: EventKind::Read,
                node_id: None,
                path: None,
                range: None,
                meta: EventMeta::default(),
            })?;
        }
        transaction.commit()?;
        Ok(writer)
    }

    /// Worked by hand, at day 200: the link of 0.004 is below 0.01 and goes, the one of 0.5 stays;
    /// so with the ways tasks read on from `a.py`, 0.004 to `b.py` and 0.5 to `c.py` (no task has
    /// run, so neither has faded); the event of day 100 is older than 90 days and goes, the one of
    /// day 199 stays. Nothing else is there to remove. The run finishes.
    #[test]
    fn a_run_removes_what_faded_and_what_is_too_old() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = store(root.path())?;
        let nodes: Vec<NodeId> = writer
            .queries()
            .coaccess_links_after(None, 9)?
            .iter()
            .flat_map(|link| [link.src, link.dst])
            .collect();
        let [a, b, .., c] = nodes[..] else {
            return Err("the links' nodes".into());
        };
        let way = |dst, weight| TransitionRow {
            src: a,
            dst,
            weight,
            count: 1,
            last_seq: 0,
        };
        let mut transaction = writer.transaction()?;
        transaction.set_transitions(&[way(b, 0.004), way(c, 0.5)])?;
        transaction.commit()?;
        let mut upkeep = Upkeep::default();
        let far = Instant::now() + Duration::from_secs(60);
        let (swept, ended) =
            upkeep.run(&mut writer, (200 * DAY_MS, &SETTINGS), (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        assert_eq!(
            swept,
            Swept {
                links: 1,
                transitions: 1,
                events: 1,
                ..Swept::default()
            }
        );
        assert_eq!(writer.queries().coaccess_links_after(None, 9)?.len(), 1);
        assert_eq!(writer.queries().transitions_after(None, 9)?, [way(c, 0.5)]);
        Ok(())
    }

    /// A run told other work waits after its first batch stops there, the link removed and the
    /// rest left, and the store keeps `transitions` as the step to carry on from, where upkeep
    /// resumed from the store (a daemon started later) stands; the next run carries on from the
    /// second batch and finishes, removing the old event, and a third run finds nothing left.
    #[test]
    fn a_run_stops_when_other_work_waits_and_the_next_carries_on() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = store(root.path())?;
        let mut upkeep = Upkeep::default();
        let far = Instant::now() + Duration::from_secs(60);
        let asked = Cell::new(0);
        let after_one = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        let now = (200 * DAY_MS, &SETTINGS);
        let (first, ended) = upkeep.run(&mut writer, now, (far, &after_one))?;
        assert_eq!(ended, Ended::Yielded);
        assert_eq!(
            first,
            Swept {
                links: 1,
                ..Swept::default()
            }
        );
        assert_eq!(
            writer.queries().meta(MetaKey::UpkeepStep)?.as_deref(),
            Some("transitions")
        );
        let mut upkeep = Upkeep::resumed(writer.queries())?;
        assert_eq!(upkeep.step, Step::Transitions(None));
        let (second, ended) = upkeep.run(&mut writer, now, (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        assert_eq!(
            second,
            Swept {
                events: 1,
                ..Swept::default()
            }
        );
        let (third, ended) = upkeep.run(&mut writer, now, (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        assert_eq!(third, Swept::default());
        Ok(())
    }

    /// A step that fails leaves the steps after it to run. The fitting's last attempt kept in the
    /// store as `not json` makes the calibration step fail: the first run removes the link, then
    /// stops with the error; the next run carries on from the events and removes the old one,
    /// and finishes. The pass after that tries the fitting again (and fails again).
    #[test]
    fn a_step_that_fails_leaves_the_steps_after_it_to_run() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = store(root.path())?;
        rusqlite::Connection::open(root.path().join(".cortex").join("cortex.db"))?.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('calibration_attempt', 'not json')",
            [],
        )?;
        let mut upkeep = Upkeep::default();
        let far = Instant::now() + Duration::from_secs(60);
        let now = (200 * DAY_MS, &SETTINGS);
        assert!(upkeep.run(&mut writer, now, (far, &|| false)).is_err());
        assert_eq!(upkeep.step, Step::Events);
        let (swept, ended) = upkeep.run(&mut writer, now, (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        assert_eq!(
            swept,
            Swept {
                events: 1,
                ..Swept::default()
            }
        );
        assert!(upkeep.run(&mut writer, now, (far, &|| false)).is_err());
        Ok(())
    }

    /// Old events go batch after batch while a batch deletes any, even when a newer event sits
    /// among the oldest. By hand: an event of day 300 recorded first, then 600 of day 100; at day
    /// 200 the first batch, the oldest 500 recorded, deletes 499 (all but day 300's); the second,
    /// the 101 left with it, deletes 101; the third deletes none and the step ends: 600 in all.
    #[test]
    fn old_events_go_while_a_batch_deletes_any() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let mut transaction = writer.transaction()?;
        for day in std::iter::once(300).chain(std::iter::repeat_n(100, 600)) {
            transaction.append_event(&EventRecord {
                task_id: None,
                session_id: None,
                ts_ms: day * DAY_MS,
                kind: EventKind::Read,
                node_id: None,
                path: None,
                range: None,
                meta: EventMeta::default(),
            })?;
        }
        transaction.commit()?;
        let far = Instant::now() + Duration::from_secs(60);
        let (swept, ended) =
            Upkeep::default().run(&mut writer, (200 * DAY_MS, &SETTINGS), (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        assert_eq!(swept.events, 600);
        Ok(())
    }

    /// Worked by hand: quiet from 0, upkeep is due at 10 minutes; an event at 3 minutes moves it
    /// to 13; a run done leaves none due until the next event; a session's end asks for one at
    /// once; a run that yielded leaves it due at once; one out of time does not; asked, it is due
    /// at once; a run that paced at 19 minutes leaves it due 2 s later, and one that paced in a
    /// quiet spell too.
    #[test]
    fn upkeep_is_due_after_a_quiet_spell_or_when_asked() {
        let start = Instant::now();
        let minutes = |count: u64| start + Duration::from_secs(count * 60);
        let mut clock = UpkeepClock::new(start);
        assert_eq!(clock.due_at(start), Some(minutes(10)));
        clock.heard(minutes(3), false);
        assert_eq!(clock.due_at(minutes(3)), Some(minutes(13)));
        clock.ran(Ended::Done, minutes(13));
        assert_eq!(clock.due_at(minutes(14)), None);
        clock.heard(minutes(15), true);
        assert_eq!(clock.due_at(minutes(15)), Some(minutes(15)));
        clock.ran(Ended::Yielded, minutes(16));
        assert_eq!(clock.due_at(minutes(16)), Some(minutes(16)));
        clock.ran(Ended::OutOfTime, minutes(17));
        assert_eq!(clock.due_at(minutes(17)), None);
        clock.ask();
        assert_eq!(clock.due_at(minutes(18)), Some(minutes(18)));
        let two_seconds_on = minutes(19) + Duration::from_secs(2);
        clock.ran(Ended::Paced, minutes(19));
        assert_eq!(clock.due_at(minutes(19)), Some(two_seconds_on));
        clock.ran(Ended::Done, two_seconds_on);
        clock.heard(minutes(20), false);
        clock.ran(Ended::Paced, minutes(30));
        assert_eq!(
            clock.due_at(minutes(30)),
            Some(minutes(30) + Duration::from_secs(2))
        );
    }

    /// One run keeps one budget through its pieces. Worked by hand: a run starting at 40 minutes
    /// must stop by 42; paced at 41 and carried on at 41 and 2 s, then yielded and carried on at
    /// 41 and a half, it must still stop by 42. Out of time at 42, the next run, at 50, must stop
    /// by 52; done at 51, the next, at 60, by 62.
    #[test]
    fn a_run_keeps_one_budget_through_its_pieces() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut clock = UpkeepClock::new(start);
        assert_eq!(clock.run_deadline(at(40 * 60)), at(42 * 60));
        clock.ran(Ended::Paced, at(41 * 60));
        assert_eq!(clock.run_deadline(at(41 * 60 + 2)), at(42 * 60));
        clock.ran(Ended::Yielded, at(41 * 60 + 20));
        assert_eq!(clock.run_deadline(at(41 * 60 + 30)), at(42 * 60));
        clock.ran(Ended::OutOfTime, at(42 * 60));
        assert_eq!(clock.run_deadline(at(50 * 60)), at(52 * 60));
        clock.ran(Ended::Done, at(51 * 60));
        assert_eq!(clock.run_deadline(at(60 * 60)), at(62 * 60));
    }

    /// 250 labelled injections of prompts, each used exactly when its top lexical score, (37 n mod
    /// 100) / 100 for the n-th, is above 0.5: a run fits a model on the oldest 200 and judges it on
    /// the newest 50, which it ranks perfectly by that score, an AUC of 1, so it keeps it as the
    /// first. With nothing labelled since, the next run fits nothing.
    #[test]
    fn a_run_fits_the_confidence_model_once_enough_are_labelled() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let mut transaction = writer.transaction()?;
        transaction.record_session(&SessionRecord {
            session_id: Identifier::new("s0")?,
            agent: Agent::Claude,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 1,
        })?;
        for at in 0..250_u32 {
            let top_lexical = f64::from(at * 37 % 100) / 100.0;
            let id = transaction.record_injection(&InjectionRecord {
                task_id: None,
                session_id: Identifier::new("s0")?,
                hook: HookKind::Prompt,
                at_ms: 10 + i64::from(at),
                files: Vec::new(),
                tokens: 10,
                confidence: None,
                is_gated_out: false,
                is_holdout: false,
                text_hash: Some(ContentHash::of(b"a work order")),
                features: Some(GateFeatures {
                    top_lexical,
                    top_mass: 0.5,
                    agreeing_kinds: 2,
                    supporting_kinds: 2,
                    similar_task: 0.0,
                    recent_precision: None,
                    is_continuing: false,
                }),
            })?;
            transaction.label_injection(id, 0, top_lexical > 0.5)?;
        }
        transaction.commit()?;
        let mut upkeep = Upkeep::default();
        let far = Instant::now() + Duration::from_secs(60);
        let now = (200 * DAY_MS, &SETTINGS);
        let (_, ended) = upkeep.run(&mut writer, now, (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        let kept = writer.queries().calibration()?.ok_or("a model kept")?;
        assert_eq!((kept.version, kept.samples), (1, 200));
        assert!((kept.auc - 1.0).abs() < f64::EPSILON, "{kept:?}");
        let (_, ended) = upkeep.run(&mut writer, now, (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        assert_eq!(
            writer.queries().calibration()?.map(|model| model.version),
            Some(1)
        );
        Ok(())
    }

    /// The log is held to a limit of nothing at all, so that it always holds too much. A run that
    /// yields after its first batch removes the faded link. The next run looks at the log before
    /// writing and paces at once, at the same step; the one after a pace writes a batch first
    /// (what tasks read next, which holds nothing) and then paces. With the usual limit the next run carries
    /// on and finishes, removing the old event.
    #[test]
    fn a_run_waits_while_the_log_holds_too_much() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = store(root.path())?;
        assert!(writer.log_bytes()? > 0);
        let mut upkeep = Upkeep::default();
        let far = Instant::now() + Duration::from_secs(60);
        let now = (200 * DAY_MS, &SETTINGS);
        let asked = Cell::new(0);
        let after_one = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        let (swept, ended) = upkeep.run(&mut writer, now, (far, &after_one))?;
        assert_eq!(ended, Ended::Yielded);
        assert_eq!(
            swept,
            Swept {
                links: 1,
                ..Swept::default()
            }
        );
        let full = UpkeepSettings {
            log_limit_bytes: 0,
            ..SETTINGS
        };
        let full = (200 * DAY_MS, &full);
        let (swept, ended) = upkeep.run(&mut writer, full, (far, &|| false))?;
        assert_eq!((swept, ended), (Swept::default(), Ended::Paced));
        assert_eq!(upkeep.step, Step::Transitions(None));
        let (swept, ended) = upkeep.run(&mut writer, full, (far, &|| false))?;
        assert_eq!((swept, ended), (Swept::default(), Ended::Paced));
        assert_eq!(upkeep.step, Step::Interest(None));
        let (swept, ended) = upkeep.run(&mut writer, now, (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        assert_eq!(
            swept,
            Swept {
                events: 1,
                ..Swept::default()
            }
        );
        Ok(())
    }

    /// A store whose deleted events left more than 2,000 free pages (10,000 events of a 1,000-byte
    /// path each, about 10 MB, in 4 KiB pages). From the vacuum step, a run that yields after one
    /// batch gives back 500 pages and stays at the vacuum, 500 given; the next gives back the
    /// 1,500 the cap leaves, and is done with free pages still left.
    #[test]
    fn the_vacuum_gives_pages_back_500_at_a_time_up_to_2000() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let path = RepoPath::new(&format!("src/{}.py", "a".repeat(1_000)))?;
        let mut transaction = writer.transaction()?;
        for ts_ms in 0..10_000 {
            transaction.append_event(&EventRecord {
                task_id: None,
                session_id: None,
                ts_ms,
                kind: EventKind::Read,
                node_id: None,
                path: Some(path.clone()),
                range: None,
                meta: EventMeta::default(),
            })?;
        }
        transaction.commit()?;
        let mut transaction = writer.transaction()?;
        assert_eq!(transaction.delete_events_before(10_000, 10_000)?, 10_000);
        transaction.commit()?;
        assert!(writer.queries().free_pages()? > 2_000);
        let mut upkeep = Upkeep {
            step: Step::Vacuum(0),
            has_paced: false,
        };
        let far = Instant::now() + Duration::from_secs(60);
        let now = (DAY_MS, &SETTINGS);
        let asked = Cell::new(0);
        let after_one = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        let (swept, ended) = upkeep.run(&mut writer, now, (far, &after_one))?;
        assert_eq!((swept.pages, ended), (500, Ended::Yielded));
        assert_eq!(upkeep.step, Step::Vacuum(500));
        let (swept, ended) = upkeep.run(&mut writer, now, (far, &|| false))?;
        assert_eq!((swept.pages, ended), (1_500, Ended::Done));
        assert!(writer.queries().free_pages()? > 0);
        Ok(())
    }
}
