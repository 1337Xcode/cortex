//! Receipts added up by hand: which task each response belongs to, the token sums, the tool
//! calls, and estimates from the shipped price table with every unpriced response counted by its
//! reason.

use std::collections::BTreeMap;

use cortex_core::{Agent, ContentHash, HookKind, Identifier, SessionStatus, TaskOutcome, Ulid};
use cortex_meter::{
    Activity, Arms, Estimate, Precision, PriceTable, SHIPPED_PRICES, Tally, Unpriced, receipt,
};
use cortex_store::{InjectionRecord, SessionRecord, SessionRow, TaskRecord, TaskRow, UsageRow};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The session `s1` of Claude Code, started at 900 and still live.
fn session() -> TestResult<SessionRow> {
    Ok(SessionRow {
        record: SessionRecord {
            session_id: Identifier::new("s1")?,
            agent: Agent::Claude,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 900,
        },
        ended_at_ms: None,
        status: SessionStatus::Live,
    })
}

/// Task number `seq` of `s1`, from `started_at_ms` to `ended_at_ms` if it ended.
fn task(seq: u64, started_at_ms: i64, ended_at_ms: Option<i64>) -> TestResult<TaskRow> {
    Ok(TaskRow {
        record: TaskRecord {
            task_id: Ulid::from_seed(u64::try_from(started_at_ms)?, &seq.to_be_bytes()),
            session_id: Identifier::new("s1")?,
            started_at_ms,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: None,
        },
        seq,
        ended_at_ms,
        outcome: ended_at_ms.map(|_| TaskOutcome::Success),
        outcome_evidence: None,
    })
}

/// A response of Opus 5.5 written at `at_ms`, served at the standard tier and speed anywhere, with
/// `[input, cache writes, cache reads, output]` tokens and every cache write for five minutes.
fn response(
    at_ms: i64,
    [input, cache_write, cache_read, output]: [u64; 4],
) -> TestResult<UsageRow> {
    let name = |text: &str| Identifier::new(text).map(Some);
    Ok(UsageRow {
        session_id: Identifier::new("s1")?,
        turn: 1,
        at_ms,
        model: name("claude-opus-5-5")?,
        input,
        cache_write,
        cache_write_1h: Some(0),
        cache_read,
        output,
        reasoning: None,
        service_tier: name("standard")?,
        speed: name("standard")?,
        inference_geo: name("global")?,
        response: None,
    })
}

/// A tally of `[turns, cumulative input, uncached input, cache reads, cache writes, output]`.
fn tally(
    [
        turns,
        cumulative_input,
        uncached_input,
        cache_reads,
        cache_writes,
        output,
    ]: [u64; 6],
    estimate: Option<Estimate>,
) -> Tally {
    Tally {
        turns,
        cumulative_input,
        uncached_input,
        cache_reads,
        cache_writes,
        output,
        estimate,
    }
}

/// An estimate of `picodollars` over `priced_turns`, leaving out `unpriced` responses.
fn estimate(
    picodollars: u128,
    priced_turns: u64,
    unpriced: &[(Unpriced, u64)],
) -> Option<Estimate> {
    Some(Estimate {
        picodollars,
        priced_turns,
        unpriced_turns: unpriced.iter().copied().collect(),
    })
}

/// The first task runs from 1,000 to 2,000; the second starts at 3,000 and has not ended, and is
/// listed first. Seven responses, Opus 5.5 at $4, $5 and $8 for cache writes of five minutes and
/// an hour, $0.20 for cache reads and $20 for output per million tokens, so 4,000,000, 5,000,000,
/// 8,000,000, 200,000 and 20,000,000 picodollars a token:
///
/// - at 500, before any task: 10 input and 1 output, 40,000,000 + 20,000,000 = 60,000,000;
/// - at 1,000, the first task's start: 100 input, 1,000 written, 10 output,
///   400,000,000 + 5,000,000,000 + 200,000,000 = 5,600,000,000;
/// - at 1,500, a subagent's, no model named: 7 input and 3 output, unpriced;
/// - at 2,000, the first task's end: 2 input, 1,000 read, 5 output,
///   8,000,000 + 200,000,000 + 100,000,000 = 308,000,000;
/// - at 2,500, between the tasks: 1 input, 50 written for an untold time, 2 output, unpriced;
/// - at 3,000, the second task's start, kept in the US: 3 input, 100 written for an hour, 2,000
///   read, 20 output, (12,000,000 + 800,000,000 + 400,000,000 + 400,000,000) x 1.1 =
///   1,773,200,000;
/// - at 9,000,000, still in the open second task, at the priority tier: 4 input, 6 output,
///   unpriced.
///
/// The first task: 3 turns, input 1,100 + 7 + 1,002 = 2,109, uncached 100 + 7 + 2 = 109, 1,000
/// read, 1,000 written, 18 output, 5,908,000,000 over 2 turns. The second: 2 turns, input
/// 2,103 + 4 = 2,107, uncached 7, 2,000 read, 100 written, 26 output. Outside: 2 turns, input
/// 10 + 51 = 61, uncached 11, 50 written, 3 output. In all: 7 turns, 4,277 input, 127 uncached,
/// 3,000 read, 1,150 written, 47 output, 7,741,200,000 over 4 turns. The first task made 4 tool
/// calls and the session 2 outside any task: 6 in all.
#[test]
fn responses_add_up_by_task() -> TestResult {
    let prices = PriceTable::parse(SHIPPED_PRICES)?;
    let (first, second) = (task(1, 1_000, Some(2_000))?, task(2, 3_000, None)?);
    let subagent = UsageRow {
        session_id: Identifier::new("s1/agent-1")?,
        model: None,
        ..response(1_500, [7, 0, 0, 3])?
    };
    let untold = UsageRow {
        cache_write_1h: None,
        ..response(2_500, [1, 50, 0, 2])?
    };
    let us = UsageRow {
        cache_write_1h: Some(100),
        inference_geo: Some(Identifier::new("us")?),
        ..response(3_000, [3, 100, 2_000, 20])?
    };
    let priority = UsageRow {
        service_tier: Some(Identifier::new("priority")?),
        ..response(9_000_000, [4, 0, 0, 6])?
    };
    let activity = Activity {
        session: session()?,
        subagents: 1,
        tasks: vec![second.clone(), first.clone()],
        usage: vec![
            response(500, [10, 0, 0, 1])?,
            response(1_000, [100, 1_000, 0, 10])?,
            subagent,
            response(2_000, [2, 0, 1_000, 5])?,
            untold,
            us,
            priority,
        ],
        tool_calls: BTreeMap::from([(first.record.task_id, 4)]),
        outside_tool_calls: 2,
        injections: Vec::new(),
    };
    let receipt = receipt(&activity, Some(&prices));
    let tasks: Vec<_> = receipt
        .tasks
        .iter()
        .map(|task| {
            (
                task.seq,
                task.outcome,
                task.tool_calls,
                task.injected_tokens,
            )
        })
        .collect();
    assert_eq!(tasks, [(1, Some("success"), 4, 0), (2, None, 0, 0)]);
    let first_tally = tally(
        [3, 2_109, 109, 1_000, 1_000, 18],
        estimate(5_908_000_000, 2, &[(Unpriced::Model, 1)]),
    );
    let second_tally = tally(
        [2, 2_107, 7, 2_000, 100, 26],
        estimate(1_773_200_000, 1, &[(Unpriced::Tier, 1)]),
    );
    assert_eq!(receipt.tasks[0].usage, first_tally);
    assert_eq!(receipt.tasks[1].usage, second_tally);
    assert_eq!(
        receipt.outside_tasks,
        tally(
            [2, 61, 11, 0, 50, 3],
            estimate(60_000_000, 1, &[(Unpriced::CacheSplit, 1)])
        )
    );
    assert_eq!(
        receipt.total,
        tally(
            [7, 4_277, 127, 3_000, 1_150, 47],
            estimate(
                7_741_200_000,
                4,
                &[
                    (Unpriced::Model, 1),
                    (Unpriced::Tier, 1),
                    (Unpriced::CacheSplit, 1)
                ]
            )
        )
    );
    assert_eq!(
        (
            receipt.session_id.as_str(),
            receipt.agent,
            receipt.started_at_ms,
            receipt.status,
            receipt.subagents,
            receipt.tool_calls,
            receipt.outside_tool_calls
        ),
        ("s1", "claude", 900, "live", 1, 6, 2)
    );
    assert!(receipt.is_metered, "Cortex reads Claude Code's transcripts");
    assert_eq!(
        serde_json::to_value(&receipt.outside_tasks.estimate)?,
        serde_json::json!({
            "picodollars": 60_000_000,
            "priced_turns": 1,
            "unpriced_turns": {"cache_split": 1},
        })
    );
    Ok(())
}

/// With no price table there is no estimate anywhere, and the counts are the same; with no task,
/// every response is outside one.
#[test]
fn without_prices_or_tasks_counts_stand_alone() -> TestResult {
    let activity = Activity {
        session: session()?,
        subagents: 0,
        tasks: Vec::new(),
        usage: vec![
            response(1_000, [100, 1_000, 0, 10])?,
            response(2_000, [2, 0, 1_000, 5])?,
        ],
        tool_calls: BTreeMap::new(),
        outside_tool_calls: 0,
        injections: Vec::new(),
    };
    let receipt = receipt(&activity, None);
    let counts = tally([2, 2_102, 102, 1_000, 1_000, 15], None);
    assert!(receipt.tasks.is_empty());
    assert_eq!(receipt.outside_tasks, counts);
    assert_eq!(receipt.total, counts);
    Ok(())
}

/// A session of an agent whose transcripts Cortex does not read, Codex here, is not metered: its
/// usage was never read, though its tallies hold nothing as a session with no response does.
#[test]
fn an_agent_whose_transcripts_are_not_read_is_not_metered() -> TestResult {
    let mut codex = session()?;
    codex.record.agent = Agent::Codex;
    let activity = Activity {
        session: codex,
        subagents: 0,
        tasks: vec![task(1, 1_000, Some(2_000))?],
        usage: Vec::new(),
        tool_calls: BTreeMap::new(),
        outside_tool_calls: 0,
        injections: Vec::new(),
    };
    let receipt = receipt(&activity, None);
    assert_eq!((receipt.agent, receipt.is_metered), ("codex", false));
    assert_eq!(receipt.total, tally([0, 0, 0, 0, 0, 0], None));
    Ok(())
}

/// A task's receipt says which arm it is in once decided, and only then. Thirty tasks in each arm
/// are enough to compare; one fewer in either is not.
#[test]
fn arms_are_shown_and_counted_as_decided() -> TestResult {
    let decided = [Some(true), Some(false), None];
    let tasks = decided
        .iter()
        .zip(1..)
        .map(|(holdout, seq)| {
            let mut row = task(seq, i64::try_from(seq)? * 1_000, None)?;
            row.record.holdout = *holdout;
            Ok(row)
        })
        .collect::<TestResult<Vec<_>>>()?;
    let activity = Activity {
        session: session()?,
        subagents: 0,
        tasks,
        usage: Vec::new(),
        tool_calls: BTreeMap::new(),
        outside_tool_calls: 0,
        injections: Vec::new(),
    };
    let shown: Vec<Option<bool>> = receipt(&activity, None)
        .tasks
        .iter()
        .map(|task| task.held_out)
        .collect();
    assert_eq!(shown, decided);
    let arms = |held_out, treated| Arms { held_out, treated };
    assert!(arms(30, 30).is_enough());
    assert!(!arms(30, 29).is_enough());
    assert!(!arms(29, 30).is_enough());
    Ok(())
}

/// What Cortex injected counts only what reached the agent. Of a primer of 120 tokens given, a work
/// order of 300 given in task 1 and one of 90 there whose text was late, one of 200 the gate kept
/// back in task 2 and one of 150 held out there: task 1 was given 300, task 2 nothing, the
/// session's primers 120, and the session 120 + 300 = 420.
#[test]
fn only_what_reached_the_agent_counts_as_injected() -> TestResult {
    let (first, second) = (task(1, 1_000, Some(2_000))?, task(2, 3_000, None)?);
    let session_id = Identifier::new("s1")?;
    let given = |task_id: Option<Ulid>, hook: HookKind, tokens: u32| InjectionRecord {
        task_id,
        session_id: session_id.clone(),
        hook,
        at_ms: 1_000,
        files: Vec::new(),
        tokens,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the text given")),
        features: None,
    };
    let (one, two) = (first.record.task_id, second.record.task_id);
    let injections = vec![
        given(None, HookKind::SessionStart, 120),
        given(Some(one), HookKind::Prompt, 300),
        InjectionRecord {
            text_hash: None,
            ..given(Some(one), HookKind::Prompt, 90)
        },
        InjectionRecord {
            is_gated_out: true,
            text_hash: None,
            ..given(Some(two), HookKind::Prompt, 200)
        },
        InjectionRecord {
            is_holdout: true,
            ..given(Some(two), HookKind::Prompt, 150)
        },
    ];
    let activity = Activity {
        session: session()?,
        subagents: 0,
        tasks: vec![first, second],
        usage: Vec::new(),
        tool_calls: BTreeMap::new(),
        outside_tool_calls: 0,
        injections,
    };
    let receipt = receipt(&activity, None);
    let by_task: Vec<(u64, u64)> = receipt
        .tasks
        .iter()
        .map(|task| (task.seq, task.injected_tokens))
        .collect();
    assert_eq!(by_task, [(1, 300), (2, 0)]);
    assert_eq!((receipt.primer_tokens, receipt.injected_tokens), (120, 420));
    Ok(())
}

/// The precision is the share of the labelled work orders that were used: 3 of 4 is 0.75; with
/// none labelled there is none.
#[test]
fn precision_is_the_share_used() {
    let precision = Precision {
        labelled: 4,
        used: 3,
    };
    assert_eq!(precision.share(), Some(0.75));
    let none = Precision {
        labelled: 0,
        used: 0,
    };
    assert_eq!(none.share(), None);
}
