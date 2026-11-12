//! The live socket end to end: a map served on a free port, opened as a browser opens a web socket,
//! its frames read off the connection by hand (a server's frames are not masked, so each is a byte
//! of kind, a length and the payload).

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cortex_core::ipc::{MapAnswer, ResponseBody};
use cortex_core::{Agent, Config, EventKind, EventMeta, Identifier, PositionMode, RepoPath};
use cortex_store::{EventRecord, PositionRecord, SessionRecord, Writer};

use super::Pacing;
use crate::inbox::Inbox;
use crate::map::{Map, MapSettings, Source};
use crate::test_store::{TestResult, add_file, writer};

/// The time the map reads the store at, in UTC milliseconds.
const NOW_MS: i64 = 10_000_000;

/// How long the test waits for a byte before it fails.
const WAIT: Duration = Duration::from_secs(10);

/// A fast pace with a refresh far beyond the test, so a frame after the first can only come from
/// a change noted.
const CHANGES_ONLY: Pacing = Pacing {
    gap: Duration::from_millis(50),
    poll: Duration::from_millis(20),
    refresh: Duration::from_secs(3_600),
};

/// A pace whose refresh comes within the test, for the ping, the note of use and the read again;
/// two seconds, so the test reads the first frame and looks at the inbox well before it.
const REFRESHING: Pacing = Pacing {
    gap: Duration::from_millis(50),
    poll: Duration::from_millis(20),
    refresh: Duration::from_secs(2),
};

/// The key a browser sends in its opening, from the protocol's own example.
const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

/// Stores session `id` of Claude Code with one event at `event_ms` and a read of `node` then.
fn store_read(
    writer: &mut Writer,
    id: &str,
    (node, event_ms): (cortex_store::NodeId, i64),
) -> TestResult {
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: Identifier::new(id)?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 0,
    })?;
    transaction.append_event(&EventRecord {
        task_id: None,
        session_id: Some(Identifier::new(id)?),
        ts_ms: event_ms,
        kind: EventKind::Read,
        node_id: None,
        path: None::<RepoPath>,
        range: None,
        meta: EventMeta::default(),
    })?;
    transaction.keep_position(&PositionRecord {
        session_id: Identifier::new(id)?,
        node,
        mode: PositionMode::Read,
        since_ms: event_ms,
        last_ms: event_ms,
        worktree: None,
        branch: None,
    })?;
    transaction.commit()?;
    Ok(())
}

/// The port and token of the map `map` serves.
fn address_of(map: &Map, runtime: &tokio::runtime::Runtime) -> TestResult<(u16, String)> {
    let ResponseBody::Map(MapAnswer::Serving { url, .. }) = runtime.block_on(map.answer()) else {
        return Err("the map was not served".into());
    };
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .ok_or("not on 127.0.0.1")?;
    let (port, token) = rest.split_once('/').ok_or("no path")?;
    Ok((port.parse()?, token.trim_end_matches('/').to_owned()))
}

/// Opens `path` on the map at `port` as a web socket, from the page's own `origin`, and returns
/// the connection with the status line of the answer.
fn open(port: u16, path: &str, origin: &str) -> TestResult<(TcpStream, String)> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    stream.set_read_timeout(Some(WAIT))?;
    let head = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\n\
         Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    while !answer.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        answer.extend_from_slice(&byte);
    }
    let text = String::from_utf8(answer)?;
    let status = text.lines().next().unwrap_or_default().to_owned();
    Ok((stream, status))
}

/// The next frame on `stream`: its kind (1 text, 9 ping) and its payload.
fn next_frame(stream: &mut TcpStream) -> TestResult<(u8, Vec<u8>)> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head)?;
    let kind = head[0] & 0x0f;
    let length = match head[1] & 0x7f {
        126 => {
            let mut extended = [0u8; 2];
            stream.read_exact(&mut extended)?;
            usize::from(u16::from_be_bytes(extended))
        }
        127 => return Err("a frame over 64 KiB".into()),
        short => usize::from(short),
    };
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload)?;
    Ok((kind, payload))
}

/// The status line of a plain GET of `path` from the map's own page at `port`.
fn plain_get(port: u16, path: &str) -> TestResult<String> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    stream.set_read_timeout(Some(WAIT))?;
    let head =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer)?;
    let text = String::from_utf8(answer)?;
    Ok(text.lines().next().unwrap_or_default().to_owned())
}

/// Sends a text message of `length` bytes on `stream` as a page does: masked, here with a key of
/// zeros, which leaves the payload as it is.
fn send_text(stream: &mut TcpStream, length: u16) -> TestResult {
    let [high, low] = length.to_be_bytes();
    // A final text frame; then the mask bit with 126, saying a two-byte length follows.
    let mut message = vec![0x81, 0x80 | 0x7e, high, low, 0, 0, 0, 0];
    message.extend(std::iter::repeat_n(b'a', usize::from(length)));
    stream.write_all(&message)?;
    Ok(())
}

/// Whether `stream` ended within [`WAIT`]: a close frame, the end of the connection or a reset,
/// before any text frame; pings in between are skipped.
fn has_ended(stream: &mut TcpStream) -> bool {
    let until = Instant::now() + WAIT;
    while Instant::now() < until {
        match next_frame(stream) {
            Ok((8, _)) | Err(_) => return true,
            Ok((1, _)) => return false,
            Ok(_) => {}
        }
    }
    false
}

/// The next text frame on `stream` within [`WAIT`], skipping pings, as a string.
fn next_text(stream: &mut TcpStream) -> TestResult<String> {
    // Pings keep coming while the socket is open, so the wait is bounded as a whole, not per frame.
    let until = Instant::now() + WAIT;
    while Instant::now() < until {
        let (kind, payload) = next_frame(stream)?;
        if kind == 1 {
            return Ok(String::from_utf8(payload)?);
        }
    }
    Err("no text frame came in time".into())
}

/// A frame of Claude Code session `s1` reading the files with ids `files`, newest first, each
/// with when it was read.
fn frame(files: &[(usize, i64)]) -> String {
    let files: Vec<String> = files
        .iter()
        .map(|(file, at)| format!(r#"{{"file":{file},"mode":"read","last_ms":{at}}}"#))
        .collect();
    format!(
        r#"{{"version":1,"sessions":[{{"id":"s1","agent":"claude","title":"Claude Code","files":[{}]}}]}}"#,
        files.join(",")
    )
}

/// A map served on a free port over the store at `root`, read at [`NOW_MS`] at `pacing`, told of
/// recorded batches by `inbox`; with the runtime serving it, its port and its token.
fn serve(
    root: &std::path::Path,
    pacing: Pacing,
    inbox: &Arc<Inbox>,
) -> TestResult<(Map, tokio::runtime::Runtime, u16, String)> {
    let mut settings = MapSettings::of(&Config::default(), root);
    settings.pacing = pacing;
    let source = Source::new(
        (root.join(".cortex"), 0),
        cortex_retrieve::GraphSettings::default(),
        (Arc::default(), Arc::default(), Arc::clone(inbox)),
        settings,
    )
    .with_clock(|| NOW_MS);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let map = Map::default();
    map.configure(0, source);
    let (port, token) = address_of(&map, &runtime)?;
    Ok((map, runtime, port, token))
}

/// Opened from the page, the socket answers 101 and sends the live state at once: session `s1`
/// read `a.rs` (id 0) 10 s ago. When the store records `s1` reading `b.rs` (id 1) 5 s ago and the
/// writer notes the batch, the next frame shows both, newest first; the refresh is an hour away,
/// so only the change can have sent it. A message from the page of 2,048 bytes, over the cap of
/// 1,024, ends the socket, and a second socket ends when the map stops. Opened from another site's
/// page the socket is refused with 403, without the token 404, and a plain GET of it is 400.
#[test]
fn the_socket_sends_the_live_state_and_its_changes() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut store = writer(root.path())?;
    let a = add_file(&mut store, "a.rs", 1)?;
    let b = add_file(&mut store, "b.rs", 1)?;
    store_read(&mut store, "s1", (a, NOW_MS - 10_000))?;
    let inbox = Arc::new(Inbox::default());
    let (map, _runtime, port, token) = serve(root.path(), CHANGES_ONLY, &inbox)?;
    let page = format!("http://127.0.0.1:{port}");
    let live = format!("/{token}/api/live");

    let (mut socket, status) = open(port, &live, &page)?;
    assert!(status.starts_with("HTTP/1.1 101"), "{status}");
    assert_eq!(next_text(&mut socket)?, frame(&[(0, NOW_MS - 10_000)]));
    store_read(&mut store, "s1", (b, NOW_MS - 5_000))?;
    inbox.note_recorded();
    assert_eq!(
        next_text(&mut socket)?,
        frame(&[(1, NOW_MS - 5_000), (0, NOW_MS - 10_000)])
    );

    send_text(&mut socket, 2_048)?;
    assert!(
        has_ended(&mut socket),
        "a message over the cap did not end the socket"
    );

    let (_, refused) = open(port, &live, "http://evil.example")?;
    assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");
    let (_, unknown) = open(port, "/api/live", &page)?;
    assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
    let plain = plain_get(port, &live)?;
    assert!(plain.starts_with("HTTP/1.1 400"), "{plain}");

    let (mut second, status) = open(port, &live, &page)?;
    assert!(status.starts_with("HTTP/1.1 101"), "{status}");
    next_text(&mut second)?;
    map.stop();
    assert!(has_ended(&mut second), "the socket outlived the map");
    Ok(())
}

/// Each refresh pings the page and notes the map in use, which puts off the daemon's idle stop,
/// and reads the state again: unchanged at the first refresh, so the first ping is followed by the
/// second and no text; then a read of `b.rs` is stored with no batch noted, and the new state comes
/// after a later refresh, which only the read again can have found.
#[test]
fn an_open_socket_keeps_the_daemon_in_use_and_reads_again() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut store = writer(root.path())?;
    let a = add_file(&mut store, "a.rs", 1)?;
    let b = add_file(&mut store, "b.rs", 1)?;
    store_read(&mut store, "s1", (a, NOW_MS - 10_000))?;
    let inbox = Arc::new(Inbox::default());
    let (map, _runtime, port, token) = serve(root.path(), REFRESHING, &inbox)?;
    let page = format!("http://127.0.0.1:{port}");
    let (mut socket, status) = open(port, &format!("/{token}/api/live"), &page)?;
    assert!(status.starts_with("HTTP/1.1 101"), "{status}");
    assert_eq!(next_text(&mut socket)?, frame(&[(0, NOW_MS - 10_000)]));
    assert_eq!(inbox.last_request(), None);
    let (kind, _) = next_frame(&mut socket)?;
    assert_eq!(kind, 9, "the frame after the first is not a ping");
    assert!(
        inbox.last_request().is_some(),
        "the map was not noted in use"
    );
    let (kind, _) = next_frame(&mut socket)?;
    assert_eq!(kind, 9, "an unchanged state was sent again");
    // Stored only now, after the second ping: the read of that refresh may or may not see it, the
    // next one does, and no batch is noted, so only a refresh's read can send it.
    store_read(&mut store, "s1", (b, NOW_MS - 5_000))?;
    assert_eq!(
        next_text(&mut socket)?,
        frame(&[(1, NOW_MS - 5_000), (0, NOW_MS - 10_000)])
    );
    map.stop();
    Ok(())
}
