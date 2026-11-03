//! One model response as a transcript line reports it, the tokens it took and how it was served.

use cortex_core::Identifier;

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

/// How a response was served, as far as its price depends on it, each part only when the line
/// names it with a name: the service tier (`standard`, or another the provider prices apart), the
/// speed (`standard` or `fast`) and where it ran (`global`, or `us` for inference kept in the
/// United States).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Service {
    /// The service tier.
    pub tier: Option<Identifier>,
    /// The speed.
    pub speed: Option<Identifier>,
    /// Where it ran.
    pub inference_geo: Option<Identifier>,
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
    /// How it was served.
    pub service: Service,
}
