//! The meter through the real binary: captured Claude Code payloads name their session's
//! transcript, the daemon reads it at the end of the answer, at a subagent's stop (the subagent's
//! own transcript) and at the session's end, and writes the tokens each response took as usage
//! rows, numbered by the response's place in its transcript. A transcript outside Claude Code's
//! projects folders is not read; the configuration folder `CLAUDE_CONFIG_DIR` names holds one.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::hook_call::send;
use common::{Daemon, TestResult, daemon_command, isolate, payloads, wait_for};
use cortex_core::Identifier;
use cortex_store::{Reader, UsageRow};

/// The Claude Code version whose captured payloads and transcripts these tests use.
const CAPTURED: &str = "2.1.316";

/// The session of the captured payloads.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// Another session, the captured payloads sent as its.
const S2: &str = "00000000-0000-4000-8000-000000000002";

/// The model every captured response came from.
const MODEL: &str = "claude-sonnet-5-5";

/// A home folder holding a repository with one Python file, and the repository's root.
fn repository() -> TestResult<(tempfile::TempDir, PathBuf)> {
    let home = tempfile::tempdir()?;
    let repo = home.path().join("repo");
    std::fs::create_dir_all(&repo)?;
    std::fs::write(repo.join("a.py"), "a = 1\n")?;
    Ok((home, repo))
}

/// The usage rows of `session` the store of the repository at `root` holds, by turn.
fn usage(root: &Path, session: &str) -> TestResult<Vec<UsageRow>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    Ok(reader
        .queries()
        .usage_of_session(&Identifier::new(session)?)?)
}

/// What `cortex status --root <root>` printed, with `home` as the user's home folder.
fn status_of(root: &Path, home: &Path) -> TestResult<String> {
    let mut status = Command::new(env!("CARGO_BIN_EXE_cortex"));
    status.args(["status", "--root"]).arg(root);
    isolate(&mut status, home);
    Ok(String::from_utf8(status.output()?.stdout)?)
}

/// The row of turn `turn` of `session`, written at `at_ms`, with input, cache writes, the cache
/// writes made for an hour, cache reads and output `counts` and no reasoning, served as every
/// captured response was: at the standard tier and speed, run anywhere.
fn row(session: &str, (turn, at_ms): (u32, i64), counts: [u64; 5]) -> TestResult<UsageRow> {
    let [input, cache_write, cache_write_1h, cache_read, output] = counts;
    Ok(UsageRow {
        session_id: Identifier::new(session)?,
        turn,
        at_ms,
        model: Some(Identifier::new(MODEL)?),
        input,
        cache_write,
        cache_write_1h: Some(cache_write_1h),
        cache_read,
        output,
        reasoning: Some(0),
        service_tier: Some(Identifier::new("standard")?),
        speed: Some(Identifier::new("standard")?),
        inference_geo: Some(Identifier::new("global")?),
    })
}

/// A session through the binary, its usage read at each of its stops. The rows, read off the
/// captured transcripts by hand, each time worked out from the UTC timestamp of the response's
/// first line: 2026-11-02 is 20,759 days after 1970-01-01 (56 years with 14 leap days, 20,454
/// days, and 305 days into 2026), so 1,793,577,600 s at midnight, 2026-11-01 one day earlier.
///
/// - The Grep session's transcript, read at the end of the answer: four responses, at 23:04:17.241
///   (83,057 s into the day), 23:04:18.748, 23:04:20.454 and 23:04:22.200; the third written four
///   times with the same counts, counted once.
/// - The subagent's transcript, read at its stop and written as the subagent's session: two
///   responses, at 19:01:33.153 (68,493 s into the day) and 19:01:35.180.
/// - One more response written to the session's transcript after the answer, read at the
///   session's end as the fifth turn, the first four left as they were.
#[test]
fn a_session_s_usage_is_read_at_each_stop() -> TestResult {
    let (home, repo) = repository()?;
    let main = payloads::transcripts_folder(home.path()).join(format!("{S1}.jsonl"));
    payloads::copy_transcript(CAPTURED, "grep-session.jsonl", &main)?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    let session = (S1, main.as_path());
    for name in [
        "session-start-startup.json",
        "user-prompt-submit.json",
        "stop.json",
    ] {
        send(
            &payloads::as_session(CAPTURED, name, &repo, session)?,
            home.path(),
        )?;
    }
    wait_for(|| Ok(usage(&repo, S1)?.len() == 4))?;
    let day_02 = 1_793_577_600_000;
    let first_four = [
        row(
            S1,
            (1, day_02 + 83_057_241),
            [3_702, 29_797, 29_797, 0, 100],
        )?,
        row(S1, (2, day_02 + 83_058_748), [2, 3_819, 3_819, 29_797, 95])?,
        row(S1, (3, day_02 + 83_060_454), [2, 155, 155, 33_616, 360])?,
        row(S1, (4, day_02 + 83_062_200), [2, 763, 763, 33_771, 3])?,
    ];
    assert_eq!(usage(&repo, S1)?, first_four);

    let subagent = payloads::transcripts_folder(home.path())
        .join(S1)
        .join("subagents")
        .join("agent-fixtureagent0001.jsonl");
    payloads::copy_transcript(
        CAPTURED,
        "hooks-session/subagents/agent-fixtureagent0001.jsonl",
        &subagent,
    )?;
    let stopped = payloads::as_session(CAPTURED, "subagent-stop.json", &repo, (S1, &subagent))?;
    send(&stopped, home.path())?;
    let subagent_session = format!("{S1}/fixtureagent0001");
    wait_for(|| Ok(usage(&repo, &subagent_session)?.len() == 2))?;
    let day_01 = day_02 - 86_400_000;
    assert_eq!(
        usage(&repo, &subagent_session)?,
        [
            row(
                &subagent_session,
                (1, day_01 + 68_493_153),
                [2_716, 22_837, 0, 0, 148]
            )?,
            row(
                &subagent_session,
                (2, day_01 + 68_495_180),
                [2, 2_963, 0, 22_837, 221]
            )?,
        ]
    );

    // A fifth response: the fourth's line with its own message, request and time, and counts of
    // its own, its cache writes all to the one-hour cache as the fourth's were.
    let transcript = std::fs::read_to_string(&main)?;
    let fourth = transcript
        .lines()
        .find(|line| line.contains("\"msg_fixture0004\""))
        .ok_or("no fourth response")?;
    let fifth = fourth
        .replace("msg_fixture0004", "msg_fixture0005")
        .replace("req_fixture0004", "req_fixture0005")
        .replace("23:04:22.200Z", "23:04:30.000Z")
        .replace("\"input_tokens\":2,", "\"input_tokens\":4,")
        .replace(
            "\"cache_creation_input_tokens\":763,",
            "\"cache_creation_input_tokens\":12,",
        )
        .replace(
            "\"ephemeral_1h_input_tokens\":763,",
            "\"ephemeral_1h_input_tokens\":12,",
        )
        .replace(
            "\"cache_read_input_tokens\":33771,",
            "\"cache_read_input_tokens\":34534,",
        )
        .replace("\"output_tokens\":3,", "\"output_tokens\":9,");
    assert_ne!(fifth, fourth);
    std::fs::write(&main, format!("{transcript}{fifth}\n"))?;
    send(
        &payloads::as_session(CAPTURED, "session-end.json", &repo, session)?,
        home.path(),
    )?;
    wait_for(|| Ok(usage(&repo, S1)?.len() == 5))?;
    let mut all_five = first_four.to_vec();
    all_five.push(row(S1, (5, day_02 + 83_070_000), [4, 12, 12, 34_534, 9])?);
    assert_eq!(usage(&repo, S1)?, all_five);
    drop(daemon);
    Ok(())
}

/// A stop naming a transcript outside Claude Code's projects folder writes no usage, though the
/// file is a transcript the daemon reads in the folder, as a stop of another session naming it
/// there shows: the daemon's status says a transcript was not read and why, and the first session
/// has no row.
#[test]
fn a_transcript_outside_the_projects_folder_is_not_read() -> TestResult {
    let (home, repo) = repository()?;
    let outside = home.path().join("elsewhere").join(format!("{S1}.jsonl"));
    payloads::copy_transcript(CAPTURED, "grep-session.jsonl", &outside)?;
    let inside = payloads::transcripts_folder(home.path()).join(format!("{S2}.jsonl"));
    payloads::copy_transcript(CAPTURED, "grep-session.jsonl", &inside)?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    send(
        &payloads::as_session(CAPTURED, "stop.json", &repo, (S1, &outside))?,
        home.path(),
    )?;
    send(
        &payloads::as_session(CAPTURED, "stop.json", &repo, (S2, &inside))?,
        home.path(),
    )?;
    wait_for(|| Ok(usage(&repo, S2)?.len() == 4))?;
    let refused = "a transcript is not read: it is not in its agent's transcripts folder";
    wait_for(|| Ok(status_of(&repo, home.path())?.contains(refused)))?;
    assert_eq!(usage(&repo, S1)?, []);
    drop(daemon);
    Ok(())
}

/// A daemon started where `CLAUDE_CONFIG_DIR` moves Claude Code's configuration, as a hook run by
/// such a Claude Code starts it, reads a transcript in the moved folder's `projects`: the four
/// responses of the Grep session's transcript, as at the end of an answer in the default folder.
#[test]
fn a_moved_configuration_folder_holds_transcripts_the_daemon_reads() -> TestResult {
    let (home, repo) = repository()?;
    let config = home.path().join("claude-work");
    let transcript = config
        .join("projects")
        .join("C--repo")
        .join(format!("{S1}.jsonl"));
    payloads::copy_transcript(CAPTURED, "grep-session.jsonl", &transcript)?;
    let mut command = daemon_command(&repo, home.path());
    command.env("CLAUDE_CONFIG_DIR", &config);
    let daemon = Daemon::spawn(command)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    send(
        &payloads::as_session(CAPTURED, "stop.json", &repo, (S1, &transcript))?,
        home.path(),
    )?;
    wait_for(|| Ok(usage(&repo, S1)?.len() == 4))?;
    let outputs: Vec<u64> = usage(&repo, S1)?.iter().map(|row| row.output).collect();
    assert_eq!(outputs, [100, 95, 360, 3]);
    drop(daemon);
    Ok(())
}
