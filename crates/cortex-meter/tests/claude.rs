//! Claude Code transcripts read as their responses: three real 2.1.316 transcripts, reduced and
//! scrubbed, read whole, in pieces, cut in the middle of a line, written again, and with lines
//! changed to repeat, replay or fake a response.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use cortex_core::{Agent, Identifier};
use cortex_meter::{
    Claude, MAX_LINE_BYTES, MAX_RECENT_RESPONSES, Service, Tail, TranscriptFormat, Turn, Usage,
    format_of,
};
use proptest::prelude::*;

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// What a turn says, for comparing with values read off a fixture: its number, time, input,
/// cache writes (all, then the one-hour part), cache reads, output and reasoning.
type Said = (u32, i64, u64, u64, Option<u64>, u64, u64, Option<u64>);

/// The folder of the 2.1.316 transcripts.
fn transcripts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/claude/2.1.316/transcripts")
}

/// The bytes of the transcript `name`.
fn fixture(name: &str) -> TestResult<Vec<u8>> {
    Ok(std::fs::read(transcripts().join(name))?)
}

/// How every response of the fixtures was served: the standard tier and speed, run anywhere.
fn standard() -> Service {
    let name = |text| Identifier::new(text).ok();
    Service {
        tier: name("standard"),
        speed: name("standard"),
        inference_geo: name("global"),
    }
}

/// What each of `turns` says, its model and how it was served checked to be what every fixture
/// says.
fn said(turns: &[Turn]) -> Vec<Said> {
    turns
        .iter()
        .inspect(|turn| assert_eq!(turn.model, "claude-sonnet-5-5"))
        .inspect(|turn| assert_eq!(turn.service, standard()))
        .map(|turn| {
            let usage = turn.usage;
            (
                turn.ordinal,
                turn.at_ms,
                usage.input,
                usage.cache_write,
                usage.cache_write_1h,
                usage.cache_read,
                usage.output,
                usage.reasoning,
            )
        })
        .collect()
}

/// A transcript file in a temporary folder that a test writes as an agent would.
struct Transcript {
    /// The folder, deleted when the test ends.
    _dir: tempfile::TempDir,
    /// The file.
    path: PathBuf,
}

impl Transcript {
    /// An empty transcript.
    fn new() -> TestResult<Self> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("session.jsonl");
        File::create(&path)?;
        Ok(Self { _dir: dir, path })
    }

    /// Appends `bytes`.
    fn append(&self, bytes: &[u8]) -> TestResult {
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        file.write_all(bytes)?;
        Ok(())
    }

    /// Writes the file again with `bytes` alone.
    fn rewrite(&self, bytes: &[u8]) -> TestResult {
        Ok(std::fs::write(&self.path, bytes)?)
    }

    /// The turns `tail` reads from it now, at most `max_bytes` of it.
    fn read(&self, tail: &mut Tail, max_bytes: u64) -> TestResult<Vec<Turn>> {
        let mut file = File::open(&self.path)?;
        Ok(tail.read(&mut file, &Claude, max_bytes)?)
    }
}

/// What `turns` add up to, each response once at its last counts: input, cache writes, cache reads
/// and output.
fn summed(turns: &[Turn]) -> [u64; 4] {
    let last: BTreeMap<u32, &Turn> = turns.iter().map(|turn| (turn.ordinal, turn)).collect();
    last.values()
        .fold([0; 4], |[input, written, read, output], turn| {
            [
                input + turn.usage.input,
                written + turn.usage.cache_write,
                read + turn.usage.cache_read,
                output + turn.usage.output,
            ]
        })
}

/// Claude Code's own running totals in a `cost-state` line, every model's added up: input, cache
/// writes, cache reads and output. `None` for any other line.
fn cost_state(line: &str) -> Option<[u64; 4]> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "cost-state" {
        return None;
    }
    let keys = [
        "inputTokens",
        "cacheCreationInputTokens",
        "cacheReadInputTokens",
        "outputTokens",
    ];
    value
        .get("modelUsage")?
        .as_object()?
        .values()
        .try_fold([0; 4], |totals, model| {
            let mut next = totals;
            for (total, key) in next.iter_mut().zip(keys) {
                *total += model.get(key)?.as_u64()?;
            }
            Some(next)
        })
}

/// The turns of `bytes` read whole by a new tail.
fn read_whole(bytes: &[u8]) -> TestResult<Vec<Turn>> {
    let transcript = Transcript::new()?;
    transcript.append(bytes)?;
    transcript.read(&mut Tail::new(), MAX_LINE_BYTES)
}

/// The Grep capture's seven usage lines are four responses, one line per content block, read off
/// the fixture: line 22, line 29, lines 33, 35, 36 and 39 (four tool calls) and line 44. Every
/// cache write went to the one-hour cache, and no reasoning was spent. Times are the first line's
/// of each response, `2026-11-02T23:04:17.241Z` and so on as UTC milliseconds. Claude Code's own
/// running totals, in the file's `cost-state` line, agree with their sum.
#[test]
fn a_session_s_responses_are_read_once_each() -> TestResult {
    let bytes = fixture("grep-session.jsonl")?;
    let turns = read_whole(&bytes)?;
    assert_eq!(
        said(&turns),
        [
            (
                1,
                1_793_660_657_241,
                3_702,
                29_797,
                Some(29_797),
                0,
                100,
                Some(0)
            ),
            (
                2,
                1_793_660_658_748,
                2,
                3_819,
                Some(3_819),
                29_797,
                95,
                Some(0)
            ),
            (
                3,
                1_793_660_660_454,
                2,
                155,
                Some(155),
                33_616,
                360,
                Some(0)
            ),
            (4, 1_793_660_662_200, 2, 763, Some(763), 33_771, 3, Some(0)),
        ]
    );
    let costs = cost_totals(&bytes)?;
    let summed = turns
        .iter()
        .fold((0, 0, 0, 0), |(input, output, read, write), turn| {
            (
                input + turn.usage.input,
                output + turn.usage.output,
                read + turn.usage.cache_read,
                write + turn.usage.cache_write,
            )
        });
    assert_eq!(summed, costs);
    Ok(())
}

/// Claude Code's own totals for the model, from the last `cost-state` line of `bytes`: input,
/// output, cache reads and cache writes.
fn cost_totals(bytes: &[u8]) -> TestResult<(u64, u64, u64, u64)> {
    let mut totals = None;
    for line in std::str::from_utf8(bytes)?.lines() {
        let entry: serde_json::Value = serde_json::from_str(line)?;
        if entry.get("type").and_then(serde_json::Value::as_str) == Some("cost-state") {
            let usage = entry
                .pointer("/modelUsage/claude-sonnet-5-5")
                .ok_or("no usage for the model")?;
            let count = |key: &str| {
                usage
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .ok_or("a count")
            };
            totals = Some((
                count("inputTokens")?,
                count("outputTokens")?,
                count("cacheReadInputTokens")?,
                count("cacheCreationInputTokens")?,
            ));
        }
    }
    Ok(totals.ok_or("no cost-state line")?)
}

/// The hook capture's 23 usage lines are 11 responses across its prompts, read off the fixture;
/// the sixth spent 66 of its output reasoning. The user line that returns the Explore agent's
/// result carries that agent's last usage, which is the agent's own transcript's to count, not
/// this one's.
#[test]
fn a_tool_s_result_counts_no_response() -> TestResult {
    let turns = read_whole(&fixture("hooks-session.jsonl")?)?;
    let outputs: Vec<(u32, u64, u64, Option<u64>)> = turns
        .iter()
        .map(|turn| {
            (
                turn.ordinal,
                turn.usage.input,
                turn.usage.output,
                turn.usage.reasoning,
            )
        })
        .collect();
    assert_eq!(
        outputs,
        [
            (1, 3_702, 323, Some(0)),
            (2, 2, 332, Some(0)),
            (3, 2, 472, Some(0)),
            (4, 2, 523, Some(0)),
            (5, 4, 4, Some(0)),
            (6, 4, 197, Some(66)),
            (7, 4, 110, Some(0)),
            (8, 4, 65, Some(0)),
            (9, 2, 66, Some(0)),
            (10, 4_058, 119, Some(0)),
            (11, 2, 92, Some(0)),
        ]
    );
    Ok(())
}

/// A call a transcript does not record, as Claude Code's summary of the session at `/compact` is
/// not, is not counted; Claude Code's own totals, its `cost-state` lines, show the gap, worked out
/// from the hook capture's lines. Before `/compact`, the first totals, line 68, are the main
/// transcript's responses to that line (3,708 input, 36,319 written to the cache, 99,396 read from
/// it, 1,650 output) and the Explore agent's two, which its own transcript holds (2,718, 25,800,
/// 22,837, 369): 6,426, 62,119, 122,233 and 2,019. The last totals, line 168, hold one call more
/// than the main transcript's eleven responses (7,786, 44,955, 344,724, 2,303) and the agent's
/// two: the compaction's 2,217 input, 69 written, 37,225 read and 2,085 output, which no line
/// records, making 12,721, 70,824, 404,786 and 4,757.
#[test]
fn a_call_the_transcript_does_not_record_is_not_counted() -> TestResult {
    let main = String::from_utf8(fixture("hooks-session.jsonl")?)?;
    let agent = summed(&read_whole(&fixture(
        "hooks-session/subagents/agent-fixtureagent0001.jsonl",
    )?)?);
    assert_eq!(agent, [2_718, 25_800, 22_837, 369]);
    let lines: Vec<&str> = main.lines().collect();
    let states: Vec<(usize, [u64; 4])> = lines
        .iter()
        .enumerate()
        .filter_map(|(at, line)| Some((at + 1, cost_state(line)?)))
        .collect();
    let first = states.first().ok_or("no totals")?;
    let last = states.last().ok_or("no totals")?;
    assert_eq!(first.0, 68);
    let before: String = lines
        .iter()
        .take(68)
        .map(|line| format!("{line}\n"))
        .collect();
    let before = summed(&read_whole(before.as_bytes())?);
    assert_eq!(before, [3_708, 36_319, 99_396, 1_650]);
    assert_eq!(first.1, [6_426, 62_119, 122_233, 2_019]);
    assert_eq!(last.0, 168);
    let whole = summed(&read_whole(main.as_bytes())?);
    assert_eq!(whole, [7_786, 44_955, 344_724, 2_303]);
    assert_eq!(last.1, [12_721, 70_824, 404_786, 4_757]);
    let counted: Vec<u64> = whole
        .iter()
        .zip(agent)
        .map(|(main, agent)| main + agent)
        .collect();
    let compaction: Vec<u64> = last
        .1
        .iter()
        .zip(&counted)
        .map(|(all, counted)| all - counted)
        .collect();
    assert_eq!(compaction, [2_217, 69, 37_225, 2_085]);
    Ok(())
}

/// The Explore agent's own transcript, every line a sidechain's, is two responses of its own, read
/// off the fixture; it wrote its cache for five minutes, so none of its writes is the one-hour
/// cache's.
#[test]
fn a_subagent_s_transcript_counts_its_own_responses() -> TestResult {
    let turns = read_whole(&fixture(
        "hooks-session/subagents/agent-fixtureagent0001.jsonl",
    )?)?;
    assert_eq!(
        said(&turns),
        [
            (
                1,
                1_793_559_693_153,
                2_716,
                22_837,
                Some(0),
                0,
                148,
                Some(0)
            ),
            (
                2,
                1_793_559_695_180,
                2,
                2_963,
                Some(0),
                22_837,
                221,
                Some(0)
            ),
        ]
    );
    Ok(())
}

/// A read that ends in the middle of a line leaves that line for the next read: the Grep session
/// written up to half of its line 33, the third response's first line, gives the first two
/// responses and stops at that line's start; the rest written, the next read gives the third and
/// fourth, and the tail stands at the end of the file.
#[test]
fn a_line_is_read_once_whole() -> TestResult {
    let bytes = fixture("grep-session.jsonl")?;
    let line_starts = line_starts(&bytes);
    let third = *line_starts.get(32).ok_or("line 33")?;
    let cut = third + 40;
    let transcript = Transcript::new()?;
    let mut tail = Tail::new();
    transcript.append(bytes.get(..cut).ok_or("the cut")?)?;
    let first = transcript.read(&mut tail, MAX_LINE_BYTES)?;
    assert_eq!(ordinals(&first), [1, 2]);
    assert_eq!(tail.offset(), u64::try_from(third)?);
    transcript.append(bytes.get(cut..).ok_or("the rest")?)?;
    let rest = transcript.read(&mut tail, MAX_LINE_BYTES)?;
    assert_eq!(ordinals(&rest), [3, 4]);
    assert_eq!(tail.offset(), u64::try_from(bytes.len())?);
    assert!(transcript.read(&mut tail, MAX_LINE_BYTES)?.is_empty());
    Ok(())
}

/// Where each line of `bytes` starts.
fn line_starts(bytes: &[u8]) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(at, _)| at + 1),
        )
        .collect()
}

/// The numbers of `turns`.
fn ordinals(turns: &[Turn]) -> Vec<u32> {
    turns.iter().map(|turn| turn.ordinal).collect()
}

/// A transcript written again shorter than where the tail stood is read from its start, its
/// responses numbered as a new tail numbers them: the Grep session read whole, then written again
/// as its first 30 lines (two responses), reads as those two, numbered 1 and 2.
#[test]
fn a_transcript_written_again_is_read_from_its_start() -> TestResult {
    let bytes = fixture("grep-session.jsonl")?;
    let transcript = Transcript::new()?;
    let mut tail = Tail::new();
    transcript.append(&bytes)?;
    assert_eq!(
        ordinals(&transcript.read(&mut tail, MAX_LINE_BYTES)?),
        [1, 2, 3, 4]
    );
    let shorter = *line_starts(&bytes).get(30).ok_or("line 31")?;
    transcript.rewrite(bytes.get(..shorter).ok_or("the first lines")?)?;
    let again = transcript.read(&mut tail, MAX_LINE_BYTES)?;
    assert_eq!(
        said(&again),
        said(&read_whole(bytes.get(..shorter).ok_or("lines")?)?)
    );
    assert_eq!(ordinals(&again), [1, 2]);
    Ok(())
}

/// A copy of a response's line with more tokens comes again under the response's number, its
/// counts replacing the earlier ones; a copy with fewer or as many adds nothing. Here the
/// Grep session's third response, written on four lines, has its second line's output raised
/// from 360 to 400 and its third's lowered to 300.
#[test]
fn a_later_copy_with_more_tokens_replaces_the_counts() -> TestResult {
    let text = String::from_utf8(fixture("grep-session.jsonl")?)?;
    let mut copies = 0;
    let changed: Vec<String> = text
        .lines()
        .map(|line| {
            if !line.contains("\"msg_fixture0003\"") {
                return line.to_owned();
            }
            copies += 1;
            match copies {
                2 => line.replace("\"output_tokens\":360", "\"output_tokens\":400"),
                3 => line.replace("\"output_tokens\":360", "\"output_tokens\":300"),
                _ => line.to_owned(),
            }
        })
        .collect();
    assert_eq!(copies, 4);
    let turns = read_whole((changed.join("\n") + "\n").as_bytes())?;
    let outputs: Vec<(u32, u64)> = turns
        .iter()
        .map(|turn| (turn.ordinal, turn.usage.output))
        .collect();
    assert_eq!(outputs, [(1, 100), (2, 95), (3, 360), (3, 400), (4, 3)]);
    Ok(())
}

/// A side conversation's line that replays a recent response's message under another request is
/// that response again and counts nothing; a response of Claude Code's own synthetic model counts
/// nothing; the response after both is the second.
#[test]
fn a_replay_and_a_synthetic_response_count_nothing() -> TestResult {
    let line = |message: &str, request: &str, model: &str, is_sidechain: bool| {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"2026-11-02T23:04:17.241Z\",\"requestId\":\"{request}\",\"isSidechain\":{is_sidechain},\"message\":{{\"id\":\"{message}\",\"model\":\"{model}\",\"usage\":{{\"input_tokens\":1,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":1}}}}}}\n"
        )
    };
    let text = [
        line("msg_a", "req_a", "claude-sonnet-5-5", false),
        line("msg_a", "req_b", "claude-sonnet-5-5", true),
        line("msg_b", "req_c", "<synthetic>", false),
        line("msg_c", "req_d", "claude-sonnet-5-5", true),
    ]
    .concat();
    let turns = read_whole(text.as_bytes())?;
    assert_eq!(ordinals(&turns), [1, 2]);
    assert_eq!(
        turns.iter().map(|turn| turn.usage).collect::<Vec<Usage>>(),
        [Usage {
            input: 1,
            output: 1,
            ..Usage::default()
        }; 2]
    );
    Ok(())
}

/// A line missing one of the four counts every response reports is not understood: its response
/// is not counted, rather than counted with a zero nobody measured. Of the Grep session's first
/// response line with `output_tokens` taken out, and the second response whole, only the second
/// is read, as the first.
#[test]
fn a_line_missing_a_count_is_not_understood() -> TestResult {
    let bytes = fixture("grep-session.jsonl")?;
    let lines: Vec<&[u8]> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
    let first = String::from_utf8(lines.get(21).ok_or("line 22")?.to_vec())?;
    let second = lines.get(28).ok_or("line 29")?;
    let cut = first.replace("\"output_tokens\":100,", "");
    assert_ne!(cut, first);
    let turns = read_whole(&[cut.as_bytes(), second].concat())?;
    assert_eq!(
        turns
            .iter()
            .map(|turn| (turn.ordinal, turn.usage.output))
            .collect::<Vec<_>>(),
        [(1, 95)]
    );
    Ok(())
}

/// A tail remembers only its latest responses: after 64 others, a copy of the first is taken for
/// a new response, the 66th.
#[test]
fn a_tail_remembers_so_many_responses() -> TestResult {
    let line = |number: usize| {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"2026-11-02T23:04:17.241Z\",\"requestId\":\"req_{number}\",\"message\":{{\"id\":\"msg_{number}\",\"model\":\"claude-sonnet-5-5\",\"usage\":{{\"input_tokens\":1,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":1}}}}}}\n"
        )
    };
    let mut text: String = (0..=MAX_RECENT_RESPONSES).map(line).collect();
    text.push_str(&line(0));
    let turns = read_whole(text.as_bytes())?;
    assert_eq!(turns.len(), MAX_RECENT_RESPONSES + 2);
    assert_eq!(turns.last().map(|turn| turn.ordinal), Some(66));
    Ok(())
}

/// The first response of the Grep session's transcript, its line 22, with its line ending.
fn first_response() -> TestResult<Vec<u8>> {
    Ok(fixture("grep-session.jsonl")?
        .split_inclusive(|byte| *byte == b'\n')
        .nth(21)
        .ok_or("line 22")?
        .to_vec())
}

/// How a response was served is read from its line: the first response's line rewritten to the
/// priority tier, fast, run in the United States, gives those three; with its speed a text holding
/// a control character and its region left out, neither is known, and its tier still is.
#[test]
fn how_a_response_was_served_is_read() -> TestResult {
    let line = String::from_utf8(first_response()?)?;
    let served = line
        .replace(
            "\"service_tier\":\"standard\"",
            "\"service_tier\":\"priority\"",
        )
        .replace("\"speed\":\"standard\"", "\"speed\":\"fast\"")
        .replace("\"inference_geo\":\"global\"", "\"inference_geo\":\"us\"");
    let name = |text| Identifier::new(text).ok();
    let services = |text: &str| -> TestResult<Vec<Service>> {
        Ok(read_whole(text.as_bytes())?
            .into_iter()
            .map(|turn| turn.service)
            .collect())
    };
    assert_eq!(
        services(&served)?,
        [Service {
            tier: name("priority"),
            speed: name("fast"),
            inference_geo: name("us"),
        }]
    );
    let unclear = served
        .replace("\"speed\":\"fast\"", "\"speed\":\"fa\\u0007st\"")
        .replace("\"inference_geo\":\"us\",", "");
    assert_ne!(unclear, served);
    assert_eq!(
        services(&unclear)?,
        [Service {
            tier: name("priority"),
            speed: None,
            inference_geo: None,
        }]
    );
    // Values of another type than text are not known either, and the response is still read.
    let typed = served
        .replace("\"speed\":\"fast\"", "\"speed\":1")
        .replace(
            "\"inference_geo\":\"us\"",
            "\"inference_geo\":{\"region\":\"us\"}",
        )
        .replace("\"service_tier\":\"priority\"", "\"service_tier\":[]");
    assert_ne!(typed, served);
    let turns = read_whole(typed.as_bytes())?;
    assert_eq!(
        turns
            .iter()
            .map(|turn| (turn.usage.input, turn.service.clone()))
            .collect::<Vec<_>>(),
        [(3_702, Service::default())]
    );
    Ok(())
}

/// How cache writes split is known only when the line gives both lifetimes: the first response's
/// line, which wrote 29,797 for an hour and none for five minutes, with its five-minute count
/// left out says nothing of the split, and its writes are the flat 29,797.
#[test]
fn a_split_with_one_lifetime_missing_is_not_known() -> TestResult {
    let line = String::from_utf8(first_response()?)?;
    let one_missing = line.replace("\"ephemeral_5m_input_tokens\":0", "\"other\":0");
    assert_ne!(one_missing, line);
    let turns = read_whole(one_missing.as_bytes())?;
    assert_eq!(
        turns
            .iter()
            .map(|turn| (turn.usage.cache_write, turn.usage.cache_write_1h))
            .collect::<Vec<_>>(),
        [(29_797, None)]
    );
    Ok(())
}

/// A read stops after the whole lines that fit its allowance, but reads a line longer than the
/// allowance to its end: with reads of 64 bytes, a 100-byte line of letters is read whole, 101
/// bytes with its ending, and the response after it, longer than 64 bytes too, is read whole by
/// the next read.
#[test]
fn a_line_longer_than_a_read_is_read_whole() -> TestResult {
    let response = first_response()?;
    let transcript = Transcript::new()?;
    transcript.append(&[b'x'; 100])?;
    transcript.append(b"\n")?;
    transcript.append(&response)?;
    let mut tail = Tail::new();
    assert!(transcript.read(&mut tail, 64)?.is_empty());
    assert_eq!(tail.offset(), 101);
    assert_eq!(ordinals(&transcript.read(&mut tail, 64)?), [1]);
    assert_eq!(tail.offset(), 101 + u64::try_from(response.len())?);
    Ok(())
}

/// A line longer than `MAX_LINE_BYTES` is passed over: the first read stops after exactly that
/// many bytes of it with no response, and the next reads its last 10 bytes and ending as a broken
/// line, then the response after it.
#[test]
fn a_line_longer_than_the_longest_is_passed_over() -> TestResult {
    let response = first_response()?;
    let transcript = Transcript::new()?;
    let longest = usize::try_from(MAX_LINE_BYTES)?;
    transcript.append(&vec![b'x'; longest + 10])?;
    transcript.append(b"\n")?;
    transcript.append(&response)?;
    let mut tail = Tail::new();
    assert!(transcript.read(&mut tail, 4_096)?.is_empty());
    assert_eq!(tail.offset(), MAX_LINE_BYTES);
    assert_eq!(ordinals(&transcript.read(&mut tail, 4_096)?), [1]);
    Ok(())
}

/// Claude Code's transcripts are read, and no agent's but Claude Code's and Codex's.
#[test]
fn claude_code_has_a_format_and_other_agents_none() {
    assert!(format_of(Agent::Claude).is_some());
    assert!(format_of(Agent::Gemini).is_none());
}

proptest! {
    // Each case writes and reads the hook capture three times; 64 cases cover cuts in every part
    // of it in a few seconds.
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// However a transcript arrives, in up to three pieces cut anywhere, the turns read after
    /// each piece are those of one read of the whole, in the same order.
    #[test]
    fn pieces_read_as_the_whole(first in 0.0..1.0_f64, second in 0.0..1.0_f64) {
        let bytes = fixture("hooks-session.jsonl").map_err(|error| TestCaseError::fail(error.to_string()))?;
        let whole = read_whole(&bytes).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut cuts = [first, second].map(|share| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss, reason = "a share of a length, in range")]
            let at = (share * bytes.len() as f64) as usize;
            at
        });
        cuts.sort_unstable();
        let transcript = Transcript::new().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut tail = Tail::new();
        let mut read = Vec::new();
        let mut from = 0;
        for to in cuts.into_iter().chain([bytes.len()]) {
            let piece = bytes.get(from..to).ok_or_else(|| TestCaseError::fail("a piece"))?;
            transcript.append(piece).map_err(|error| TestCaseError::fail(error.to_string()))?;
            read.extend(transcript.read(&mut tail, MAX_LINE_BYTES).map_err(|error| TestCaseError::fail(error.to_string()))?);
            from = to;
        }
        prop_assert_eq!(read, whole);
    }
}

/// Claude Code keeps every transcript under `.claude/projects` in the user's home folder, one
/// folder a project, the subagents' in a folder beside their session's transcript; with
/// `CLAUDE_CONFIG_DIR` set to an absolute path, in its `projects` folder too. An empty or relative
/// value moves nothing, and no other variable counts.
#[test]
fn claude_code_keeps_its_transcripts_in_its_projects_folders()
-> Result<(), Box<dyn std::error::Error>> {
    let base = std::env::current_dir()?;
    let home = base.join("home");
    let config = base.join("claude-work");
    let default = home.join(".claude").join("projects");
    let set_to = |value: &Path| {
        let value = value.as_os_str().to_owned();
        move |name: &str| (name == "CLAUDE_CONFIG_DIR").then(|| value.clone())
    };
    assert_eq!(
        Claude.folders(&home, &|_| None),
        std::slice::from_ref(&default)
    );
    assert_eq!(
        Claude.folders(&home, &set_to(&config)),
        [config.join("projects"), default.clone()]
    );
    assert_eq!(
        Claude.folders(&home, &set_to(Path::new(""))),
        std::slice::from_ref(&default)
    );
    assert_eq!(
        Claude.folders(&home, &set_to(Path::new("claude-work"))),
        std::slice::from_ref(&default)
    );
    let other = |name: &str| (name == "CODEX_HOME").then(|| config.as_os_str().to_owned());
    assert_eq!(Claude.folders(&home, &other), [default]);
    Ok(())
}
