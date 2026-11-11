//! What the history worker hands the writer: the cards it made and the blame it kept, written in
//! one transaction on the writer's own thread, and where the writer says how that went. The worker
//! waits for the answer before its next job, so one write at most waits at a time.

use std::fmt;
use std::sync::mpsc::{self, Receiver, SyncSender};

use cortex_store::{BlameRecord, ChangeCardRecord, StoreError, Writer};

/// Cards made and a blame kept, for the writer.
#[derive(Default)]
pub(crate) struct HistoryWrite {
    /// Each card made, with its terms.
    pub(crate) cards: Vec<(ChangeCardRecord, Vec<String>)>,
    /// The blame kept, if the job made one.
    pub(crate) blame: Option<BlameRecord>,
}

impl fmt::Debug for HistoryWrite {
    /// The cards' commits and the blame, without the cards' terms: they are words of a person's
    /// message, which a debug line could carry into a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cards: Vec<&str> = self
            .cards
            .iter()
            .map(|(card, _)| card.oid.as_str())
            .collect();
        formatter
            .debug_struct("HistoryWrite")
            .field("cards", &cards)
            .field("blame", &self.blame)
            .finish()
    }
}

/// A write for the writer, with where to say how it went.
#[derive(Debug)]
pub(crate) struct HistoryWriteJob {
    /// What to write.
    pub(crate) write: HistoryWrite,
    /// Where the writer says whether it was stored, or why not.
    written: SyncSender<Result<(), String>>,
}

impl HistoryWriteJob {
    /// A job for `write`, and where its answer arrives.
    pub(crate) fn new(write: HistoryWrite) -> (Self, Receiver<Result<(), String>>) {
        let (written, answer) = mpsc::sync_channel(1);
        (Self { write, written }, answer)
    }

    /// Tells the worker whether the write was stored, or why not. A worker that stopped waiting is
    /// told nothing.
    pub(crate) fn answer(self, written: Result<(), String>) {
        drop(self.written.send(written));
    }
}

/// Writes `write` with `writer` in one transaction: the cards first, then the blame, so that a
/// blame read back finds its owners' cards.
///
/// # Errors
///
/// A card or blame the store refuses, and SQLite errors; nothing is written then.
pub(crate) fn store(writer: &mut Writer, write: &HistoryWrite) -> Result<(), StoreError> {
    // INVARIANT: single-writer. The worker never opens a writer: what it found is written here, on
    // the writer's own thread.
    let mut transaction = writer.transaction()?;
    for (card, tokens) in &write.cards {
        transaction.put_change_card(card, tokens)?;
    }
    if let Some(blame) = &write.blame {
        transaction.put_blame(blame)?;
    }
    transaction.commit()
}
