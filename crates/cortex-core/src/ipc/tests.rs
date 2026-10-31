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

/// An event request written as a frame reads back as the same request, and two frames written
/// one after the other read back in order.
#[test]
fn frames_read_back_in_order() -> TestResult {
    let event = Event {
        event_id: Ulid::from_seed(1_700_000_000_000, b"seed"),
        ts_ms: 1_700_000_000_000,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("session-1")?,
        parent_session_id: None,
        cwd: PathBuf::from("/repo"),
        repo_root: PathBuf::from("/repo"),
        hook: HookKind::Stop,
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
    };
    let first = Request {
        request_id: 1,
        body: RequestBody::Event {
            event: Box::new(event),
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
