//! Upkeep: the daemon's housekeeping, run on the writer's own thread (one core, and never a second
//! writer) once the repository has been quiet for [`QUIET_BEFORE_UPKEEP`] or a session ended. Its
//! jobs run in order, each a batch at a time in a transaction of its own: links of use that have
//! faded below 0.01 are removed, then interest that has; the facts are reviewed and those retired
//! for 90 days deleted; events older than 90 days are deleted, episodes kept to 50,000 (unknown
//! outcomes first) and positions that can no longer hold removed. Between batches a run stops when
//! a hook event or a request waits, or after [`UPKEEP_RUN_BUDGET`], and the next run carries on
//! where it stopped.

use std::time::{Duration, Instant};

use cortex_learn::{has_faded_out, review_facts};
use cortex_store::{NodeId, StoreError, Writer};

/// How long the repository must be quiet, no hook event recorded, before upkeep runs.
pub(crate) const QUIET_BEFORE_UPKEEP: Duration = Duration::from_secs(10 * 60);

/// The longest one run of upkeep goes on before it stops and leaves the rest for the next.
pub(crate) const UPKEEP_RUN_BUDGET: Duration = Duration::from_secs(120);

/// Rows weighed or deleted in one batch: a transaction short enough that a hook event waiting
/// behind it is recorded within milliseconds.
const BATCH_ROWS: u64 = 500;

/// How long events, and facts retired, are kept: 90 days, in milliseconds.
const KEPT_FOR_MS: i64 = 90 * 24 * 60 * 60 * 1_000;

/// The most episodes kept.
const MAX_EPISODES: u64 = 50_000;

/// What upkeep weighs with: the settings that say how links and interest fade, and how long a
/// position can hold at the longest.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UpkeepSettings {
    /// `learning.link_decay`.
    pub(crate) link_decay: f64,
    /// `learning.interest_decay`.
    pub(crate) interest_decay: f64,
    /// The longest any position holds, in milliseconds.
    pub(crate) longest_hold_ms: i64,
}

/// What a run removed or changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Swept {
    /// Links of use removed.
    pub(crate) links: u64,
    /// Nodes whose interest was removed.
    pub(crate) interest: u64,
    /// Facts deleted.
    pub(crate) facts: u64,
    /// Events deleted.
    pub(crate) events: u64,
    /// Episodes deleted.
    pub(crate) episodes: u64,
    /// Positions removed.
    pub(crate) positions: u64,
}

/// Why a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    /// Every job is done.
    Done,
    /// Other work waits: the run carries on once it is done.
    Yielded,
    /// The run used its budget: the next quiet spell carries on.
    OutOfTime,
}

/// When upkeep is due: once the repository has been quiet for [`QUIET_BEFORE_UPKEEP`] since the
/// last hook event, again only after another event; at once after a session ended or when asked;
/// and again at once after a run that yielded to other work.
#[derive(Debug)]
pub(crate) struct UpkeepClock {
    /// When the last hook event was recorded.
    quiet_since: Instant,
    /// Whether a run ended, done or out of time, since then.
    has_run: bool,
    /// Whether a run is asked for now: a session ended, or a person asked.
    is_asked: bool,
}

impl UpkeepClock {
    /// A clock whose quiet spell starts at `now`.
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            quiet_since: now,
            has_run: false,
            is_asked: false,
        }
    }

    /// Notes hook events recorded at `now`, among them a session's end when `has_session_end`.
    pub(crate) fn heard(&mut self, now: Instant, has_session_end: bool) {
        self.quiet_since = now;
        self.has_run = false;
        self.is_asked |= has_session_end;
    }

    /// When upkeep is due next, at `now` or later; none until the next hook event.
    pub(crate) fn due_at(&self, now: Instant) -> Option<Instant> {
        if self.is_asked {
            return Some(now);
        }
        (!self.has_run).then(|| self.quiet_since + QUIET_BEFORE_UPKEEP)
    }

    /// Notes that a person asked for upkeep now.
    pub(crate) fn ask(&mut self) {
        self.is_asked = true;
    }

    /// Notes how a run ended.
    pub(crate) fn ran(&mut self, ended: Ended) {
        if ended != Ended::Yielded {
            self.has_run = true;
            self.is_asked = false;
        }
    }
}

/// Where upkeep is: the job it is on and how far into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Links of use, after this pair.
    Links(Option<(NodeId, NodeId)>),
    /// Interest, after this node.
    Interest(Option<NodeId>),
    /// The facts.
    Facts,
    /// Old events.
    Events,
    /// Episodes past the cap.
    Episodes,
    /// Positions that no longer hold.
    Positions,
}

/// Upkeep's place between runs.
#[derive(Debug)]
pub(crate) struct Upkeep {
    /// The next batch to run.
    step: Step,
}

impl Default for Upkeep {
    /// Upkeep at its first job.
    fn default() -> Self {
        Self {
            step: Step::Links(None),
        }
    }
}

impl Upkeep {
    /// Runs upkeep at `now_ms` with `writer`, from where the last run stopped, until every job is
    /// done, `deadline` passes or `should_stop` says other work waits: what it removed, and which
    /// of these ended it.
    ///
    /// # Errors
    ///
    /// The store's errors; the batch that failed is rolled back and is the next run's first.
    pub(crate) fn run(
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
            match self.batch(writer, (now_ms, settings), &mut swept)? {
                Some(next) => self.step = next,
                None => {
                    self.step = Step::Links(None);
                    return Ok((swept, Ended::Done));
                }
            }
        }
    }

    /// Runs the batch upkeep is at, adding what it removed to `swept`: the step after it, none
    /// once the last job is done.
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
                    Some(last) if is_full(page.len()) => Step::Links(Some((last.src, last.dst))),
                    _ => Step::Interest(None),
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
                    Some(last) if is_full(page.len()) => Step::Interest(Some(last.node)),
                    _ => Step::Facts,
                }
            }
            Step::Facts => {
                review_facts(&mut transaction, now_ms)?;
                let deleted = transaction
                    .delete_retired_facts_before(now_ms.saturating_sub(KEPT_FOR_MS), BATCH_ROWS)?;
                swept.facts += deleted;
                if deleted == BATCH_ROWS {
                    Step::Facts
                } else {
                    Step::Events
                }
            }
            Step::Events => {
                let deleted = transaction
                    .delete_events_before(now_ms.saturating_sub(KEPT_FOR_MS), BATCH_ROWS)?;
                swept.events += deleted;
                if deleted == BATCH_ROWS {
                    Step::Events
                } else {
                    Step::Episodes
                }
            }
            Step::Episodes => {
                let deleted = transaction.cap_episodes(MAX_EPISODES, BATCH_ROWS)?;
                swept.episodes += deleted;
                if deleted == BATCH_ROWS {
                    Step::Episodes
                } else {
                    Step::Positions
                }
            }
            Step::Positions => {
                swept.positions += transaction
                    .delete_positions_before(now_ms.saturating_sub(settings.longest_hold_ms))?;
                transaction.commit()?;
                return Ok(None);
            }
        };
        transaction.commit()?;
        Ok(Some(next))
    }
}

/// Whether a page of `rows` was full, so that more may follow it.
fn is_full(rows: usize) -> bool {
    u64::try_from(rows).is_ok_and(|rows| rows >= BATCH_ROWS)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use cortex_core::{EdgeType, EventKind, EventMeta};
    use cortex_store::{EdgeRecord, EventRecord};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// A day in milliseconds.
    const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

    /// Links fade by half for each task; interest too; positions hold a minute at the longest.
    const SETTINGS: UpkeepSettings = UpkeepSettings {
        link_decay: 0.5,
        interest_decay: 0.5,
        longest_hold_ms: 60_000,
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
    /// the event of day 100 is older than 90 days and goes, the one of day 199 stays. Nothing else
    /// is there to remove. The run finishes.
    #[test]
    fn a_run_removes_what_faded_and_what_is_too_old() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = store(root.path())?;
        let mut upkeep = Upkeep::default();
        let far = Instant::now() + Duration::from_secs(60);
        let (swept, ended) =
            upkeep.run(&mut writer, (200 * DAY_MS, &SETTINGS), (far, &|| false))?;
        assert_eq!(ended, Ended::Done);
        assert_eq!(
            swept,
            Swept {
                links: 1,
                events: 1,
                ..Swept::default()
            }
        );
        assert_eq!(writer.queries().coaccess_links_after(None, 9)?.len(), 1);
        Ok(())
    }

    /// A run told other work waits after its first batch stops there, the link removed and the
    /// rest left; the next run carries on from the second batch and finishes, removing the old
    /// event, and a third run finds nothing left.
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

    /// Worked by hand: quiet from 0, upkeep is due at 10 minutes; an event at 3 minutes moves it
    /// to 13; a run done leaves none due until the next event; a session's end asks for one at
    /// once; a run that yielded leaves it due at once; one out of time does not; asked, it is due
    /// at once.
    #[test]
    fn upkeep_is_due_after_a_quiet_spell_or_when_asked() {
        let start = Instant::now();
        let minutes = |count: u64| start + Duration::from_secs(count * 60);
        let mut clock = UpkeepClock::new(start);
        assert_eq!(clock.due_at(start), Some(minutes(10)));
        clock.heard(minutes(3), false);
        assert_eq!(clock.due_at(minutes(3)), Some(minutes(13)));
        clock.ran(Ended::Done);
        assert_eq!(clock.due_at(minutes(14)), None);
        clock.heard(minutes(15), true);
        assert_eq!(clock.due_at(minutes(15)), Some(minutes(15)));
        clock.ran(Ended::Yielded);
        assert_eq!(clock.due_at(minutes(16)), Some(minutes(16)));
        clock.ran(Ended::OutOfTime);
        assert_eq!(clock.due_at(minutes(17)), None);
        clock.ask();
        assert_eq!(clock.due_at(minutes(18)), Some(minutes(18)));
    }
}
