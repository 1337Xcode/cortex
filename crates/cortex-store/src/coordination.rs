//! Coordination across sessions as the store keeps it: each session's position on a node (the
//! one that holds longest, which the caller decides), what the store knows of the sessions holding
//! them, how strongly an edited file is linked to the files they hold, and the notices sessions
//! were sent or given. A notice's text names agents, files, symbols and links, never code, prompt
//! text or a command.

use cortex_core::{Agent, EdgeType, Identifier, PositionMode, SessionStatus};
use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::ids::{NodeId, NoticeId};
use crate::queries::Queries;
use crate::{schema, values};

/// A session's position on a node, as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionRecord {
    /// The session.
    pub session_id: Identifier,
    /// The node.
    pub node: NodeId,
    /// What the session did to it.
    pub mode: PositionMode,
    /// When the session first took this position, in UTC milliseconds.
    pub since_ms: i64,
    /// When it last did, in UTC milliseconds.
    pub last_ms: i64,
}

/// A position read back with when the task the session had at its last time ended, if it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionRow {
    /// The position.
    pub record: PositionRecord,
    /// When the session's task at the position's last time ended.
    pub task_ended_ms: Option<i64>,
}

/// What the store knows of a session that holds a position: who it is and how recently it was
/// heard from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenSession {
    /// The session.
    pub session_id: Identifier,
    /// Its agent.
    pub agent: Agent,
    /// The session that started it, for a subagent's.
    pub parent_session_id: Option<Identifier>,
    /// When it ended, if it has.
    pub ended_ms: Option<i64>,
    /// When its last event was, if it had one.
    pub last_event_ms: Option<i64>,
    /// Whether it is live, ended, or cleared by the user.
    pub status: SessionStatus,
}

/// The heaviest edge of one type between an edited file and a file a session holds, counting the
/// files' symbols: a structural edge's weight is how surely its reference was tied, a co-change
/// edge's is how often the held file changed when the edited one did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coupling {
    /// The held file's node.
    pub held: NodeId,
    /// The edge's type: structural or co-change.
    pub edge_type: EdgeType,
    /// Its weight.
    pub weight: f64,
}

/// A notice for a session: what it says and when it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeRecord {
    /// The session it is for.
    pub to_session: Identifier,
    /// What it says.
    pub text: String,
    /// When it was made, in UTC milliseconds.
    pub at_ms: i64,
}

/// A notice waiting for a session's next hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeRow {
    /// Its row.
    pub id: NoticeId,
    /// What it says.
    pub text: String,
    /// When it was queued, in UTC milliseconds.
    pub created_ms: i64,
}

/// Keeps `record` as its session's position on its node, in place of the one kept before.
pub(crate) fn keep_position(
    connection: &Connection,
    record: &PositionRecord,
) -> Result<(), StoreError> {
    connection
        .prepare_cached(schema::POSITION_KEEP)
        .and_then(|mut statement| {
            statement.execute((
                record.session_id.as_str(),
                record.node.get(),
                record.mode.name(),
                record.since_ms,
                record.last_ms,
            ))
        })
        .map_err(sqlite("keeping a position"))?;
    Ok(())
}

/// Queues `notice`, unless its session was sent or given the same text at `unless_since_ms` or
/// later: its row, or none when it was.
pub(crate) fn queue_notice(
    connection: &Connection,
    notice: &NoticeRecord,
    unless_since_ms: i64,
) -> Result<Option<NoticeId>, StoreError> {
    let added = connection
        .prepare_cached(schema::NOTICE_QUEUE)
        .and_then(|mut statement| {
            statement.execute((
                notice.to_session.as_str(),
                notice.text.as_str(),
                notice.at_ms,
                unless_since_ms,
            ))
        })
        .map_err(sqlite("queueing a notice"))?;
    Ok((added == 1).then(|| NoticeId(connection.last_insert_rowid())))
}

/// Records `notice` as told to its session as it was made, delivered at once.
pub(crate) fn record_told(
    connection: &Connection,
    notice: &NoticeRecord,
) -> Result<(), StoreError> {
    connection
        .prepare_cached(schema::NOTICE_TOLD)
        .and_then(|mut statement| {
            statement.execute((
                notice.to_session.as_str(),
                notice.text.as_str(),
                notice.at_ms,
            ))
        })
        .map_err(sqlite("recording a notice told"))?;
    Ok(())
}

/// Marks notice `id` delivered at `at_ms`. Whether it was waiting.
pub(crate) fn deliver_notice(
    connection: &Connection,
    id: NoticeId,
    at_ms: i64,
) -> Result<bool, StoreError> {
    let changed = connection
        .prepare_cached(schema::NOTICE_DELIVER)
        .and_then(|mut statement| statement.execute((id.get(), at_ms)))
        .map_err(sqlite("delivering a notice"))?;
    Ok(changed == 1)
}

/// `values` as a JSON array, for a statement that reads a list through `json_each`.
fn json_list<T: serde::Serialize>(
    values: &[T],
    column: &'static str,
) -> Result<String, StoreError> {
    serde_json::to_string(values).map_err(|_| StoreError::Corrupt { column, row: 0 })
}

impl Queries<'_> {
    /// Session `session_id`'s position on `node`, if one is kept, with when its task ended.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn position_of(
        self,
        session_id: &Identifier,
        node: NodeId,
    ) -> Result<Option<PositionRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::POSITION_OF)
            .map_err(sqlite("reading a position"))?;
        let mut rows = statement
            .query((session_id.as_str(), node.get()))
            .map_err(sqlite("reading a position"))?;
        let Some(row) = rows.next().map_err(sqlite("reading a position"))? else {
            return Ok(None);
        };
        let mode: String = row.get(0).map_err(sqlite("reading a position"))?;
        Ok(Some(PositionRow {
            record: PositionRecord {
                session_id: session_id.clone(),
                node,
                mode: values::kind(&mode, PositionMode::from_name, "positions.mode", node.get())?,
                since_ms: row.get(1).map_err(sqlite("reading a position"))?,
                last_ms: row.get(2).map_err(sqlite("reading a position"))?,
            },
            task_ended_ms: row.get(3).map_err(sqlite("reading a position"))?,
        }))
    }

    /// The positions last taken at `since_ms` or later, by session and node.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn positions_since(self, since_ms: i64) -> Result<Vec<PositionRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::POSITIONS_SINCE)
            .map_err(sqlite("reading positions"))?;
        let rows = statement
            .query_map([since_ms], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(sqlite("reading positions"))?;
        let mut positions = Vec::new();
        for row in rows {
            let (session, node, mode, since_ms, last_ms, task_ended_ms) =
                row.map_err(sqlite("reading positions"))?;
            positions.push(PositionRow {
                record: PositionRecord {
                    session_id: values::identifier(&session, "positions.session_id", node)?,
                    node: NodeId(node),
                    mode: values::kind(&mode, PositionMode::from_name, "positions.mode", node)?,
                    since_ms,
                    last_ms,
                },
                task_ended_ms,
            });
        }
        Ok(positions)
    }

    /// What the store knows of each of `sessions` it holds, by session.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn sessions_seen(self, sessions: &[Identifier]) -> Result<Vec<SeenSession>, StoreError> {
        let names: Vec<&str> = sessions.iter().map(Identifier::as_str).collect();
        let list = json_list(&names, "sessions.session_id")?;
        let mut statement = self
            .connection()
            .prepare_cached(schema::SESSIONS_SEEN)
            .map_err(sqlite("reading sessions"))?;
        let rows = statement
            .query_map([list], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(sqlite("reading sessions"))?;
        let mut seen = Vec::new();
        for (row, at) in rows.zip(0_i64..) {
            let (session, agent, parent, ended_ms, last_event_ms, status) =
                row.map_err(sqlite("reading sessions"))?;
            let parent_session_id = parent
                .map(|parent| values::identifier(&parent, "sessions.parent_session_id", at))
                .transpose()?;
            seen.push(SeenSession {
                session_id: values::identifier(&session, "sessions.session_id", at)?,
                agent: values::kind(&agent, Agent::from_name, "sessions.agent", at)?,
                parent_session_id,
                ended_ms,
                last_event_ms,
                status: values::kind(&status, SessionStatus::from_name, "sessions.status", at)?,
            });
        }
        Ok(seen)
    }

    /// How the file whose node is `edited` is linked to each file of `held`, counting both files'
    /// symbols: for each held file, its heaviest structural edge either way and its heaviest
    /// co-change edge from the edited file, by held file and then type. A held file with neither
    /// is left out.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn couplings(self, edited: NodeId, held: &[NodeId]) -> Result<Vec<Coupling>, StoreError> {
        let ids: Vec<i64> = held.iter().map(|node| node.get()).collect();
        let list = json_list(&ids, "nodes.node_id")?;
        let mut statement = self
            .connection()
            .prepare_cached(schema::COUPLINGS)
            .map_err(sqlite("reading couplings"))?;
        let rows = statement
            .query_map(
                (
                    edited.get(),
                    list,
                    EdgeType::Structural.number(),
                    EdgeType::CoChange.number(),
                ),
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, u8>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                },
            )
            .map_err(sqlite("reading couplings"))?;
        let mut couplings = Vec::new();
        for row in rows {
            let (held, number, weight) = row.map_err(sqlite("reading couplings"))?;
            let edge_type = EdgeType::from_number(number).ok_or(StoreError::Corrupt {
                column: "edges.type",
                row: held,
            })?;
            couplings.push(Coupling {
                held: NodeId(held),
                edge_type,
                weight,
            });
        }
        Ok(couplings)
    }

    /// The texts session `session_id` was sent or given at `since_ms` or later, oldest first.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn notices_since(
        self,
        session_id: &Identifier,
        since_ms: i64,
    ) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::NOTICES_SINCE)
            .map_err(sqlite("reading notices"))?;
        let rows = statement
            .query_map((session_id.as_str(), since_ms), |row| row.get(0))
            .map_err(sqlite("reading notices"))?;
        rows.collect::<Result<_, _>>()
            .map_err(sqlite("reading notices"))
    }

    /// The notices made at `since_ms` or later that wait for session `session_id`, oldest first.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn notices_waiting(
        self,
        session_id: &Identifier,
        since_ms: i64,
    ) -> Result<Vec<NoticeRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::NOTICES_WAITING)
            .map_err(sqlite("reading notices"))?;
        let rows = statement
            .query_map((session_id.as_str(), since_ms), |row| {
                Ok(NoticeRow {
                    id: NoticeId(row.get(0)?),
                    text: row.get(1)?,
                    created_ms: row.get(2)?,
                })
            })
            .map_err(sqlite("reading notices"))?;
        rows.collect::<Result<_, _>>()
            .map_err(sqlite("reading notices"))
    }
}
