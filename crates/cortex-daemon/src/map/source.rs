//! Where the map's data comes from: the repository's store, opened for reading on each request,
//! the weights that bring stored scores up to now, the history worker's cache of commit subjects,
//! and the clock the one time-windowed number reads.
//!
//! The map never writes: [`Source::open`] gives a [`Reader`], and the only other things here are
//! numbers to weigh with and a read of text the daemon holds in memory.

use std::path::PathBuf;
use std::sync::Arc;

use cortex_retrieve::GraphSettings;
use cortex_store::{Reader, StoreError};

use super::settings::{HistoryLimits, MapSettings};
use crate::graph::Snapshot;
use crate::history::Jobs;
use crate::inbox::Inbox;

/// What the map's endpoints read.
#[derive(Debug)]
pub(crate) struct Source {
    /// The repository's `.cortex` folder.
    folder: PathBuf,
    /// How much of the database to memory map, as `mmap_bytes` says.
    mmap_bytes: u64,
    /// What brings a stored score or link weight forward to now.
    settings: GraphSettings,
    /// The history worker's jobs, whose cache holds the subjects of commits it has read.
    texts: Arc<Jobs>,
    /// The graph's snapshot, which counts the changes of the store.
    snapshot: Arc<Snapshot>,
    /// The writer's inbox, read only for whether the daemon's first index is done.
    inbox: Arc<Inbox>,
    /// The time now in UTC milliseconds; the one place the clock enters an answer, so that a test
    /// can fix it.
    clock: fn() -> i64,
    /// What the answers carry of the configuration.
    settings_shown: MapSettings,
}

impl Source {
    /// A source reading the store in `folder`, opened with `mmap_bytes`, weighing with
    /// `settings`, reading commit subjects from `texts`, told of the store's changes by
    /// `snapshot` and of the first index by `inbox`, and showing `settings_shown` of the
    /// configuration; its clock is the system's.
    pub(crate) fn new(
        (folder, mmap_bytes): (PathBuf, u64),
        settings: GraphSettings,
        (texts, snapshot, inbox): (Arc<Jobs>, Arc<Snapshot>, Arc<Inbox>),
        settings_shown: MapSettings,
    ) -> Self {
        Self {
            folder,
            mmap_bytes,
            settings,
            texts,
            snapshot,
            inbox,
            clock: crate::host::now_ms,
            settings_shown,
        }
    }

    /// This source with `clock` as its clock.
    #[cfg(test)]
    pub(crate) fn with_clock(self, clock: fn() -> i64) -> Self {
        Self { clock, ..self }
    }

    /// A reader of the store, opened for this one request.
    ///
    /// # Errors
    ///
    /// The store is not there yet, or not at this build's schema, or cannot be opened.
    pub(super) fn open(&self) -> Result<Reader, StoreError> {
        // INVARIANT: single-writer (the map only ever opens the store to read it: a `Reader`
        // has no way to write)
        Reader::open(&self.folder, self.mmap_bytes)
    }

    /// How many times the daemon noted that the store changed: a copy of what the store holds is
    /// out of date once this is not the number it was built at.
    pub(super) fn generation(&self) -> u64 {
        self.snapshot.generation()
    }

    /// What brings a stored score or link weight forward to now.
    pub(super) fn settings(&self) -> &GraphSettings {
        &self.settings
    }

    /// Whether the daemon's first index of the repository is done: until it is, the store holds
    /// only some of the repository's files, or none.
    pub(super) fn is_first_index_done(&self) -> bool {
        self.inbox.counts().is_indexed
    }

    /// The history Cortex reads, as the configuration limits it.
    pub(super) fn history_limits(&self) -> HistoryLimits {
        self.settings_shown.history
    }

    /// The address that opens the file at repository path `path` in the person's editor; none
    /// when `ui.editor_url` is empty.
    pub(super) fn editor_url(&self, path: &str) -> Option<String> {
        self.settings_shown.editor_url(path)
    }

    /// The time now in UTC milliseconds.
    pub(super) fn now_ms(&self) -> i64 {
        (self.clock)()
    }

    /// The subject of the commit `oid`, when the history worker's cache holds it; never read
    /// from git here.
    pub(super) fn commit_subject(&self, oid: &str) -> Option<String> {
        self.texts.text(oid).map(|text| text.subject)
    }
}
