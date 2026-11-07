//! Codex rollout files read as their responses: a real 0.166.0 rollout, scrubbed, read whole and in
//! pieces, and lines changed to drop a count, a model or an id.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use cortex_core::Agent;
use cortex_meter::{Codex, Tail, TranscriptFormat, Turn, Usage, format_of, utc_millis};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The rollout of a session of eleven model responses, captured with Codex 0.166.0 and scrubbed:
/// its ids fixed, its text replaced, its numbers and types kept.
fn rollout() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/codex/0.166.0/transcripts/session.jsonl")
}

/// Every turn `file` holds, read whole.
fn read_whole(file: &Path) -> TestResult<Vec<Turn>> {
    let mut tail = Tail::new();
    Ok(tail.read(&mut File::open(file)?, &Codex, u64::MAX)?)
}

/// A response's counts as the rollout's `usage` gives them: the whole input, the input read from
/// the cache, the output and the reasoning; no input was written to the cache in this session.
fn usage(whole: u64, cache_read: u64, output: u64, reasoning: u64) -> Usage {
    Usage {
        input: whole - cache_read,
        cache_write: 0,
        cache_write_1h: None,
        cache_read,
        output,
        reasoning: Some(reasoning),
    }
}

/// The eleven responses of the rollout, read off its `token_usage_record` lines by hand: their
/// times, and `input_tokens`, `cached_input_tokens`, `output_tokens` and
/// `reasoning_output_tokens`; the model is the `turn_context` line's, `gpt-6-luna`.
#[test]
fn every_response_of_a_rollout_is_read() -> TestResult {
    let turns = read_whole(&rollout())?;
    let expected = [
        ("2026-11-06T19:33:22.275Z", usage(18_126, 11_136, 104, 0)),
        ("2026-11-06T19:33:28.818Z", usage(18_324, 17_920, 76, 0)),
        ("2026-11-06T19:33:32.484Z", usage(18_468, 18_176, 70, 0)),
        ("2026-11-06T19:33:36.426Z", usage(18_689, 18_304, 127, 0)),
        ("2026-11-06T19:33:39.927Z", usage(18_840, 18_560, 70, 0)),
        ("2026-11-06T19:33:42.785Z", usage(18_952, 11_136, 64, 0)),
        ("2026-11-06T19:33:47.431Z", usage(19_040, 18_688, 80, 0)),
        ("2026-11-06T19:33:52.798Z", usage(19_150, 18_816, 168, 81)),
        ("2026-11-06T19:33:56.316Z", usage(19_348, 18_944, 115, 46)),
        ("2026-11-06T19:33:57.764Z", usage(19_485, 18_816, 22, 0)),
        ("2026-11-06T19:34:06.327Z", usage(19_594, 19_328, 67, 0)),
    ];
    assert_eq!(turns.len(), expected.len());
    for (number, (turn, (at, usage))) in (1..).zip(turns.iter().zip(expected)) {
        assert_eq!(turn.ordinal, number);
        assert_eq!(Some(turn.at_ms), utc_millis(at), "{number}");
        assert_eq!(turn.model, "gpt-6-luna");
        assert_eq!(turn.usage, usage, "{number}");
    }
    Ok(())
}

/// The input written to the cache is taken out of the whole input with the input read from it: a
/// whole input of 1,000 with 600 read and 100 written leaves 300 at the full price.
#[test]
fn cache_reads_and_writes_come_out_of_the_whole_input() -> TestResult {
    let lines = concat!(
        r#"{"timestamp":"2026-11-06T10:00:00.000Z","type":"turn_context","payload":{"model":"gpt-6"}}"#,
        "\n",
        r#"{"timestamp":"2026-11-06T10:00:01.000Z","type":"token_usage_record","payload":{"response_id":"resp_1","usage":{"input_tokens":1000,"cached_input_tokens":600,"cache_write_input_tokens":100,"output_tokens":50,"reasoning_output_tokens":20,"total_tokens":1050}}}"#,
        "\n",
    );
    let turns = read_text(lines)?;
    let [turn] = turns.as_slice() else {
        return Err(format!("{turns:?}").into());
    };
    assert_eq!(turn.model, "gpt-6");
    assert_eq!(
        turn.usage,
        Usage {
            input: 300,
            cache_write: 100,
            cache_write_1h: None,
            cache_read: 600,
            output: 50,
            reasoning: Some(20),
        }
    );
    Ok(())
}

/// A usage line with a count missing, with no id, before any line names the model, or of another
/// type that mentions the record's name, is no response; the model of the latest turn is taken.
#[test]
fn a_line_without_all_it_needs_is_no_response() -> TestResult {
    let record = |id: &str, counts: &str| {
        format!(
            r#"{{"timestamp":"2026-11-06T10:00:01.000Z","type":"token_usage_record","payload":{{"response_id":"{id}","usage":{{{counts}}}}}}}"#
        )
    };
    let counts = [
        r#""input_tokens":10"#,
        r#""cached_input_tokens":0"#,
        r#""cache_write_input_tokens":0"#,
        r#""output_tokens":1"#,
        r#""reasoning_output_tokens":0"#,
    ];
    let full = counts.join(",");
    let mut lines = vec![
        record("resp_before_model", &full),
        r#"{"timestamp":"2026-11-06T10:00:00.000Z","type":"turn_context","payload":{"model":"first"}}"#.to_owned(),
    ];
    // Each count missing in turn, the others there.
    for missing in 0..counts.len() {
        let some: Vec<&str> = (0..counts.len())
            .filter(|at| *at != missing)
            .filter_map(|at| counts.get(at).copied())
            .collect();
        lines.push(record(&format!("resp_missing_{missing}"), &some.join(",")));
    }
    lines.extend([
        record("", &full),
        r#"{"timestamp":"2026-11-06T10:00:02.000Z","type":"event_msg","payload":{"type":"token_usage_record"}}"#.to_owned(),
        r#"{"timestamp":"2026-11-06T10:00:03.000Z","type":"turn_context","payload":{"model":"second"}}"#.to_owned(),
        record("resp_counted", &full),
    ]);
    let text = lines.join("\n") + "\n";
    let turns = read_text(&text)?;
    let models: Vec<&str> = turns.iter().map(|turn| turn.model.as_str()).collect();
    assert_eq!(models, ["second"]);
    Ok(())
}

/// The turns `text`, written to a rollout file of its own, holds.
fn read_text(text: &str) -> TestResult<Vec<Turn>> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("rollout.jsonl");
    std::fs::write(&path, text)?;
    read_whole(&path)
}

/// A rollout read in passes as Codex writes it, a line at a time, gives the same responses with the
/// same numbers as read whole.
#[test]
fn a_rollout_read_as_it_grows_reads_the_same() -> TestResult {
    let whole = read_whole(&rollout())?;
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("rollout.jsonl");
    let mut writer = OpenOptions::new().create(true).append(true).open(&path)?;
    let mut tail = Tail::new();
    let mut grown = Vec::new();
    for line in std::fs::read_to_string(rollout())?.split_inclusive('\n') {
        writer.write_all(line.as_bytes())?;
        writer.flush()?;
        grown.extend(tail.read(&mut File::open(&path)?, &Codex, 4096)?);
    }
    assert_eq!(grown, whole);
    Ok(())
}

/// Codex keeps its rollouts under the home folder's `.codex/sessions`, and under `CODEX_HOME`'s
/// `sessions` too when that is an absolute path; a relative value moves nothing, and no other
/// variable counts. Its format is Codex's.
#[test]
fn codex_keeps_its_rollouts_in_its_sessions_folders() -> TestResult {
    let base = std::env::current_dir()?;
    let home = base.join("home");
    let moved = base.join("codex-home");
    let default = home.join(".codex").join("sessions");
    let set_to = |value: &Path| {
        let value = value.as_os_str().to_owned();
        move |name: &str| (name == "CODEX_HOME").then(|| value.clone())
    };
    assert_eq!(
        Codex.folders(&home, &|_| None),
        std::slice::from_ref(&default)
    );
    assert_eq!(
        Codex.folders(&home, &set_to(&moved)),
        [moved.join("sessions"), default.clone()]
    );
    assert_eq!(
        Codex.folders(&home, &set_to(Path::new("codex-home"))),
        std::slice::from_ref(&default)
    );
    let other = |name: &str| (name == "CLAUDE_CONFIG_DIR").then(|| moved.as_os_str().to_owned());
    assert_eq!(Codex.folders(&home, &other), [default]);
    assert!(format_of(Agent::Codex).is_some());
    Ok(())
}
