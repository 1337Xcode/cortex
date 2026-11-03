//! Reading a transcript a little at a time, from where the last read stopped.
//!
//! An agent appends to its transcript while it works, so a read can end in the middle of a line;
//! that line is read once it is whole. A transcript shorter than where the last read stopped was
//! written again from its start, and is read again from there: its responses get the same numbers
//! as before, as they are numbered by their order in the file.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use crate::error::MeterError;
use crate::format::TranscriptFormat;
use crate::response::{Response, SYNTHETIC_MODEL, Usage};

/// Most bytes one read takes from a transcript. Whatever is beyond waits for the next read, so a
/// transcript that grew by hundreds of megabytes is taken in steps without holding them at once.
pub const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;

/// How many of its latest responses a tail remembers, to know a line that repeats one of them:
/// Claude Code writes a response's lines one after another, one per content block.
pub const MAX_RECENT_RESPONSES: usize = 64;

/// One response read from a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// Its place among the transcript's responses, from 1.
    pub ordinal: u32,
    /// When it was written, in UTC milliseconds.
    pub at_ms: i64,
    /// The model that answered.
    pub model: String,
    /// The tokens it took.
    pub usage: Usage,
}

/// A response the tail has counted, as far as telling a repeat of it goes.
#[derive(Debug, Clone)]
struct Counted {
    /// The response's message id.
    message_id: String,
    /// Its place among the transcript's responses.
    ordinal: u32,
    /// The most tokens a line of it reported.
    total: u64,
}

/// Where the reading of one transcript stands: how far it has read, how many responses it has
/// counted, and the latest of them.
#[derive(Debug, Clone, Default)]
pub struct Tail {
    /// The byte offset reading goes on from: the end of the last whole line read.
    offset: u64,
    /// How many responses have been counted.
    counted: u32,
    /// The latest responses counted, the oldest first.
    recent: VecDeque<Counted>,
}

impl Tail {
    /// A tail that has read nothing of its transcript.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The byte offset the next read starts from.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Reads `file`, a transcript written in `format`, from where the last read stopped, at most
    /// `max_bytes` of it ([`MAX_READ_BYTES`] for the daemon), and returns the responses its new
    /// whole lines report, in order. A line that repeats a response already counted, with no more
    /// tokens, adds nothing; with more, it comes again with the response's number and the larger
    /// counts, which replace the earlier ones. A file shorter than where the last read stopped is
    /// read from its start, as a tail that has read nothing.
    ///
    /// # Errors
    ///
    /// [`MeterError::Read`] when the file cannot be measured or read; the tail is then as it was.
    pub fn read(
        &mut self,
        file: &mut File,
        format: &dyn TranscriptFormat,
        max_bytes: u64,
    ) -> Result<Vec<Turn>, MeterError> {
        let length = file.metadata()?.len();
        let start = if length < self.offset { 0 } else { self.offset };
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(max_bytes).read_to_end(&mut bytes)?;
        if start < self.offset {
            *self = Self::default();
        }
        Ok(self.take(&bytes, format, max_bytes))
    }

    /// Counts the responses the whole lines of `bytes`, read from the offset, report, moving the
    /// offset past them. A read of the whole `max_bytes` that holds no line ending at all is a line
    /// longer than any response's: it is passed over, its rest read next time as a broken line.
    fn take(&mut self, bytes: &[u8], format: &dyn TranscriptFormat, max_bytes: u64) -> Vec<Turn> {
        let mut turns = Vec::new();
        let mut consumed: u64 = 0;
        for piece in bytes.split_inclusive(|byte| *byte == b'\n') {
            let Some(line) = piece.strip_suffix(b"\n") else {
                break;
            };
            consumed += u64::try_from(piece.len()).unwrap_or(u64::MAX);
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if let Some(response) = format.response(line) {
                self.count(response, &mut turns);
            }
        }
        let is_one_long_line = consumed == 0
            && u64::try_from(bytes.len()).is_ok_and(|length| length >= max_bytes && length > 0);
        if is_one_long_line {
            consumed = max_bytes;
        }
        self.offset = self.offset.saturating_add(consumed);
        turns
    }

    /// Counts `response`, adding to `turns` what it changes: a new response, or a repeat of a
    /// recent one, a copy or a replay, with more tokens. A response of the synthetic model changes
    /// nothing.
    fn count(&mut self, response: Response, turns: &mut Vec<Turn>) {
        if response.model == SYNTHETIC_MODEL {
            return;
        }
        let total = response.usage.total();
        let repeated = self
            .recent
            .iter_mut()
            .find(|seen| seen.message_id == response.message_id);
        if let Some(seen) = repeated {
            if total > seen.total {
                seen.total = total;
                turns.push(turn(seen.ordinal, response));
            }
            return;
        }
        self.counted = self.counted.saturating_add(1);
        // INVARIANT: budgeted (a tail remembers at most MAX_RECENT_RESPONSES responses)
        if self.recent.len() == MAX_RECENT_RESPONSES {
            self.recent.pop_front();
        }
        self.recent.push_back(Counted {
            message_id: response.message_id.clone(),
            ordinal: self.counted,
            total,
        });
        turns.push(turn(self.counted, response));
    }
}

/// The turn `response` is, numbered `ordinal`.
fn turn(ordinal: u32, response: Response) -> Turn {
    Turn {
        ordinal,
        at_ms: response.at_ms,
        model: response.model,
        usage: response.usage,
    }
}
