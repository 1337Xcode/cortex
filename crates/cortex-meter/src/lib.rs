//! What agents' work costs in tokens, read from the transcripts the agents write as they work.
//!
//! An agent writes each of its sessions to a transcript, one JSON object a line, and each line
//! that reports a model's response carries the tokens that response took. [`Tail`] reads a
//! transcript from where it stopped last time, a line only once it is whole, and turns those lines
//! into [`Turn`]s: one per response, numbered in the order the transcript holds them, so that a
//! transcript read again from its start numbers them the same. How an agent writes its lines is a
//! [`TranscriptFormat`], one per agent, found with [`format_of`].
//!
//! Nothing here keeps what a transcript says beyond the counts, the model's name and the time: no
//! prompt, no answer, no tool's input or output.

#![warn(clippy::pedantic)]

mod cache_breaks;
mod claude;
mod codex;
mod error;
mod format;
mod holdout;
mod price_file;
mod pricing;
mod receipts;
mod report;
mod response;
mod tail;
mod timestamp;

pub use cache_breaks::{BreakCause, BreakContext, CacheBreak, cache_breaks};
pub use claude::Claude;
pub use codex::Codex;
pub use error::MeterError;
pub use format::{Earlier, TranscriptFormat, format_of};
pub use holdout::{
    BOOTSTRAP_RESAMPLES, Compared, Comparison, Interval, Weighed, compare, mean_interval,
    ratio_interval, weighed,
};
pub use price_file::{MAX_PRICE_FILE_BYTES, load_or_write};
pub use pricing::{
    Billed, Cost, ModelPrices, PriceTable, PricingError, Rates, SHIPPED_PRICES, Unpriced,
};
pub use receipts::{
    Activity, Arms, Estimate, MIN_TASKS_PER_ARM, Precision, SessionReceipt, Tally, TaskReceipt,
    receipt,
};
pub use report::{Holdout, Recorded, ReportError, Selection, recorded};
pub use response::{Response, SYNTHETIC_MODEL, Service, Usage};
pub use tail::{MAX_LINE_BYTES, MAX_RECENT_RESPONSES, Tail, Turn};
pub use timestamp::{utc_millis, utc_text};
