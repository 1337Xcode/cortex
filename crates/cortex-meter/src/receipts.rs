//! Receipts: what each task of a session took, in turns, tool calls, tokens and estimated cost.
//!
//! A task runs from the prompt that started it to the stop that ended it, or until the next task
//! started when it never ended. Each response the meter recorded, the session's own and its
//! subagents', belongs to the task whose span holds the time the transcript wrote it; a response
//! no span holds is counted apart, so that the session's total is every response it took.
//!
//! Tokens are counted as the transcripts report them: cumulative input is every input token, read
//! fresh, written to the cache or read from it; uncached input is the part read fresh and not
//! written to the cache. A cost is summed over the responses the price table can price, and the
//! rest are counted with the reason they have no price, never priced at a guess.

use std::collections::BTreeMap;

use cortex_core::{Identifier, TaskOutcome, Ulid};
use cortex_store::{SessionRow, TaskRow, UsageRow};
use serde::Serialize;

use crate::format::format_of;
use crate::pricing::{Billed, Cost, PriceTable, Unpriced};

/// How many tasks each arm needs before receipts compare held-out tasks with the rest: fewer
/// make a comparison too uncertain to state.
pub const MIN_TASKS_PER_ARM: u64 = 30;

/// How many of a repository's tasks were held out and how many were not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Arms {
    /// Tasks Cortex held back from.
    pub held_out: u64,
    /// Tasks Cortex could help.
    pub treated: u64,
}

impl Arms {
    /// Whether both arms have enough tasks to compare.
    #[must_use]
    pub fn is_enough(self) -> bool {
        self.held_out >= MIN_TASKS_PER_ARM && self.treated >= MIN_TASKS_PER_ARM
    }
}

/// One session's activity, as receipts read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    /// The session.
    pub session: SessionRow,
    /// How many subagent sessions it started whose responses `usage` holds.
    pub subagents: u64,
    /// Its tasks.
    pub tasks: Vec<TaskRow>,
    /// The responses of the session and of its subagents.
    pub usage: Vec<UsageRow>,
    /// How many tool calls each task made, its subagents' among them; a task left out made none.
    pub tool_calls: BTreeMap<Ulid, u64>,
    /// How many tool calls the session and its subagents made while it had no task.
    pub outside_tool_calls: u64,
}

/// What one session took, task by task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionReceipt {
    /// The session's id.
    pub session_id: Identifier,
    /// The agent that ran it.
    pub agent: &'static str,
    /// Whether Cortex reads its agent's transcripts. When it does not, the session's usage was
    /// never read, which is not the same as none: its tallies are empty and say nothing.
    pub is_metered: bool,
    /// When it started, in UTC milliseconds.
    pub started_at_ms: i64,
    /// When it ended, if it has.
    pub ended_at_ms: Option<i64>,
    /// Whether it still runs, ended, or stopped sending events without ending.
    pub status: &'static str,
    /// How many subagent sessions it started whose responses are counted here.
    pub subagents: u64,
    /// Every tool call it and its subagents made, in its tasks and outside them.
    pub tool_calls: u64,
    /// Its tasks, in the order they started.
    pub tasks: Vec<TaskReceipt>,
    /// The responses no task's span holds.
    pub outside_tasks: Tally,
    /// The tool calls made while it had no task.
    pub outside_tool_calls: u64,
    /// Every response of the session and its subagents.
    pub total: Tally,
}

/// What one task took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskReceipt {
    /// The task's id.
    pub task_id: Ulid,
    /// Its number in the repository's order of tasks.
    pub seq: u64,
    /// When it started, in UTC milliseconds.
    pub started_at_ms: i64,
    /// When it ended, if it has.
    pub ended_at_ms: Option<i64>,
    /// How it ended, once known.
    pub outcome: Option<&'static str>,
    /// Whether Cortex held back from it, once decided: held-out tasks are the ones receipts
    /// compare the rest with.
    pub held_out: Option<bool>,
    /// The tool calls it made, its subagents' among them.
    pub tool_calls: u64,
    /// The tokens Cortex added to its context. This version adds none to any task.
    pub injected_tokens: u64,
    /// Its responses.
    pub usage: Tally,
}

/// The responses of a task, or of a session, added up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tally {
    /// How many responses.
    pub turns: u64,
    /// Every input token: read fresh, written to the cache and read from it.
    pub cumulative_input: u64,
    /// Input read fresh and not written to the cache.
    pub uncached_input: u64,
    /// Input read from the cache.
    pub cache_reads: u64,
    /// Input written to the cache.
    pub cache_writes: u64,
    /// Output, reasoning included.
    pub output: u64,
    /// The estimated cost, or `None` when there is no price table to estimate from.
    pub estimate: Option<Estimate>,
}

/// An estimated cost and the responses it leaves out.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Estimate {
    /// The cost of the responses that have a price, in picodollars.
    pub picodollars: u128,
    /// How many responses have a price.
    pub priced_turns: u64,
    /// How many responses have no price, by the reason why.
    pub unpriced_turns: BTreeMap<Unpriced, u64>,
}

/// The receipt of `activity`, with costs from `prices` when there is a price table.
#[must_use]
pub fn receipt(activity: &Activity, prices: Option<&PriceTable>) -> SessionReceipt {
    let mut tasks: Vec<&TaskRow> = activity.tasks.iter().collect();
    tasks.sort_by_key(|task| (task.record.started_at_ms, task.seq));
    let mut tallies: Vec<Tally> = tasks.iter().map(|_| Tally::new(prices)).collect();
    let mut outside_tasks = Tally::new(prices);
    let mut total = Tally::new(prices);
    for row in &activity.usage {
        let tally = task_holding(&tasks, row.at_ms)
            .and_then(|index| tallies.get_mut(index))
            .unwrap_or(&mut outside_tasks);
        tally.add(row, prices);
        total.add(row, prices);
    }
    let tasks: Vec<TaskReceipt> = tasks
        .iter()
        .zip(tallies)
        .map(|(task, usage)| task_receipt(task, usage, &activity.tool_calls))
        .collect();
    let session = &activity.session;
    SessionReceipt {
        session_id: session.record.session_id.clone(),
        agent: session.record.agent.name(),
        is_metered: format_of(session.record.agent).is_some(),
        started_at_ms: session.record.started_at_ms,
        ended_at_ms: session.ended_at_ms,
        status: session.status.name(),
        subagents: activity.subagents,
        tool_calls: tasks
            .iter()
            .map(|task| task.tool_calls)
            .fold(activity.outside_tool_calls, u64::saturating_add),
        tasks,
        outside_tasks,
        outside_tool_calls: activity.outside_tool_calls,
        total,
    }
}

/// The receipt of `task`, whose responses add up to `usage`, with its count in `tool_calls`.
fn task_receipt(task: &TaskRow, usage: Tally, tool_calls: &BTreeMap<Ulid, u64>) -> TaskReceipt {
    TaskReceipt {
        task_id: task.record.task_id,
        seq: task.seq,
        started_at_ms: task.record.started_at_ms,
        ended_at_ms: task.ended_at_ms,
        outcome: task.outcome.map(TaskOutcome::name),
        held_out: task.holdout,
        tool_calls: tool_calls.get(&task.record.task_id).copied().unwrap_or(0),
        injected_tokens: 0,
        usage,
    }
}

/// The index in `tasks`, sorted by start, of the task whose span holds `at_ms`: the latest to
/// start at or before it, unless that task ended before it.
fn task_holding(tasks: &[&TaskRow], at_ms: i64) -> Option<usize> {
    let index = tasks
        .partition_point(|task| task.record.started_at_ms <= at_ms)
        .checked_sub(1)?;
    match tasks.get(index)?.ended_at_ms {
        Some(ended_at_ms) if ended_at_ms < at_ms => None,
        _ => Some(index),
    }
}

impl Tally {
    /// No response yet, with an empty estimate when there are `prices`.
    fn new(prices: Option<&PriceTable>) -> Self {
        Self {
            turns: 0,
            cumulative_input: 0,
            uncached_input: 0,
            cache_reads: 0,
            cache_writes: 0,
            output: 0,
            estimate: prices.map(|_| Estimate::default()),
        }
    }

    /// Adds the response `row`, priced from `prices`.
    fn add(&mut self, row: &UsageRow, prices: Option<&PriceTable>) {
        self.turns += 1;
        self.cumulative_input = self.cumulative_input.saturating_add(
            row.input
                .saturating_add(row.cache_write)
                .saturating_add(row.cache_read),
        );
        self.uncached_input = self.uncached_input.saturating_add(row.input);
        self.cache_reads = self.cache_reads.saturating_add(row.cache_read);
        self.cache_writes = self.cache_writes.saturating_add(row.cache_write);
        self.output = self.output.saturating_add(row.output);
        if let (Some(estimate), Some(prices)) = (self.estimate.as_mut(), prices) {
            match price(row, prices) {
                Ok(cost) => {
                    estimate.picodollars = estimate.picodollars.saturating_add(cost.picodollars());
                    estimate.priced_turns += 1;
                }
                Err(reason) => *estimate.unpriced_turns.entry(reason).or_insert(0) += 1,
            }
        }
    }
}

/// The estimated cost of the response `row`, or why it has none.
fn price(row: &UsageRow, prices: &PriceTable) -> Result<Cost, Unpriced> {
    let model = row.model.as_ref().ok_or(Unpriced::Model)?;
    prices.estimate(&Billed {
        model: model.as_str(),
        input: row.input,
        cache_write: row.cache_write,
        cache_write_1h: row.cache_write_1h,
        cache_read: row.cache_read,
        output: row.output,
        service_tier: row.service_tier.as_ref().map(Identifier::as_str),
        speed: row.speed.as_ref().map(Identifier::as_str),
        inference_geo: row.inference_geo.as_ref().map(Identifier::as_str),
    })
}
