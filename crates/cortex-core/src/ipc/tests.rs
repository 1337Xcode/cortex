//! Frames and messages: the frozen shapes written by hand, round trips, and every way a frame can
//! be refused.

use std::io::ErrorKind;
use std::path::PathBuf;

use proptest::prelude::*;

use super::*;
use crate::{Agent, Event, HookKind, Identifier, Ulid};

/// Result type for these tests: any failure fails the test.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// `json` as a frame: its length in 4 bytes, little-endian, then the JSON.
fn frame_of(len_bytes: u32, json: &str) -> Vec<u8> {
    let mut frame = len_bytes.to_le_bytes().to_vec();
    frame.extend_from_slice(json.as_bytes());
    frame
}

/// The frozen messages keep the shapes written here by hand, prefixed with their lengths counted
/// by hand: a Shutdown is 43 bytes of JSON, a Hello 116, a Mismatch 101.
#[test]
fn frozen_messages_keep_their_shape() -> TestResult {
    let shutdown = Request {
        request_id: 2,
        body: RequestBody::Shutdown,
    };
    assert_eq!(
        encode(&shutdown)?,
        frame_of(43, r#"{"request_id":2,"body":{"type":"shutdown"}}"#)
    );
    let hello = Request {
        request_id: 1,
        body: RequestBody::Hello(Hello {
            protocol: Protocol { major: 1, minor: 0 },
            cortex_version: "2.0.0-dev".to_owned(),
            role: Role::Hook,
        }),
    };
    assert_eq!(
        encode(&hello)?,
        frame_of(
            116,
            concat!(
                r#"{"request_id":1,"body":{"type":"hello","protocol":{"major":1,"minor":0},"#,
                r#""cortex_version":"2.0.0-dev","role":"hook"}}"#
            )
        )
    );
    let mismatch = Response {
        request_id: 1,
        body: ResponseBody::Mismatch(Mismatch {
            protocol: Protocol { major: 2, minor: 0 },
            cortex_version: "3.0.0".to_owned(),
        }),
    };
    assert_eq!(
        encode(&mismatch)?,
        frame_of(
            101,
            concat!(
                r#"{"request_id":1,"body":{"type":"mismatch","protocol":{"major":2,"minor":0},"#,
                r#""cortex_version":"3.0.0"}}"#
            )
        )
    );
    Ok(())
}

/// An event of `hook` from session `session-1` in `/repo`, at 1,700,000,000,000 ms.
fn event_of(hook: HookKind) -> Result<Event, Box<dyn std::error::Error>> {
    Ok(Event {
        event_id: Ulid::from_seed(1_700_000_000_000, b"seed"),
        ts_ms: 1_700_000_000_000,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("session-1")?,
        parent_session_id: None,
        cwd: PathBuf::from("/repo"),
        repo_root: PathBuf::from("/repo"),
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: None,
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// An event request written as a frame reads back as the same request, and two frames written
/// one after the other read back in order.
#[test]
fn frames_read_back_in_order() -> TestResult {
    let first = Request {
        request_id: 1,
        body: RequestBody::Event {
            event: Box::new(event_of(HookKind::Stop)?),
        },
    };
    let second = Request {
        request_id: 2,
        body: RequestBody::Status,
    };
    let mut wire = Vec::new();
    write_frame(&mut wire, &first)?;
    write_frame(&mut wire, &second)?;
    let mut reader = wire.as_slice();
    assert_eq!(read_frame::<Request>(&mut reader)?, first);
    assert_eq!(read_frame::<Request>(&mut reader)?, second);
    assert_eq!(reader, [0u8; 0]);
    Ok(())
}

/// A frame is refused when its prefix is cut short, when it announces more than 1 MiB (before any
/// of the body is read), when the body ends early, and when the body is not the message expected;
/// a message whose JSON passes 1 MiB is refused when written.
#[test]
fn broken_frames_are_refused() {
    let short_prefix: &[u8] = &[5, 0];
    assert!(matches!(
        read_frame::<Request>(&mut &*short_prefix),
        Err(FrameError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof
    ));
    let oversized = frame_of(1_048_577, "");
    assert!(matches!(
        read_frame::<Request>(&mut oversized.as_slice()),
        Err(FrameError::TooLarge {
            len_bytes: 1_048_577
        })
    ));
    let cut = frame_of(43, r#"{"request_id":2"#);
    assert!(matches!(
        read_frame::<Request>(&mut cut.as_slice()),
        Err(FrameError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof
    ));
    let not_a_request = frame_of(2, "[]");
    assert!(matches!(
        read_frame::<Request>(&mut not_a_request.as_slice()),
        Err(FrameError::Json(_))
    ));
    let huge = ResponseBody::Error {
        message: "x".repeat(MAX_FRAME_BYTES),
    };
    assert!(matches!(encode(&huge), Err(FrameError::TooLarge { .. })));
}

/// A handshake from a client of another version still reads: a role this version does not know
/// reads as `other`, and fields it does not know are ignored, in the envelope, the Hello and the
/// protocol, so that such a client can be told of the mismatch.
#[test]
fn a_handshake_from_any_version_reads() -> TestResult {
    let json = concat!(
        r#"{"request_id":1,"trace":"x","body":{"type":"hello","#,
        r#""protocol":{"major":7,"minor":3,"patch":1},"cortex_version":"9.1.0","#,
        r#""role":"plugin","features":["a","b"]}}"#
    );
    let request: Request = serde_json::from_str(json)?;
    assert_eq!(
        request,
        Request {
            request_id: 1,
            body: RequestBody::Hello(Hello {
                protocol: Protocol { major: 7, minor: 3 },
                cortex_version: "9.1.0".to_owned(),
                role: Role::Other,
            }),
        }
    );
    Ok(())
}

/// A status answer reads across versions: one from an older daemon, without latencies, recent
/// errors, the miner's part, the graph's or the work tree's, reads with both empty and none of the
/// parts; one from a newer daemon, with fields this version does not know, a hook it does not know
/// and a miner's and a work tree's state it does not know, reads with that hook's name as it came
/// and each state as another.
#[test]
fn a_status_from_any_version_reads() -> TestResult {
    let older = concat!(
        r#"{"cortex_version":"1.0.0","protocol":{"major":1,"minor":0},"pid":7,"#,
        r#""uptime_ms":5}"#
    );
    let report: StatusReport = serde_json::from_str(older)?;
    assert_eq!((report.latencies, report.recent_errors), (vec![], vec![]));
    assert_eq!(report.git, None);
    assert_eq!(report.graph, None);
    assert_eq!(report.worktree, None);
    let newer = concat!(
        r#"{"cortex_version":"9.0.0","protocol":{"major":1,"minor":4},"pid":7,"#,
        r#""uptime_ms":5,"queues":{},"latencies":[{"hook":"permission_request","#,
        r#""requests":2,"p50_us":10,"p95_us":20,"p99_us":20,"p999_us":20}],"#,
        r#""recent_errors":[{"at_ms":3,"count":1,"message":"m","kind":"store"}],"#,
        r#""git":{"state":"rebasing","detail":"d","commits":3},"#,
        r#""graph":{"nodes":12,"links":30,"has_cochange":true,"builds":2,"build_ms":4,"#,
        r#""failed_builds":1,"layers":1},"#,
        r#""worktree":{"state":"stashed","uncommitted_files":1,"listed_files":4,"looks":2,"#,
        r#""failed_looks":0,"skipped_files":3}}"#
    );
    let report: StatusReport = serde_json::from_str(newer)?;
    assert_eq!(
        report.latencies,
        [HookLatency {
            hook: "permission_request".to_owned(),
            requests: 2,
            p50_us: 10,
            p95_us: 20,
            p99_us: 20,
        }]
    );
    assert_eq!(
        report.recent_errors,
        [RecentError {
            at_ms: 3,
            count: 1,
            message: "m".to_owned(),
        }]
    );
    assert_eq!(
        report.git,
        Some(GitReport {
            state: GitState::Other,
            detail: Some("d".to_owned()),
        })
    );
    assert_eq!(
        report.graph,
        Some(GraphReport {
            nodes: 12,
            links: 30,
            has_cochange: true,
            builds: 2,
            build_ms: 4,
            failed_builds: 1,
        })
    );
    assert_eq!(
        report.worktree,
        Some(WorktreeReport {
            state: WorktreeState::Other,
            uncommitted_files: 1,
            listed_files: 4,
            looks: 2,
            failed_looks: 0,
        })
    );
    Ok(())
}

/// A prefix may announce exactly 1 MiB, and no more.
#[test]
fn the_cap_is_one_mebibyte() {
    assert_eq!(
        body_len(1_048_576_u32.to_le_bytes()).ok(),
        Some(MAX_FRAME_BYTES)
    );
    assert!(body_len(1_048_577_u32.to_le_bytes()).is_err());
    assert!(body_len(u32::MAX.to_le_bytes()).is_err());
}

proptest! {
    /// Any bytes at all read as a request or as an error, never a panic.
    #[test]
    fn any_bytes_read_or_fail(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = read_frame::<Request>(&mut bytes.as_slice());
        let _ = decode::<Response>(&bytes);
    }

    /// Any body behind a well-formed prefix reads as a request or as an error, never a panic.
    #[test]
    fn any_body_reads_or_fails(body in prop::collection::vec(any::<u8>(), 0..512)) {
        let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
        let mut frame = len.to_le_bytes().to_vec();
        frame.extend_from_slice(&body);
        let _ = read_frame::<Request>(&mut frame.as_slice());
    }

    /// Bodies made of JSON's own characters and the words of the messages, which reach deeper
    /// into the parser than random bytes, read or fail, never panic.
    #[test]
    fn json_like_bodies_read_or_fail(
        parts in prop::collection::vec(
            prop::sample::select(vec![
                "{", "}", "[", "]", ":", ",", "\"", "1", "-1", "1e999", "null", "true",
                "\"type\"", "\"body\"", "\"request_id\"", "\"hello\"", "\"event\"", "\"status\"",
                "\"shutdown\"", "\"protocol\"", "\"major\"", "\"event_id\"", "\"ranges\"",
                "\"start_line\"", "\\u0000", "\\ud800",
            ]),
            0..80,
        )
    ) {
        let body = parts.concat();
        let _ = decode::<Request>(body.as_bytes());
        let _ = decode::<Response>(body.as_bytes());
    }
}

/// The requests that answer with text, and the answer, are written as typed here and read back as
/// they were.
#[test]
fn text_requests_and_replies_read_back() -> TestResult {
    let ask = Request {
        request_id: 2,
        body: RequestBody::Where {
            query: "where is create_order".to_owned(),
            limit: 3,
        },
    };
    let frame = encode(&ask)?;
    assert_eq!(
        frame.get(LENGTH_PREFIX_BYTES..),
        Some(
            r#"{"request_id":2,"body":{"type":"where","query":"where is create_order","limit":3}}"#
                .as_bytes()
        )
    );
    assert_eq!(decode::<Request>(&frame[LENGTH_PREFIX_BYTES..])?, ask);
    for text in [Some("[cortex] Work order".to_owned()), None] {
        let reply = Response {
            request_id: 2,
            body: ResponseBody::Reply {
                text,
                to_user: None,
            },
        };
        let frame = encode(&reply)?;
        assert_eq!(decode::<Response>(&frame[LENGTH_PREFIX_BYTES..])?, reply);
    }
    let silent = encode(&Response {
        request_id: 3,
        body: ResponseBody::Reply {
            text: None,
            to_user: None,
        },
    })?;
    assert_eq!(
        silent.get(LENGTH_PREFIX_BYTES..),
        Some(r#"{"request_id":3,"body":{"type":"reply","text":null}}"#.as_bytes())
    );
    Ok(())
}

/// A request for text that says until when its hook reads the answer carries the time as written
/// and reads back with it; one that does not, as a client of protocol 2.0 writes it, has no such
/// field written and reads back with none.
#[test]
fn a_request_for_text_says_until_when_it_is_read() -> TestResult {
    let timed = Request {
        request_id: 2,
        body: RequestBody::Retrieve {
            event: Box::new(event_of(HookKind::Prompt)?),
            answer_by_ms: Some(1_700_000_000_975),
        },
    };
    let frame = encode(&timed)?;
    let json = std::str::from_utf8(frame.get(LENGTH_PREFIX_BYTES..).ok_or("a body")?)?;
    assert!(
        json.ends_with(r#","answer_by_ms":1700000000975}}"#),
        "{json}"
    );
    assert_eq!(decode::<Request>(&frame[LENGTH_PREFIX_BYTES..])?, timed);
    let untimed = Request {
        request_id: 3,
        body: RequestBody::Primer {
            event: Box::new(event_of(HookKind::SessionStart)?),
            answer_by_ms: None,
        },
    };
    let frame = encode(&untimed)?;
    let json = std::str::from_utf8(frame.get(LENGTH_PREFIX_BYTES..).ok_or("a body")?)?;
    assert!(!json.contains("answer_by_ms"), "{json}");
    assert_eq!(decode::<Request>(&frame[LENGTH_PREFIX_BYTES..])?, untimed);
    Ok(())
}

/// An edit's request, as the hook writes it after an edit: its type, its event and until when the
/// hook reads the answer, read back as written.
#[test]
fn an_edit_s_request_reads_back_as_written() -> TestResult {
    let edit = Request {
        request_id: 2,
        body: RequestBody::PostEdit {
            event: Box::new(event_of(HookKind::PostTool)?),
            answer_by_ms: Some(1_700_000_000_080),
        },
    };
    let frame = encode(&edit)?;
    let json = std::str::from_utf8(frame.get(LENGTH_PREFIX_BYTES..).ok_or("a body")?)?;
    assert!(
        json.starts_with(r#"{"request_id":2,"body":{"type":"post_edit","event":{"#),
        "{json}"
    );
    assert!(
        json.ends_with(r#","answer_by_ms":1700000000080}}"#),
        "{json}"
    );
    assert_eq!(decode::<Request>(&frame[LENGTH_PREFIX_BYTES..])?, edit);
    Ok(())
}

/// A stop's request for its reason, as the hook writes it when the agent stops: its type, its event
/// and until when the hook reads the answer, read back as written.
#[test]
fn a_stop_s_request_reads_back_as_written() -> TestResult {
    let stop = Request {
        request_id: 3,
        body: RequestBody::StopReason {
            event: Box::new(event_of(HookKind::Stop)?),
            answer_by_ms: Some(1_700_000_000_060),
        },
    };
    let frame = encode(&stop)?;
    let json = std::str::from_utf8(frame.get(LENGTH_PREFIX_BYTES..).ok_or("a body")?)?;
    assert!(
        json.starts_with(r#"{"request_id":3,"body":{"type":"stop_reason","event":{"#),
        "{json}"
    );
    assert!(
        json.ends_with(r#","answer_by_ms":1700000000060}}"#),
        "{json}"
    );
    assert_eq!(decode::<Request>(&frame[LENGTH_PREFIX_BYTES..])?, stop);
    Ok(())
}

/// A read's request for its hint, as the hook writes it before a read: its type, its event and until
/// when the hook reads the answer, read back as written.
#[test]
fn a_read_s_request_reads_back_as_written() -> TestResult {
    let read = Request {
        request_id: 2,
        body: RequestBody::ReadHint {
            event: Box::new(event_of(HookKind::PreTool)?),
            answer_by_ms: Some(1_700_000_000_040),
        },
    };
    let frame = encode(&read)?;
    let json = std::str::from_utf8(frame.get(LENGTH_PREFIX_BYTES..).ok_or("a body")?)?;
    assert!(
        json.starts_with(r#"{"request_id":2,"body":{"type":"read_hint","event":{"#),
        "{json}"
    );
    assert!(
        json.ends_with(r#","answer_by_ms":1700000000040}}"#),
        "{json}"
    );
    assert_eq!(decode::<Request>(&frame[LENGTH_PREFIX_BYTES..])?, read);
    Ok(())
}

/// A note, the user's word on the facts and the daemon's answers are written in the shapes typed
/// here by hand and read back as they were.
#[test]
fn fact_requests_and_answers_read_back() -> TestResult {
    let note = Request {
        request_id: 4,
        body: RequestBody::Note(Note {
            text: "Money is kept in cents.".to_owned(),
            kind: crate::FactKind::Invariant,
            source: crate::FactSource::Agent,
            at: vec!["src/money.py:Money".to_owned()],
        }),
    };
    let written = |message: &Request| -> Result<String, Box<dyn std::error::Error>> {
        let frame = encode(message)?;
        assert_eq!(decode::<Request>(&frame[LENGTH_PREFIX_BYTES..])?, *message);
        Ok(String::from_utf8(frame[LENGTH_PREFIX_BYTES..].to_vec())?)
    };
    assert_eq!(
        written(&note)?,
        r#"{"request_id":4,"body":{"type":"note","text":"Money is kept in cents.","kind":"invariant","source":"agent","at":["src/money.py:Money"]}}"#
    );
    let edit = Request {
        request_id: 5,
        body: RequestBody::Facts(FactsAsk::Edit {
            id: 3,
            text: "Tax rounds half to even.".to_owned(),
        }),
    };
    assert_eq!(
        written(&edit)?,
        r#"{"request_id":5,"body":{"type":"facts","ask":"edit","id":3,"text":"Tax rounds half to even."}}"#
    );
    let list = Request {
        request_id: 6,
        body: RequestBody::Facts(FactsAsk::List { all: false }),
    };
    assert_eq!(
        written(&list)?,
        r#"{"request_id":6,"body":{"type":"facts","ask":"list","all":false}}"#
    );
    let answers = [
        FactsAnswer::Noted {
            id: 3,
            outcome: NoteOutcome::Activated,
        },
        FactsAnswer::Listed {
            facts: vec![FactLine {
                id: 3,
                status: crate::FactStatus::Suspect,
                kind: crate::FactKind::Pitfall,
                text: "Tax rounds half up.".to_owned(),
                anchors: vec!["src/tax.py".to_owned()],
                uses: 2,
            }],
        },
        FactsAnswer::Edited { id: 3, by: 4 },
    ];
    for answer in answers {
        let response = Response {
            request_id: 7,
            body: ResponseBody::Facts(answer),
        };
        let frame = encode(&response)?;
        assert_eq!(decode::<Response>(&frame[LENGTH_PREFIX_BYTES..])?, response);
    }
    let noted = encode(&Response {
        request_id: 7,
        body: ResponseBody::Facts(FactsAnswer::Noted {
            id: 3,
            outcome: NoteOutcome::Added,
        }),
    })?;
    assert_eq!(
        noted.get(LENGTH_PREFIX_BYTES..),
        Some(
            r#"{"request_id":7,"body":{"type":"facts","answer":"noted","id":3,"outcome":"added"}}"#
                .as_bytes()
        )
    );
    Ok(())
}
