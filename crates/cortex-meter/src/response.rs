//! One model response as a transcript line reports it, and the tokens it took.

/// The model name Claude Code gives the responses it writes itself, without calling a model; they
/// cost nothing and are not counted.
pub const SYNTHETIC_MODEL: &str = "<synthetic>";

/// The tokens one response took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Input read at the full price: not written to the cache nor read from it.
    pub input: u64,
    /// Input written to the cache, at either lifetime.
    pub cache_write: u64,
    /// Of `cache_write`, what was written to the one-hour cache, when the transcript says.
    pub cache_write_1h: Option<u64>,
    /// Input read from the cache.
    pub cache_read: u64,
    /// Output, reasoning included.
    pub output: u64,
    /// Of `output`, the tokens spent reasoning, when the transcript reports them.
    pub reasoning: Option<u64>,
}

impl Usage {
    /// Every input and output token together, the reasoning counted once as part of the output:
    /// how a response written several times is told complete, the copy with the most being the
    /// one kept.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.cache_write)
            .saturating_add(self.cache_read)
            .saturating_add(self.output)
    }
}

/// One response a transcript line reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The id the model's provider gave the message, which names one response: a line that
    /// repeats it, under the same request or a replay under another, is that response again.
    pub message_id: String,
    /// When the line was written, in UTC milliseconds.
    pub at_ms: i64,
    /// The model that answered.
    pub model: String,
    /// The tokens it took.
    pub usage: Usage,
}
