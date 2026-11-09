//! `cortex receipts` through the real binary: a captured Claude Code session whose transcript and
//! whose subagent's transcript are read at their stops, each response placed in the task whose span
//! holds it, with costs from the price table the command writes to the user's folder the first
//! time, as text and as JSON.

mod common;

use std::path::{Path, PathBuf};

use common::hook_call::send;
use common::{Daemon, TestResult, graph_of, payloads, receipts, wait_for};
use cortex_core::{Agent, Identifier, RepoId, TaskOutcome, Ulid, canonical_root};
use cortex_meter::{SHIPPED_PRICES, utc_text};
use cortex_store::{Reader, SessionRecord, TaskRecord, TaskRow, UsageRow, Writer, WriterOptions};
use serde_json::{Value, json};

/// The Claude Code version whose captured payloads and transcripts these tests use.
const CAPTURED: &str = "2.1.316";

/// The session of the captured payloads.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// The tokens of the primer the session's start is given, the repository being tiny: its two
/// lines, "[cortex] Project map · tiny mode · about 1 source file, 1 line" (65 bytes with its line
/// break, each middle dot two) and "Cortex records what agents do here; with this little code it
/// adds nothing to prompts." (86 bytes with its), 151 bytes at four bytes a token, rounded up.
const PRIMER_TOKENS: u64 = 38;

/// The subagent the captured payloads start, as a session of its own.
const SUBAGENT: &str = "00000000-0000-4000-8000-000000000001/fixtureagent0001";

/// A home folder holding a repository with one Python file, and the repository's root.
fn repository() -> TestResult<(tempfile::TempDir, PathBuf)> {
    let home = tempfile::tempdir()?;
    let repo = home.path().join("repo");
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join("a.py"), "a = 1\n")?;
    // No task is held out, so that the task's arm does not hang on its id.
    std::fs::write(
        repo.join(".cortex").join("config.toml"),
        "[holdout]\nshare = 0\n",
    )?;
    Ok((home, repo))
}

/// The tasks of `S1` the store of the repository at `root` holds.
fn tasks(root: &Path) -> TestResult<Vec<TaskRow>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    Ok(reader.queries().tasks_of_session(&Identifier::new(S1)?)?)
}

/// How many usage rows `session` has in the store of the repository at `root`.
fn turns(root: &Path, session: &str) -> TestResult<usize> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    Ok(reader
        .queries()
        .usage_of_session(&Identifier::new(session)?)?
        .len())
}

/// Copies the captured transcript `name` to `to` with each response's first line written at
/// another time: each `(from, to)` of `times` replaces the time `from` with the UTC time `to` ms.
fn moved_transcript(name: &str, to: &Path, times: &[(&str, i64)]) -> TestResult {
    payloads::copy_transcript(CAPTURED, name, to)?;
    let mut text = std::fs::read_to_string(to)?;
    for (from, at_ms) in times {
        let moved = utc_text(*at_ms).ok_or("a time after 1970")?;
        assert_eq!(text.matches(from).count(), 1, "{from}");
        text = text.replace(from, &moved);
    }
    std::fs::write(to, text)?;
    Ok(())
}

/// A session of one task, read through the binary. The prompt starts the task at T; the Grep
/// session's four responses are written a minute before T (outside the task), at T, T + 1 ms and
/// T + 2 ms, and the subagent's two at T + 1 ms and T + 2 ms; the session makes one Grep call and
/// its subagent another; then the subagent stops and the answer ends. Counts read off the captured
/// transcripts by hand (the meter's own test lists them), each response priced as Sonnet 5.5 at
/// the standard tier and speed, run anywhere: $2 input, $2.50 and $4 cache writes for five minutes
/// and an hour, $0.10 cache reads and $10 output per million tokens, so 2,000,000, 2,500,000,
/// 4,000,000, 100,000 and 10,000,000 picodollars a token.
///
/// - Outside the task, the first response: 3,702 input, 29,797 written for an hour, 100 output:
///   7,404,000,000 + 119,188,000,000 + 1,000,000,000 = 127,592,000,000 ($0.1276).
/// - In the task, the session's next three: 2 input, 3,819 written for an hour, 29,797 read, 95
///   output = 19,209,700,000; 2, 155, 33,616, 360 = 7,585,600,000; 2, 763, 33,771, 3 =
///   6,463,100,000. The subagent's two, written for five minutes: 2,716 input, 22,837 written,
///   148 output = 64,004,500,000; 2, 2,963 written, 22,837 read, 221 output = 11,905,200,000.
///   Five turns; input 33,618 + 33,773 + 34,536 + 25,553 + 25,802 = 153,282, of it 2,724
///   uncached, 120,021 read and 30,537 written; 827 output; 109,168,100,000 ($0.1092).
/// - The total: 6 turns, 186,781 input, 6,426 uncached, 120,021 read, 60,334 written, 927
///   output, 236,760,100,000 ($0.2368); two tool calls.
#[test]
fn receipts_show_each_task_s_usage_and_cost() -> TestResult {
    let (home, repo) = repository()?;
    let main = payloads::transcripts_folder(home.path()).join(format!("{S1}.jsonl"));
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    // The primer needs the graph: once it is built, the session's start is given its primer.
    wait_for(|| Ok(graph_of(&repo, home.path())?.is_some()))?;
    for name in ["session-start-startup.json", "user-prompt-submit.json"] {
        send(
            &payloads::as_session(CAPTURED, name, &repo, (S1, &main))?,
            home.path(),
        )?;
    }
    wait_for(|| Ok(tasks(&repo)?.len() == 1))?;
    let started_ms = tasks(&repo)?
        .first()
        .ok_or("one task")?
        .record
        .started_at_ms;
    let at = [
        started_ms - 60_000,
        started_ms,
        started_ms + 1,
        started_ms + 2,
    ];
    moved_transcript(
        "grep-session.jsonl",
        &main,
        &[
            ("2026-11-02T23:04:17.241Z", at[0]),
            ("2026-11-02T23:04:18.748Z", at[1]),
            ("2026-11-02T23:04:20.454Z", at[2]),
            ("2026-11-02T23:04:22.200Z", at[3]),
        ],
    )?;
    let subagent = payloads::transcripts_folder(home.path())
        .join(S1)
        .join("subagents")
        .join("agent-fixtureagent0001.jsonl");
    moved_transcript(
        "hooks-session/subagents/agent-fixtureagent0001.jsonl",
        &subagent,
        &[
            ("2026-11-01T19:01:33.153Z", at[2]),
            ("2026-11-01T19:01:35.180Z", at[3]),
        ],
    )?;
    for (name, transcript) in [
        ("post-tool-use-grep.json", &main),
        ("post-tool-use-grep-subagent.json", &main),
        ("subagent-stop.json", &subagent),
        ("stop.json", &main),
    ] {
        let payload = payloads::as_session(CAPTURED, name, &repo, (S1, transcript))?;
        send(&payload, home.path())?;
    }
    wait_for(|| {
        let is_ended = tasks(&repo)?.iter().all(|task| task.ended_at_ms.is_some());
        Ok(is_ended && turns(&repo, S1)? == 4 && turns(&repo, SUBAGENT)? == 2)
    })?;
    drop(daemon);

    let printed = receipts::run(&repo, home.path(), &["--json"])?;
    assert!(printed.status.success(), "{printed:?}");
    let report: Value = serde_json::from_slice(&printed.stdout)?;
    let price_file = home.path().join(".cortex").join("pricing.toml");
    assert_eq!(std::fs::read_to_string(&price_file)?, SHIPPED_PRICES);
    assert_eq!(
        report.get("prices"),
        Some(&json!({
            "state": "read",
            "file": price_file.display().to_string(),
            "updated": "2026-11-03",
            "source": "https://platform.claude.com/docs/en/about-claude/pricing",
        }))
    );
    assert_eq!(report.get("uncounted"), None);
    let recorded = report.get("recorded").ok_or("recorded")?;
    assert_eq!(
        recorded.get("holdout"),
        Some(&json!({"held_out": 0, "treated": 1, "needed_per_arm": 30, "comparison": null}))
    );
    assert_eq!(
        recorded.get("precision"),
        Some(&json!({"labelled": 0, "used": 0}))
    );
    assert_eq!(
        recorded.get("warnings"),
        Some(&json!({"labelled": 0, "used": 0}))
    );
    let sessions = recorded
        .get("sessions")
        .and_then(Value::as_array)
        .ok_or("sessions")?;
    assert_eq!(sessions.len(), 1);
    let session = sessions.first().ok_or("one session")?;
    let field = |value: &Value, name: &str| value.get(name).cloned().unwrap_or(Value::Null);
    assert_eq!(
        [
            "session_id",
            "agent",
            "is_metered",
            "status",
            "subagents",
            "tool_calls",
            "outside_tool_calls",
            "primer_tokens",
            "injected_tokens"
        ]
        .map(|name| field(session, name)),
        [
            json!(S1),
            json!("claude"),
            json!(true),
            json!("live"),
            json!(1),
            json!(2),
            json!(0),
            json!(PRIMER_TOKENS),
            json!(PRIMER_TOKENS)
        ]
    );
    let task = session
        .get("tasks")
        .and_then(|tasks| tasks.get(0))
        .ok_or("one task")?;
    assert_eq!(
        [
            "seq",
            "started_at_ms",
            "outcome",
            "held_out",
            "tool_calls",
            "injected_tokens"
        ]
        .map(|name| field(task, name)),
        [
            json!(1),
            json!(started_ms),
            json!("unknown"),
            json!(false),
            json!(2),
            json!(0)
        ]
    );
    let usage = receipts::tally;
    assert_eq!(
        field(task, "usage"),
        usage(5, [153_282, 2_724, 120_021, 30_537, 827], 109_168_100_000)
    );
    assert_eq!(
        field(session, "outside_tasks"),
        usage(1, [33_499, 3_702, 0, 29_797, 100], 127_592_000_000)
    );
    assert_eq!(
        field(session, "total"),
        usage(6, [186_781, 6_426, 120_021, 60_334, 927], 236_760_100_000)
    );

    let printed = receipts::run(&repo, home.path(), &[])?;
    assert!(printed.status.success(), "{printed:?}");
    let text = String::from_utf8(printed.stdout)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 9, "{text}");
    assert_eq!(
        lines[..4],
        [
            format!("prices: as of 2026-11-03, from {}", price_file.display()).as_str(),
            "holdout comparison: 0 of 30 held-out and 1 of 30 treated tasks so far",
            "injection precision: no work order given is labelled yet",
            "warning precision: no warning is marked yet",
        ]
    );
    let session_line = lines[4];
    assert!(
        session_line.starts_with(&format!("session {S1} (claude, live), started ")),
        "{session_line}"
    );
    assert!(session_line.ends_with(" UTC, 1 subagent"), "{session_line}");
    assert_eq!(
        lines[5],
        format!("  primer: {PRIMER_TOKENS} tokens injected by Cortex")
    );
    let task_line = lines[6];
    assert!(
        task_line.starts_with("  task 1 (unknown), started "),
        "{task_line}"
    );
    assert!(
        task_line.ends_with(
            " UTC: 5 turns, 153,282 input tokens (2,724 uncached, 120,021 read from the cache, \
             30,537 written to it), 827 output tokens, about $0.1092; 2 tool calls; 0 tokens \
             injected by Cortex"
        ),
        "{task_line}"
    );
    assert_eq!(
        lines[7..],
        [
            "  outside any task: 1 turn, 33,499 input tokens (3,702 uncached, 0 read from the \
             cache, 29,797 written to it), 100 output tokens, about $0.1276; 0 tool calls"
                .to_owned(),
            format!(
                "  total: 6 turns, 186,781 input tokens (6,426 uncached, 120,021 read from the \
                 cache, 60,334 written to it), 927 output tokens, about $0.2368; 2 tool calls; \
                 {PRIMER_TOKENS} tokens injected by Cortex"
            ),
        ]
    );

    let one = receipts::run(&repo, home.path(), &["--session", S1, "--json"])?;
    assert_eq!(serde_json::from_slice::<Value>(&one.stdout)?, report);
    let none = receipts::run(&repo, home.path(), &["--session", "nobody"])?;
    assert!(!none.status.success());
    let error = String::from_utf8(none.stderr)?;
    assert!(error.contains("no session nobody in the store"), "{error}");
    let sub = receipts::run(&repo, home.path(), &["--session", SUBAGENT])?;
    assert!(!sub.status.success());
    let error = String::from_utf8(sub.stderr)?;
    let counted_in = format!("session {SUBAGENT} is a subagent of session {S1}, whose receipts");
    assert!(error.contains(&counted_in), "{error}");
    Ok(())
}

/// A repository Cortex has not recorded anything in has no store, which receipts say rather than
/// show counts of nothing, and a session asked of it is an error; a broken price table means no
/// cost is estimated, saying why.
#[test]
fn a_repository_without_a_store_has_nothing_recorded() -> TestResult {
    let (home, repo) = repository()?;
    let folder = home.path().join(".cortex");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("pricing.toml"), "updated = 3\n")?;
    let printed = receipts::run(&repo, home.path(), &["--days", "2"])?;
    assert!(printed.status.success(), "{printed:?}");
    let text = String::from_utf8(printed.stdout)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(
        lines[0].starts_with(
            "prices: none, so no cost is estimated (the price file is not a price table: "
        ),
        "{}",
        lines[0]
    );
    assert!(lines[1].starts_with("nothing recorded: "), "{}", lines[1]);
    assert!(
        lines[1].ends_with(" has no store yet; `cortex daemon` builds one"),
        "{}",
        lines[1]
    );
    let printed = receipts::run(&repo, home.path(), &["--json"])?;
    let report: Value = serde_json::from_slice(&printed.stdout)?;
    assert_eq!(report.get("recorded"), Some(&Value::Null));
    let asked = receipts::run(&repo, home.path(), &["--session", S1])?;
    assert!(!asked.status.success());
    let error = String::from_utf8(asked.stderr)?;
    assert!(error.contains(&format!("no session {S1}: ")), "{error}");
    assert!(error.contains(" has no store yet"), "{error}");
    Ok(())
}

/// The holdout comparison through the binary, on a store written by hand: one session of Claude
/// Code with 60 tasks that all succeeded, the odd ones held out, each with one response of `input`
/// fresh input tokens, nothing cached, and no tool call. The 30 treated took 100, 200, ...,
/// 3,000: the median of 30 is the mean of the 15th and 16th, (1,500 + 1,600) / 2 = 1,550. The 30
/// held out took 4,000, 5,000, ..., 33,000: (18,000 + 19,000) / 2 = 18,500, a difference of
/// -16,950. Both arms succeed every time and take one turn and no tool call, so those differ by
/// nothing, interval and all.
#[test]
fn receipts_compare_the_arms_once_each_has_thirty_ended_tasks() -> TestResult {
    let (home, repo) = repository()?;
    let mut writer = Writer::open(&WriterOptions {
        folder: &repo.join(".cortex"),
        repo_id: &RepoId::of(&canonical_root(&repo)?),
        mmap_bytes: 0,
        now_ms: 1,
    })?;
    let session = Identifier::new("compared")?;
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: session.clone(),
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 1_000,
    })?;
    let mut responses = Vec::new();
    for at in 0..60_u32 {
        let task_id = Ulid::from_seed(u64::from(at), b"compared");
        let started_at_ms = 10_000 + i64::from(at) * 10_000;
        let is_held_out = at % 2 == 1;
        transaction.start_task(&TaskRecord {
            task_id,
            session_id: session.clone(),
            started_at_ms,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: Some(is_held_out),
        })?;
        transaction.end_task(task_id, started_at_ms + 5_000, TaskOutcome::Success)?;
        let rank = u64::from(at / 2 + 1);
        responses.push(UsageRow {
            session_id: session.clone(),
            turn: at + 1,
            at_ms: started_at_ms + 1_000,
            model: None,
            input: if is_held_out {
                3_000 + rank * 1_000
            } else {
                rank * 100
            },
            cache_write: 0,
            cache_write_1h: None,
            cache_read: 0,
            output: 1,
            reasoning: None,
            service_tier: None,
            speed: None,
            inference_geo: None,
            response: None,
        });
    }
    transaction.replace_usage(&responses)?;
    transaction.commit()?;
    drop(writer);
    let printed = receipts::run(&repo, home.path(), &[])?;
    assert!(printed.status.success(), "{printed:?}");
    let text = String::from_utf8(printed.stdout)?;
    let lines: Vec<&str> = text.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.starts_with("holdout comparison: "))
        .ok_or("no comparison")?;
    assert_eq!(
        lines.get(at..at + 6).ok_or("the comparison's lines")?[..2],
        [
            "holdout comparison: 30 tasks ended with Cortex and 30 held out; each difference is \
             with Cortex minus held out, its 95% bootstrap interval in brackets",
            "  success rate: 1.00 with Cortex, 1.00 held out, 0.00 (0.00 to 0.00)",
        ]
    );
    for (line, (what, treated, held_out, difference)) in lines[at + 2..at + 6].iter().zip([
        ("median input tokens", "1,550", "18,500", "-16,950"),
        ("median uncached input tokens", "1,550", "18,500", "-16,950"),
        ("median turns", "1", "1", "0 (0 to 0)"),
        ("median tool calls", "0", "0", "0 (0 to 0)"),
    ]) {
        let expected = format!(
            "  {what} per successful task: {treated} with Cortex, {held_out} held out, {difference}"
        );
        assert!(line.starts_with(&expected), "{line}");
    }
    assert_eq!(
        lines.get(at + 6).copied(),
        Some("  medians from 30 successful tasks with Cortex and 30 held out whose usage was read")
    );
    let report: Value =
        serde_json::from_slice(&receipts::run(&repo, home.path(), &["--json"])?.stdout)?;
    let input = report
        .pointer("/recorded/holdout/comparison/per_success/0")
        .ok_or("the input's comparison")?;
    let number = |name: &str| input.get(name).and_then(Value::as_f64).unwrap_or(f64::NAN);
    assert_eq!(
        (number("treated"), number("held_out"), number("difference")),
        (1_550.0, 18_500.0, -16_950.0)
    );
    // Every treated input is below every held-out one, so every resampled difference of medians
    // lies within 100 - 33,000 = -32,900 and 3,000 - 4,000 = -1,000, below zero.
    let (low, high) = (number("low"), number("high"));
    assert!(low >= -32_900.0 && high <= -1_000.0, "{input}");
    assert!(low <= -16_950.0 && -16_950.0 <= high, "{input}");
    Ok(())
}
