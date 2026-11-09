//! The one writer of a repository's database, and its transactions.

use std::path::{Path, PathBuf};

use cortex_core::{
    EdgeType, FactStatus, Identifier, RepoId, RepoPath, ScaleMode, SessionStatus, TaskOutcome, Ulid,
};
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::activity::{self, EventRecord, SessionRecord, TaskRecord};
use crate::archetype::{self, ArchetypeRecord};
use crate::calibration::{self, CalibrationAttempt};
use crate::checkpoint::{CheckpointFailures, Checkpointer};
use crate::claims;
use crate::cochange::{self, FileCommitsRow, GitStateKey, PairRow};
use crate::connection::{self, Access};
use crate::coordination::{self, NoticeRecord, PositionRecord};
use crate::episode::{self, EpisodeRecord};
use crate::error::{StoreError, io, sqlite};
use crate::facts::{self, FactAnchor, FactRecord};
use crate::fading;
use crate::folders;
use crate::handoffs::{self, HandoffRecord};
use crate::ids::{FactId, HandoffId, InjectionId, NodeId, NoticeId, WarningId};
use crate::injections::{self, InjectionRecord};
use crate::interest::{self, InterestRow};
use crate::layout;
use crate::links;
use crate::lock::WriterLock;
use crate::meta::MetaKey;
use crate::migrate;
use crate::queries::Queries;
use crate::records::{EdgeRecord, FileChange, FileIndex, FileRemoval, Resolved};
use crate::replace;
use crate::retention;
use crate::schema;
use crate::self_check::{self, SelfCheckRecord};
use crate::transitions::{self, TransitionRow};
use crate::usage::{self, UsageRow};
use crate::warnings::{self, WarningRecord};

/// What [`Writer::open`] needs.
#[derive(Debug, Clone, Copy)]
pub struct WriterOptions<'a> {
    /// The repository's `.cortex` folder, created when missing.
    pub folder: &'a Path,
    /// The repository's id, recorded in `meta`.
    pub repo_id: &'a RepoId,
    /// How much of the database SQLite may map into memory (`store.mmap_bytes`).
    pub mmap_bytes: u64,
    /// The time in UTC milliseconds, recorded as a new database's creation time.
    pub now_ms: i64,
}

/// The one writer of a repository's database. While it exists, no other writer can open the
/// same database, in this process or any other. A thread of its own checkpoints the write-ahead
/// log every second; its commits do only once the log passes 64 MiB.
#[derive(Debug)]
pub struct Writer {
    // Fields drop in order: the checkpointer stops and closes its connection, then the writer's
    // connection closes, the last one, so SQLite checkpoints and removes the log; the lock is
    // released after both, so a new writer never opens the database while this one is closing it.
    /// The thread that checkpoints the log.
    checkpointer: Checkpointer,
    /// The read-write connection.
    connection: Connection,
    /// The lock that makes this the only writer.
    _lock: WriterLock,
    /// The write-ahead log's file.
    log: PathBuf,
}

impl Writer {
    /// Takes the writer lock, opens the database and brings its schema up to date, backing it up
    /// first when it already has one.
    ///
    /// # Errors
    ///
    /// [`StoreError::WriterBusy`] when another writer has the database, the schema errors of a
    /// migration, and I/O and SQLite errors.
    pub fn open(options: &WriterOptions<'_>) -> Result<Self, StoreError> {
        cortex_core::create_private_folder(options.folder)
            .map_err(io("creating the store folder", options.folder))?;
        let lock = WriterLock::acquire(&layout::writer_lock(options.folder))?;
        let database = layout::database(options.folder);
        // Created here, private, before SQLite creates it with the default permissions.
        drop(
            cortex_core::open_private_file(&database)
                .map_err(io("creating the database", &database))?,
        );
        let mut connection = connection::connect(&database, Access::ReadWrite)?;
        // Checked before any setting is applied: a database Cortex did not write, or one a newer
        // Cortex wrote, is left exactly as it was.
        migrate::check(&connection, schema::MIGRATIONS)?;
        connection::configure(&connection, Access::ReadWrite, options.mmap_bytes)?;
        migrate::migrate(
            &mut connection,
            schema::MIGRATIONS,
            &layout::backups(options.folder),
        )?;
        record_identity(&mut connection, options)?;
        let checkpointer = Checkpointer::start(&database, options.mmap_bytes)?;
        Ok(Self {
            checkpointer,
            connection,
            _lock: lock,
            log: layout::log(options.folder),
        })
    }

    /// Starts a transaction. Only one can be open at a time.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn transaction(&mut self) -> Result<WriteTx<'_>, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite("starting a transaction"))?;
        Ok(WriteTx { transaction })
    }

    /// How many bytes the write-ahead log's file holds, 0 when there is none. SQLite cuts the
    /// file back to 4 MiB on the first commit after the log starts afresh, so past that its size
    /// is how much the log holds.
    ///
    /// # Errors
    ///
    /// I/O errors other than the file not being there.
    pub fn log_bytes(&self) -> Result<u64, StoreError> {
        match std::fs::metadata(&self.log) {
            Ok(metadata) => Ok(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(io("reading the size of the write-ahead log", &self.log)(
                error,
            )),
        }
    }

    /// Read queries over the committed database.
    #[must_use]
    pub fn queries(&self) -> Queries<'_> {
        Queries::new(&self.connection)
    }

    /// The count of the checkpoints that failed since the writer opened, for the daemon's status:
    /// a log that is not checkpointed only grows, and nothing committed is lost.
    #[must_use]
    pub fn checkpoint_failures(&self) -> CheckpointFailures {
        self.checkpointer.failures()
    }
}

/// The version of this Cortex, recorded when it completes an index.
const CORTEX_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Records which repository the database belongs to, and when it was created.
fn record_identity(
    connection: &mut Connection,
    options: &WriterOptions<'_>,
) -> Result<(), StoreError> {
    let transaction = connection
        .transaction()
        .map_err(sqlite("starting a transaction"))?;
    let entries = [
        (
            schema::META_SET,
            MetaKey::RepoId,
            options.repo_id.as_str().to_owned(),
        ),
        (
            schema::META_SET_ONCE,
            MetaKey::CreatedAt,
            options.now_ms.to_string(),
        ),
    ];
    for (sql, key, value) in entries {
        transaction
            .execute(sql, (key.name(), value))
            .map_err(sqlite("writing meta"))?;
    }
    transaction.commit().map_err(sqlite("writing meta"))
}

/// One transaction of changes. Readers see none of them until [`WriteTx::commit`]; dropping the
/// transaction without committing undoes them all. Each write is all or nothing on its own: when
/// one fails, its partial changes are undone and the transaction keeps the earlier writes.
#[derive(Debug)]
pub struct WriteTx<'w> {
    /// The open transaction.
    transaction: Transaction<'w>,
}

impl WriteTx<'_> {
    /// Writes everything the index records about one file over what was stored for it, keeping
    /// the ids of symbols that match one already stored by qualified name and kind. The file's
    /// references are written unresolved; the file's and each symbol's node get their search
    /// rows, and the nodes of symbols no longer defined lose theirs.
    ///
    /// # Errors
    ///
    /// [`StoreError::Position`] when a symbol's parent does not come before it or a reference
    /// names a symbol the file does not have, [`StoreError::TransactionLost`], and SQLite errors.
    pub fn replace_file(&mut self, index: &FileIndex) -> Result<FileChange, StoreError> {
        self.atomically(|connection| replace::replace_file(connection, index))
    }

    /// Deletes everything stored for the file at `path`, its nodes' search rows included.
    /// References elsewhere to its symbols become unresolved. `None` when nothing was stored.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_file(&mut self, path: &RepoPath) -> Result<Option<FileRemoval>, StoreError> {
        self.atomically(|connection| replace::remove_file(connection, path))
    }

    /// Ties each reference to its target, or marks it unresolved; the confidence follows from
    /// the resolution.
    ///
    /// # Errors
    ///
    /// [`StoreError::UnknownRow`] for a reference that does not exist,
    /// [`StoreError::TransactionLost`], and SQLite errors, including a target symbol that does not
    /// exist. Either all resolutions are written or none.
    pub fn set_resolutions(&mut self, resolutions: &[Resolved]) -> Result<(), StoreError> {
        self.atomically(|connection| links::set_resolutions(connection, resolutions))
    }

    /// Forgets which Cortex wrote the index, before an index writes anything: one that stops
    /// partway leaves no record, and the next index reads every file again.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn clear_cortex_version(&mut self) -> Result<(), StoreError> {
        self.atomically(|connection| {
            connection
                .execute(schema::META_DELETE, [MetaKey::CortexVersion.name()])
                .map(drop)
                .map_err(sqlite("clearing the Cortex version"))
        })
    }

    /// Records this Cortex as the one whose rules wrote the index, once an index is complete.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn set_cortex_version(&mut self) -> Result<(), StoreError> {
        self.atomically(|connection| {
            connection
                .execute(
                    schema::META_SET,
                    (MetaKey::CortexVersion.name(), CORTEX_VERSION),
                )
                .map(drop)
                .map_err(sqlite("writing the Cortex version"))
        })
    }

    /// Sets the modification time of the stored file at `path` to `mtime_ns`, and when the index
    /// read it to `indexed_at_ms`, for a file read again and found to hold what is stored: the row
    /// then says the index saw those contents at that time. Returns whether the store holds the
    /// file.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn touch_file(
        &mut self,
        path: &RepoPath,
        mtime_ns: i64,
        indexed_at_ms: i64,
    ) -> Result<bool, StoreError> {
        self.atomically(|connection| {
            connection
                .execute(schema::FILE_TOUCH, (path.as_str(), mtime_ns, indexed_at_ms))
                .map(|rows| rows > 0)
                .map_err(sqlite("writing a file's times"))
        })
    }

    /// Records `mode` as the scale mode the index was built for.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn set_scale_mode(&mut self, mode: ScaleMode) -> Result<(), StoreError> {
        self.atomically(|connection| {
            connection
                .execute(schema::META_SET, (MetaKey::ScaleMode.name(), mode.name()))
                .map(drop)
                .map_err(sqlite("writing the scale mode"))
        })
    }

    /// Replaces the edges of `edge_type` that start at `src` with `edges`.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors, including a node that does not exist and
    /// two edges to the same node. Either all edges are written or none.
    pub fn replace_edges(
        &mut self,
        src: NodeId,
        edge_type: EdgeType,
        edges: &[EdgeRecord],
    ) -> Result<(), StoreError> {
        self.atomically(|connection| links::replace_edges(connection, src, edge_type, edges))
    }

    /// The node of the folder at `path`, added when it has none, with its search row. A folder
    /// keeps its node, and everything attached to it, until the node is removed.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn folder_node(&mut self, path: &RepoPath) -> Result<NodeId, StoreError> {
        self.atomically(|connection| folders::folder_node(connection, path))
    }

    /// Deletes the node of the folder at `path`, its search row and every edge at it. `false` when
    /// the folder had no node.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_folder_node(&mut self, path: &RepoPath) -> Result<bool, StoreError> {
        self.atomically(|connection| folders::remove_folder_node(connection, path))
    }

    /// Adds the session, or updates a stored one: facts given replace those stored, facts not
    /// given keep theirs, the first start time stays, and the session is live again.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotText`] for a path that is not UTF-8, [`StoreError::TransactionLost`], and
    /// SQLite errors.
    pub fn record_session(&mut self, session: &SessionRecord) -> Result<(), StoreError> {
        self.atomically(|connection| activity::record_session(connection, session))
    }

    /// Marks the session ended at the time of `ended`, with its status: ended, or cleared by the
    /// user. `false` when no such session is stored.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn end_session(
        &mut self,
        session_id: &Identifier,
        ended: (i64, SessionStatus),
    ) -> Result<bool, StoreError> {
        self.atomically(|connection| activity::end_session(connection, session_id, ended))
    }

    /// Adds the task, numbered one past every task stored, and returns its number.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors, including a session that is not stored
    /// and a task id already used.
    pub fn start_task(&mut self, task: &TaskRecord) -> Result<u64, StoreError> {
        self.atomically(|connection| activity::start_task(connection, task))
    }

    /// Ends the open task `task_id` with `outcome` at `ended_at_ms`. `false` when no such task is
    /// open.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn end_task(
        &mut self,
        task_id: Ulid,
        ended_at_ms: i64,
        outcome: TaskOutcome,
    ) -> Result<bool, StoreError> {
        self.atomically(|connection| activity::end_task(connection, task_id, ended_at_ms, outcome))
    }

    /// Sets what became of the ended task `task_id` to `outcome`, with `evidence`, the words that
    /// say why: what a check vouched for, or what put the task's files back, never a command
    /// line, a path or code. A later judgement replaces an earlier one, as when the next task shows
    /// this one was put back. `false` when no such task has ended.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn set_task_outcome(
        &mut self,
        task_id: Ulid,
        outcome: TaskOutcome,
        evidence: Option<&str>,
    ) -> Result<bool, StoreError> {
        self.atomically(|connection| {
            activity::set_task_outcome(connection, task_id, outcome, evidence)
        })
    }

    /// Appends the event and returns its number.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn append_event(&mut self, event: &EventRecord) -> Result<u64, StoreError> {
        self.atomically(|connection| activity::append_event(connection, event))
    }

    /// Deletes every file's commit counts and every pair's co-change counts, before a full mining
    /// of the history adds its own.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn clear_cochange(&mut self) -> Result<(), StoreError> {
        self.atomically(cochange::clear)
    }

    /// Records `record`, what an injection gave or weighed, returning its row.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn record_injection(
        &mut self,
        record: &InjectionRecord,
    ) -> Result<InjectionId, StoreError> {
        self.atomically(|connection| injections::record(connection, record))
    }

    /// Keeps `record` as what the self-check last found.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn set_self_check(&mut self, record: &SelfCheckRecord) -> Result<(), StoreError> {
        self.atomically(|connection| self_check::set(connection, record))
    }

    /// Keeps a confidence model with `coefficients`, fitted on `samples` labelled injections at
    /// `trained_at_ms`, whose AUC on the labelled injections held out from its fitting is `auc`,
    /// as the newest, and deletes all but the newest `keep_versions` models: its version.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn add_calibration(
        &mut self,
        model: (&[f64], u32, f64, i64),
        keep_versions: u32,
    ) -> Result<u32, StoreError> {
        self.atomically(|connection| calibration::add(connection, model, keep_versions))
    }

    /// Sets each of `rows`, the transitions from a file to the file tasks read next, adding a pair
    /// that is not there.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn set_transitions(&mut self, rows: &[TransitionRow]) -> Result<(), StoreError> {
        self.atomically(|connection| transitions::set(connection, rows))
    }

    /// Removes the transitions from each pair's first file to its second: how many.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_transitions(&mut self, pairs: &[(NodeId, NodeId)]) -> Result<u64, StoreError> {
        self.atomically(|connection| transitions::remove(connection, pairs))
    }

    /// Keeps `attempt` as the last time the confidence model was fitted.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn set_calibration_attempt(
        &mut self,
        attempt: &CalibrationAttempt,
    ) -> Result<(), StoreError> {
        self.atomically(|connection| calibration::set_attempt(connection, attempt))
    }

    /// Sets what the end of its task found of injection `id`: `used_count`, how many of its files
    /// the task read or edited, and `label`, whether one of its first three was used. Labelling it
    /// again replaces both. Whether the store holds the injection.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn label_injection(
        &mut self,
        id: InjectionId,
        used_count: u32,
        label: bool,
    ) -> Result<bool, StoreError> {
        self.atomically(|connection| injections::label(connection, id, (used_count, label)))
    }

    /// Records `record`, one file a warning named, returning its row.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors, among them a task the store does not hold.
    pub fn record_warning(&mut self, record: &WarningRecord) -> Result<WarningId, StoreError> {
        self.atomically(|connection| warnings::record(connection, record))
    }

    /// Keeps `record` as its session's position on its node, in place of the one kept before:
    /// the caller decides which of the two holds longer.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn keep_position(&mut self, record: &PositionRecord) -> Result<(), StoreError> {
        self.atomically(|connection| coordination::keep_position(connection, record))
    }

    /// Refreshes the query planner's statistics where they are stale.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn optimize(&mut self) -> Result<(), StoreError> {
        self.atomically(retention::optimize)
    }

    /// Gives back to the file system at most `pages` free pages of the database file (none from
    /// a database made before it vacuumed incrementally): how many it gave back.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn vacuum(&mut self, pages: u32) -> Result<u64, StoreError> {
        self.atomically(|connection| retention::vacuum(connection, pages))
    }

    /// Removes the links of use from each pair's first node to its second: how many.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_coaccess_links(&mut self, links: &[(NodeId, NodeId)]) -> Result<u64, StoreError> {
        self.atomically(|connection| fading::remove_links(connection, links))
    }

    /// Removes the interest of `nodes`, which then have none: how many.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_interest(&mut self, nodes: &[NodeId]) -> Result<u64, StoreError> {
        self.atomically(|connection| fading::remove_interest(connection, nodes))
    }

    /// Deletes those of the `limit` first events recorded that are older than `before_ms`: how
    /// many. An event recorded after a later one keeps the older ones behind it.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn delete_events_before(&mut self, before_ms: i64, limit: u64) -> Result<u64, StoreError> {
        self.atomically(|connection| retention::delete_events(connection, (before_ms, limit)))
    }

    /// Deletes at most `limit` episodes past the `keep` most worth keeping, those whose outcome is
    /// unknown first, then the oldest: how many.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn cap_episodes(&mut self, keep: u64, limit: u64) -> Result<u64, StoreError> {
        self.atomically(|connection| retention::cap_episodes(connection, (keep, limit)))
    }

    /// Deletes the positions last taken before `before_ms`, which can no longer hold: how many.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn delete_positions_before(&mut self, before_ms: i64) -> Result<u64, StoreError> {
        self.atomically(|connection| retention::delete_positions(connection, before_ms))
    }

    /// Deletes at most `limit` facts retired before `before_ms`, with their anchors and the
    /// record of the tasks they were given in: how many facts.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn delete_retired_facts_before(
        &mut self,
        before_ms: i64,
        limit: u64,
    ) -> Result<u64, StoreError> {
        self.atomically(|connection| {
            retention::delete_retired_facts(connection, (before_ms, limit))
        })
    }

    /// Releases the claims taken at `since_ms` or later on `nodes`, whoever holds them, or on every
    /// node when `nodes` is none: how many were released. The caller gives when a claim still
    /// holding was taken at the earliest.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn release_claims(
        &mut self,
        nodes: Option<&[NodeId]>,
        since_ms: i64,
    ) -> Result<u64, StoreError> {
        self.atomically(|connection| claims::release(connection, (nodes, since_ms)))
    }

    /// Queues `notice` for its session's next hook that carries text, unless the session was sent
    /// or given, at `unless_since_ms` or later, a notice whose subject covers this one's: its row,
    /// or none when it was.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn queue_notice(
        &mut self,
        notice: &NoticeRecord,
        unless_since_ms: i64,
    ) -> Result<Option<NoticeId>, StoreError> {
        self.atomically(|connection| {
            coordination::queue_notice(connection, notice, unless_since_ms)
        })
    }

    /// Leaves `record` as its session's open handoff, in place of any it left before: its row.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn leave_handoff(&mut self, record: &HandoffRecord) -> Result<HandoffId, StoreError> {
        self.atomically(|connection| handoffs::leave(connection, record))
    }

    /// Closes handoff `id`, as picked up by session `by`: the session given it, or the one that
    /// left it once it finished the work itself. `false` when it was closed already.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn close_handoff(&mut self, id: HandoffId, by: &Identifier) -> Result<bool, StoreError> {
        // INVARIANT: single-writer (a handoff is closed only through the writer, by its id)
        self.atomically(|connection| handoffs::close(connection, id, by))
    }

    /// Records `notice` as told to its session as it was made: given in the answer to the hook
    /// that made it, never waiting.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn record_told(&mut self, notice: &NoticeRecord) -> Result<(), StoreError> {
        self.atomically(|connection| coordination::record_told(connection, notice))
    }

    /// Adds the fact `record` with its anchors: its id.
    ///
    /// # Errors
    ///
    /// [`StoreError::FactTooLong`], [`StoreError::FactUnanchored`],
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn add_fact(&mut self, record: &FactRecord) -> Result<FactId, StoreError> {
        self.atomically(|connection| facts::add(connection, record))
    }

    /// Sets fact `id`'s status to `status` at `at_ms`; one made active counts as confirmed then.
    ///
    /// # Errors
    ///
    /// [`StoreError::UnknownRow`] for no such fact, [`StoreError::TransactionLost`] and SQLite
    /// errors.
    pub fn set_fact_status(
        &mut self,
        id: FactId,
        status: FactStatus,
        at_ms: i64,
    ) -> Result<(), StoreError> {
        self.atomically(|connection| facts::set_status(connection, id, (status, at_ms)))
    }

    /// Marks fact `old` superseded by fact `new`, which corrects it: the old one stays as history,
    /// hidden from readers of current facts. Whether `old` was current.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn supersede_fact(&mut self, old: FactId, new: FactId) -> Result<bool, StoreError> {
        self.atomically(|connection| facts::supersede(connection, old, new))
    }

    /// Records `task` as the one that first asserted fact `id`, unless one already is: an
    /// imported fact an agent asserts in a task. Whether it was recorded.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn note_fact_task(&mut self, id: FactId, task: Ulid) -> Result<bool, StoreError> {
        self.atomically(|connection| facts::note_task(connection, id, task))
    }

    /// Gives each of fact `id`'s `anchors` its hash there, as when the fact is confirmed against
    /// the code as it is now.
    ///
    /// # Errors
    ///
    /// [`StoreError::UnknownRow`] for a node the fact is not anchored to,
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn restamp_fact(&mut self, id: FactId, anchors: &[FactAnchor]) -> Result<(), StoreError> {
        self.atomically(|connection| facts::restamp(connection, id, anchors))
    }

    /// Counts one use of each of `ids`, given to an agent, and records that `task` was given them
    /// when they were given in one.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn give_facts(&mut self, task: Option<Ulid>, ids: &[FactId]) -> Result<(), StoreError> {
        self.atomically(|connection| facts::give(connection, task, ids))
    }

    /// Marks notice `id` delivered at `at_ms`. Whether it was waiting.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn deliver_notice(&mut self, id: NoticeId, at_ms: i64) -> Result<bool, StoreError> {
        self.atomically(|connection| coordination::deliver_notice(connection, id, at_ms))
    }

    /// Sets whether warning `id` was resolved: its file read or edited after it in its task.
    /// Resolving it again replaces the value. Whether the store holds the warning.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn resolve_warning(
        &mut self,
        id: WarningId,
        is_resolved: bool,
    ) -> Result<bool, StoreError> {
        self.atomically(|connection| warnings::resolve(connection, id, is_resolved))
    }

    /// Adds `files` and `pairs` to the stored counts, storing those not stored yet: counts and
    /// weights summed, the newest shared commit kept.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`], [`StoreError::TooLarge`] for a sequence number past what
    /// SQLite stores, and SQLite errors, including a file that does not exist. Either all are
    /// written or none.
    pub fn add_cochange(
        &mut self,
        files: &[FileCommitsRow],
        pairs: &[PairRow],
    ) -> Result<(), StoreError> {
        self.atomically(|connection| cochange::add(connection, files, pairs))
    }

    /// Sets each edge of `edge_type`: its weight, how many observations support it, the task it
    /// was last seen at and whether it points one way, adding an edge that is not there and
    /// leaving every other edge of the type alone. For links a task learns, which change one pair
    /// at a time, where [`WriteTx::replace_edges`] would drop a node's other edges.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors, including a node that does not exist.
    /// Either all are written or none.
    pub fn set_edges(
        &mut self,
        edge_type: EdgeType,
        edges: &[(NodeId, EdgeRecord)],
    ) -> Result<(), StoreError> {
        self.atomically(|connection| links::set_edges(connection, edge_type, edges))
    }

    /// Writes every cluster of tasks, replacing the clusters stored before. The clustering
    /// reads every record and writes its clusters whole, so they are replaced together.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`], [`StoreError::NotText`] for a cluster that cannot be
    /// written as JSON, and SQLite errors. Either all are written or none.
    pub fn replace_archetypes(&mut self, archetypes: &[ArchetypeRecord]) -> Result<(), StoreError> {
        self.atomically(|connection| archetype::replace(connection, archetypes))
    }

    /// Writes one task's record, replacing the one the task had.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`], [`StoreError::TooLarge`] for a record whose parts
    /// hold more than [`crate::MAX_EPISODE_BYTES`] as JSON, [`StoreError::NotText`] for a
    /// part that cannot be written as JSON, and SQLite errors, including a task that does
    /// not exist.
    pub fn replace_episode(&mut self, episode: &EpisodeRecord) -> Result<(), StoreError> {
        self.atomically(|connection| episode::replace(connection, episode))
    }

    /// Writes each node's interest, replacing what it had.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`], [`StoreError::TooLarge`] for a task number past what
    /// SQLite stores, and SQLite errors, including a node that does not exist. Either all are
    /// written or none.
    pub fn replace_interest(&mut self, rows: &[InterestRow]) -> Result<(), StoreError> {
        self.atomically(|connection| interest::replace(connection, rows))
    }

    /// Writes each response's usage, replacing what its session and turn had.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`], [`StoreError::TooLarge`] for a count past what SQLite
    /// stores, and SQLite errors, including a session that does not exist. Either all are
    /// written or none.
    pub fn replace_usage(&mut self, rows: &[UsageRow]) -> Result<(), StoreError> {
        self.atomically(|connection| usage::replace(connection, rows))
    }

    /// Removes every usage row of `session_id` numbered past `last_turn`: its transcript was read
    /// again from its start to its end and holds no more responses than that, so a row past it
    /// is one the file no longer holds.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn forget_usage_after(
        &mut self,
        session_id: &Identifier,
        last_turn: u32,
    ) -> Result<(), StoreError> {
        self.atomically(|connection| usage::forget_after(connection, (session_id, last_turn)))
    }

    /// Sets the co-change miner's state under `key` to `value`.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn set_git_state(&mut self, key: GitStateKey, value: &str) -> Result<(), StoreError> {
        self.atomically(|connection| cochange::set_state(connection, key, value))
    }

    /// Deletes the co-change miner's state under `key`.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_git_state(&mut self, key: GitStateKey) -> Result<(), StoreError> {
        self.atomically(|connection| cochange::remove_state(connection, key))
    }

    /// Deletes every edge of `edge_type`, before a full set is written again.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_edges_of_type(&mut self, edge_type: EdgeType) -> Result<(), StoreError> {
        self.atomically(|connection| links::remove_edges_of_type(connection, edge_type))
    }

    /// Read queries that also see this transaction's changes.
    #[must_use]
    pub fn queries(&self) -> Queries<'_> {
        Queries::new(&self.transaction)
    }

    /// Runs `write` inside a savepoint, so that a write that fails part of the way through
    /// leaves the transaction as it was before it.
    fn atomically<T>(
        &mut self,
        write: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.refuse_if_lost()?;
        let savepoint = self
            .transaction
            .savepoint()
            .map_err(sqlite("starting a savepoint"))?;
        let value = write(&savepoint)?;
        savepoint
            .commit()
            .map_err(sqlite("releasing a savepoint"))?;
        Ok(value)
    }

    /// Refuses to go on when SQLite has rolled the transaction back on its own, as it may after a
    /// full disk, an I/O error or running out of memory. A savepoint begun then would start a new
    /// transaction of its own and commit each later write by itself.
    fn refuse_if_lost(&self) -> Result<(), StoreError> {
        if self.transaction.is_autocommit() {
            Err(StoreError::TransactionLost)
        } else {
            Ok(())
        }
    }

    /// Makes every change visible to readers.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] when SQLite already rolled the transaction back, and SQLite
    /// errors; either way nothing was written.
    pub fn commit(self) -> Result<(), StoreError> {
        self.refuse_if_lost()?;
        self.transaction
            .commit()
            .map_err(sqlite("committing a transaction"))
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::{FileFlags, RepoId};

    use super::*;
    use crate::connection::LOG_RESTART_PAGES;
    use crate::records::FileRecord;

    /// A file with no symbols or references at `path`.
    fn empty_file(path: &str) -> Result<FileIndex, Box<dyn std::error::Error>> {
        Ok(FileIndex {
            file: FileRecord {
                path: RepoPath::new(path)?,
                language: None,
                size_bytes: 1,
                line_count: None,
                content_hash: None,
                mtime_ns: 1,
                flags: FileFlags::empty(),
                indexed_at_ms: 1,
            },
            symbols: Vec::new(),
            refs: Vec::new(),
            passes_on: Vec::new(),
        })
    }

    /// Commits made back to back, which never leave the checkpointer the moment it needs to copy
    /// the whole log, still let the log start afresh once it passes [`LOG_RESTART_PAGES`]: 160
    /// commits of half a mebibyte each, 80 MiB in all, leave a log file no larger than the restart
    /// size and a commit's frames. A frame is a 4,096-byte page and its 24-byte header, after the
    /// file's 32-byte header. Once the log starts afresh its file is cut back, so the last 16 MiB
    /// leave it smaller than its largest.
    #[test]
    fn the_log_starts_afresh_under_back_to_back_commits() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempfile::tempdir()?;
        let folder = root.path().join(".cortex");
        let mut writer = Writer::open(&WriterOptions {
            folder: &folder,
            repo_id: &RepoId::of(root.path()),
            mmap_bytes: 0,
            now_ms: 1,
        })?;
        // A table of this test's own: what it holds does not matter, only how many pages it takes.
        writer
            .connection
            .execute_batch("CREATE TABLE filler (bytes BLOB NOT NULL)")?;
        let log = folder.join("cortex.db-wal");
        let mut largest = 0;
        for _ in 0..160 {
            let transaction = writer.transaction()?;
            transaction
                .transaction
                .execute("INSERT INTO filler VALUES (zeroblob(524288))", [])?;
            transaction.commit()?;
            largest = largest.max(std::fs::metadata(&log)?.len());
        }
        let commit_pages = 512;
        let bound = u64::try_from(32 + (LOG_RESTART_PAGES + commit_pages) * 4_120)?;
        assert!(largest <= bound, "a log of {largest} bytes");
        let last = std::fs::metadata(&log)?.len();
        assert!(last < largest, "a log of {largest} bytes, then {last}");
        Ok(())
    }

    /// Once SQLite has rolled a transaction back on its own, as it may after a full disk, later
    /// writes and the commit are refused, and nothing written after the rollback becomes visible.
    #[test]
    fn a_lost_transaction_refuses_more_writes() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let repo_id = RepoId::of(root.path());
        let mut writer = Writer::open(&WriterOptions {
            folder: &root.path().join(".cortex"),
            repo_id: &repo_id,
            mmap_bytes: 1 << 20,
            now_ms: 1,
        })?;
        let mut transaction = writer.transaction()?;
        transaction.replace_file(&empty_file("a.py")?)?;
        // What SQLite does to the transaction after such an error.
        transaction.transaction.execute_batch("ROLLBACK")?;
        let error = transaction.replace_file(&empty_file("b.py")?).err();
        assert!(
            matches!(error, Some(StoreError::TransactionLost)),
            "{error:?}"
        );
        let error = transaction.commit().err();
        assert!(
            matches!(error, Some(StoreError::TransactionLost)),
            "{error:?}"
        );
        assert_eq!(writer.queries().counts()?.files, 0);
        Ok(())
    }
}
