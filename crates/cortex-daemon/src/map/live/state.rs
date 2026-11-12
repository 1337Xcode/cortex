//! What `api/live` sends: the agent sessions live in the repository, each with the files it read,
//! edited or claimed most lately, as file ids of `api/graph`.
//!
//! A session is shown while it is live by the rule coordination uses: last heard from within
//! `coordination.live_minutes`, where an end or a clearing at or after its last event counts as not
//! heard from since (an event after the end takes it up again), and while it holds a position
//! taken in that window, as a session that has read or edited nothing yet has nowhere to be shown.
//! Its files are those positions, newest first, at most [`MAX_FILES`]; a file the listing does not
//! hold (one deleted since) is left out. Sessions are sorted by id and the frame holds at most
//! [`MAX_SESSIONS`] of them, the most lately heard from, so a frame's size is bounded.

use std::collections::BTreeMap;

use cortex_coord::{Timing, is_live};
use cortex_core::Identifier;
use cortex_store::{PositionRow, Queries, SeenSession, StoreError};
use serde::Serialize;

use super::super::api::Listing;

/// The version of the frame's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// The most files a session's trail holds: enough to show where it went lately, few enough that
/// ten sessions' trails stay readable.
pub(super) const MAX_FILES: usize = 10;

/// The most sessions a frame holds. More than this many agents at once in one repository is far
/// past what the map can show apart; the cap keeps a frame's size bounded whatever the store holds.
pub(super) const MAX_SESSIONS: usize = 64;

/// One frame: every live session.
#[derive(Debug, Serialize, PartialEq)]
pub(super) struct Frame {
    /// The shape's version.
    version: u8,
    /// The live sessions, by id.
    sessions: Vec<LiveSession>,
}

/// A live session and where it worked lately.
#[derive(Debug, Serialize, PartialEq)]
struct LiveSession {
    /// Its id, as its agent gave it.
    id: String,
    /// Its agent's name, such as `claude`.
    agent: &'static str,
    /// Its agent's title, such as `Claude Code`.
    title: &'static str,
    /// Its files, newest first.
    files: Vec<LiveFile>,
}

/// A file a session read, edited or claimed.
#[derive(Debug, Serialize, PartialEq)]
struct LiveFile {
    /// The file's id in `api/graph`.
    file: usize,
    /// `read`, `edit` or `claim`.
    mode: &'static str,
    /// When the session last did it, in UTC milliseconds.
    last_ms: i64,
}

/// The live sessions `queries` hold at `now_ms`, a session being live as `timing` says, with
/// their files as `listing` numbers them.
///
/// # Errors
///
/// The store's errors.
pub(super) fn frame_of(
    queries: Queries<'_>,
    listing: &Listing,
    now_ms: i64,
    timing: &Timing,
) -> Result<Frame, StoreError> {
    let rows = queries.positions_since(now_ms.saturating_sub(timing.live_ms))?;
    let mut by_session: BTreeMap<Identifier, Vec<PositionRow>> = BTreeMap::new();
    for row in rows {
        by_session
            .entry(row.record.session_id.clone())
            .or_default()
            .push(row);
    }
    let ids: Vec<Identifier> = by_session.keys().cloned().collect();
    let mut live: Vec<(&SeenSession, i64)> = Vec::new();
    let seen = queries.sessions_seen(&ids)?;
    for session in &seen {
        let Some(heard_ms) = session.heard_from_ms() else {
            continue;
        };
        if is_live(heard_ms, now_ms, timing) {
            live.push((session, heard_ms));
        }
    }
    // The most lately heard from first, then by id, so the cap keeps the same sessions each time.
    live.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.session_id.cmp(&b.0.session_id))
    });
    live.truncate(MAX_SESSIONS);
    let mut sessions: Vec<LiveSession> = live
        .into_iter()
        .map(|(session, _)| LiveSession {
            id: session.session_id.as_str().to_owned(),
            agent: session.agent.name(),
            title: session.agent.title(),
            files: files_of(by_session.get(&session.session_id), listing),
        })
        .collect();
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Frame {
        version: VERSION,
        sessions,
    })
}

/// The listed files of `rows`, newest first and then by id, at most [`MAX_FILES`].
fn files_of(rows: Option<&Vec<PositionRow>>, listing: &Listing) -> Vec<LiveFile> {
    let mut files: Vec<LiveFile> = rows
        .into_iter()
        .flatten()
        .filter_map(|row| {
            listing.id_of_node(row.record.node).map(|file| LiveFile {
                file,
                mode: row.record.mode.name(),
                last_ms: row.record.last_ms,
            })
        })
        .collect();
    files.sort_by(|a, b| b.last_ms.cmp(&a.last_ms).then(a.file.cmp(&b.file)));
    files.truncate(MAX_FILES);
    files
}

#[cfg(test)]
mod tests {
    use cortex_core::{
        Agent, CoordinationConfig, EventKind, EventMeta, Identifier, PositionMode, RepoPath,
        SessionStatus,
    };
    use cortex_store::{EventRecord, NodeId, PositionRecord, Reader, SessionRecord, Writer};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// The time the frames are read at, in UTC milliseconds.
    const NOW_MS: i64 = 10_000_000;

    /// A session of `agent` started at 0.
    fn session(id: &str, agent: Agent) -> TestResult<SessionRecord> {
        Ok(SessionRecord {
            session_id: Identifier::new(id)?,
            agent,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 0,
        })
    }

    /// Stores session `id` of `agent`, its one event at `event_ms`, and its positions: each a node,
    /// a mode and when the session last took it.
    fn store_session(
        writer: &mut Writer,
        (id, agent, event_ms): (&str, Agent, i64),
        positions: &[(NodeId, PositionMode, i64)],
    ) -> TestResult {
        let mut transaction = writer.transaction()?;
        transaction.record_session(&session(id, agent)?)?;
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
        for &(node, mode, last_ms) in positions {
            transaction.keep_position(&PositionRecord {
                session_id: Identifier::new(id)?,
                node,
                mode,
                since_ms: last_ms,
                last_ms,
                worktree: None,
                branch: None,
            })?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// The frame of the store at `root` at [`NOW_MS`] under the default holds, as JSON.
    fn frame_json(root: &std::path::Path) -> TestResult<String> {
        let reader = Reader::open(&root.join(".cortex"), 0)?;
        let timing = Timing::of(&CoordinationConfig::default());
        let frame = reader.consistently(|queries| {
            let listing = Listing::read(queries)?;
            frame_of(queries, &listing, NOW_MS, &timing)
        })?;
        Ok(serde_json::to_string(&frame)?)
    }

    /// A session is live for 15 minutes (900,000 ms) after its last event unless it ended. Live:
    /// `s0` (Codex, heard 10 s ago, claimed `c.rs`, id 2) and `s1` (Claude Code, heard 1 minute
    /// ago, edited `b.rs` at 60 s ago and read `a.rs` at 5 minutes ago; its read of `c.rs` 16 2/3
    /// minutes ago is older than the window and left out). Not live: `s2`, whose end 20 s ago came
    /// after its last event 30 s ago, and `s3`, last heard 15 minutes and 50 ms ago, its position
    /// within the window. Files newest first, sessions by id.
    #[test]
    fn the_frame_holds_live_sessions_and_their_newest_files() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let a = add_file(&mut writer, "a.rs", 1)?;
        let b = add_file(&mut writer, "b.rs", 1)?;
        let c = add_file(&mut writer, "c.rs", 1)?;
        let (read, edit, claim) = (PositionMode::Read, PositionMode::Edit, PositionMode::Claim);
        store_session(
            &mut writer,
            ("s0", Agent::Codex, NOW_MS - 10_000),
            &[(c, claim, NOW_MS - 10_000)],
        )?;
        store_session(
            &mut writer,
            ("s1", Agent::Claude, NOW_MS - 60_000),
            &[
                (a, read, NOW_MS - 300_000),
                (b, edit, NOW_MS - 60_000),
                (c, read, NOW_MS - 1_000_000),
            ],
        )?;
        store_session(
            &mut writer,
            ("s2", Agent::Claude, NOW_MS - 30_000),
            &[(a, read, NOW_MS - 30_000)],
        )?;
        let mut transaction = writer.transaction()?;
        transaction.end_session(
            &Identifier::new("s2")?,
            (NOW_MS - 20_000, SessionStatus::Ended),
        )?;
        transaction.commit()?;
        store_session(
            &mut writer,
            ("s3", Agent::Claude, NOW_MS - 900_050),
            &[(b, read, NOW_MS - 800_000)],
        )?;
        drop(writer);
        let expected = concat!(
            r#"{"version":1,"sessions":["#,
            r#"{"id":"s0","agent":"codex","title":"Codex","files":[{"file":2,"mode":"claim","last_ms":9990000}]},"#,
            r#"{"id":"s1","agent":"claude","title":"Claude Code","files":[{"file":1,"mode":"edit","last_ms":9940000},{"file":0,"mode":"read","last_ms":9700000}]}"#,
            "]}"
        );
        assert_eq!(frame_json(root.path())?, expected);
        Ok(())
    }

    /// A session's trail holds its ten newest files: of twelve read one second apart, the ten read
    /// last, `f02.rs` (id 2) to `f11.rs` (id 11), newest first. With no live session the frame is
    /// empty.
    #[test]
    fn a_trail_holds_the_ten_newest_files() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let mut positions = Vec::new();
        for index in 0..12_i64 {
            let node = add_file(&mut writer, &format!("f{index:02}.rs"), 1)?;
            positions.push((node, PositionMode::Read, NOW_MS - 12_000 + index * 1_000));
        }
        drop(writer);
        assert_eq!(frame_json(root.path())?, r#"{"version":1,"sessions":[]}"#);
        let mut writer = crate::test_store::writer(root.path())?;
        store_session(
            &mut writer,
            ("s1", Agent::Claude, NOW_MS - 1_000),
            &positions,
        )?;
        drop(writer);
        let files: Vec<String> = (2..12_i64)
            .rev()
            .map(|id| {
                format!(
                    r#"{{"file":{id},"mode":"read","last_ms":{}}}"#,
                    NOW_MS - 12_000 + id * 1_000
                )
            })
            .collect();
        let expected = format!(
            r#"{{"version":1,"sessions":[{{"id":"s1","agent":"claude","title":"Claude Code","files":[{}]}}]}}"#,
            files.join(",")
        );
        assert_eq!(frame_json(root.path())?, expected);
        Ok(())
    }

    /// A frame holds the 64 sessions heard from most lately: of 65 live sessions `s00` to `s64`,
    /// heard 65 s to 1 s ago in that order, each holding a read of `a.rs` (id 0) then, `s00` is
    /// left out and the rest are sorted by id.
    #[test]
    fn a_frame_holds_the_64_sessions_heard_from_last() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let a = add_file(&mut writer, "a.rs", 1)?;
        let heard_ms = |index: i64| NOW_MS - (65 - index) * 1_000;
        for index in 0..65_i64 {
            let id = format!("s{index:02}");
            store_session(
                &mut writer,
                (&id, Agent::Claude, heard_ms(index)),
                &[(a, PositionMode::Read, heard_ms(index))],
            )?;
        }
        drop(writer);
        let sessions: Vec<String> = (1..65_i64)
            .map(|index| {
                format!(
                    r#"{{"id":"s{index:02}","agent":"claude","title":"Claude Code","files":[{{"file":0,"mode":"read","last_ms":{}}}]}}"#,
                    heard_ms(index)
                )
            })
            .collect();
        let expected = format!(r#"{{"version":1,"sessions":[{}]}}"#, sessions.join(","));
        assert_eq!(frame_json(root.path())?, expected);
        Ok(())
    }
}
