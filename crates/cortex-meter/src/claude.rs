//! Claude Code's transcripts: one JSON object a line, under `~/.claude/projects/`.
//!
//! A response is written on `assistant` lines whose message carries `usage`: the message's `id`,
//! the line's `timestamp`, the message's `model`, and in `usage`
//! the uncached input (`input_tokens`), the cache writes (`cache_creation_input_tokens`, or the sum
//! of the five-minute and one-hour writes of `cache_creation` when it gives them), the cache reads
//! (`cache_read_input_tokens`), the output (`output_tokens`) and its reasoning
//! (`output_tokens_details.thinking_tokens`). A line missing one of the four counts is not
//! understood. Claude Code writes one line per content block of a response, each with the same ids
//! and usage. Other lines' messages carry no usage; a user line that returns a subagent's result
//! carries that subagent's last usage beside its message, which is the subagent's own transcript's
//! to report.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cortex_core::Identifier;
use serde::Deserialize;

use crate::format::{Earlier, TranscriptFormat, contains};
use crate::response::{Response, Service, Usage};
use crate::timestamp::utc_millis;

/// The variable that moves Claude Code's configuration folder, `~/.claude`, and its session
/// history with it.
const CONFIG_FOLDER_VARIABLE: &str = "CLAUDE_CONFIG_DIR";

/// Claude Code's transcript format.
#[derive(Debug, Clone, Copy, Default)]
pub struct Claude;

impl TranscriptFormat for Claude {
    fn folders(&self, home: &Path, variable: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
        // Claude Code's documentation asks for an absolute path here; a relative one would depend
        // on the folder Claude Code started in, which the daemon cannot know.
        let moved = variable(CONFIG_FOLDER_VARIABLE)
            .map(PathBuf::from)
            .filter(|folder| folder.is_absolute());
        moved
            .into_iter()
            .chain([home.join(".claude")])
            .map(|folder| folder.join("projects"))
            .collect()
    }

    fn response(&self, line: &[u8], _: &mut Earlier) -> Option<Response> {
        // Most lines hold no usage at all; only the few that do are parsed.
        if !contains(line, b"\"usage\"") {
            return None;
        }
        let line: Line = serde_json::from_slice(line).ok()?;
        let message = line.message?;
        let usage = message.usage?;
        Some(Response {
            message_id: message.id.filter(|id| !id.is_empty())?,
            at_ms: utc_millis(line.timestamp.as_deref()?)?,
            model: message.model.filter(|model| !model.is_empty())?,
            usage: usage.counts()?,
            service: usage.service(),
        })
    }

    fn records_compaction(&self) -> bool {
        // The call that summarizes a session at a compaction writes no line: Claude Code's own
        // cost total, read after a `/compact`, exceeds the transcript's sum by that one call.
        false
    }
}

/// The fields of a transcript line a response is read from; every other field is ignored.
#[derive(Deserialize)]
struct Line {
    /// When it was written.
    timestamp: Option<String>,
    /// The message.
    message: Option<Message>,
}

/// A line's message.
#[derive(Deserialize)]
struct Message {
    /// The provider's id for it.
    id: Option<String>,
    /// The model that wrote it.
    model: Option<String>,
    /// The tokens it took.
    usage: Option<RawUsage>,
}

/// The tokens a message took, as written.
#[derive(Deserialize)]
struct RawUsage {
    /// Uncached input.
    input_tokens: Option<u64>,
    /// Input written to the cache, both lifetimes together.
    cache_creation_input_tokens: Option<u64>,
    /// Input read from the cache.
    cache_read_input_tokens: Option<u64>,
    /// Output.
    output_tokens: Option<u64>,
    /// What the output was spent on.
    output_tokens_details: Option<OutputDetails>,
    /// The cache writes by lifetime.
    cache_creation: Option<CacheCreation>,
    /// The service tier, as written: a value of another type than text must not make the line
    /// unreadable and lose its counts.
    service_tier: Option<serde_json::Value>,
    /// The speed, as written.
    speed: Option<serde_json::Value>,
    /// Where it ran, as written.
    inference_geo: Option<serde_json::Value>,
}

/// What the output was spent on.
#[derive(Deserialize)]
struct OutputDetails {
    /// Reasoning.
    thinking_tokens: Option<u64>,
}

/// The cache writes by lifetime.
#[derive(Deserialize)]
struct CacheCreation {
    /// Written for five minutes.
    ephemeral_5m_input_tokens: Option<u64>,
    /// Written for an hour.
    ephemeral_1h_input_tokens: Option<u64>,
}

impl RawUsage {
    /// How the response was served; a value that is not text, or text that is no name, too long or
    /// holding control characters, is not known.
    fn service(&self) -> Service {
        let name = |value: &Option<serde_json::Value>| {
            value
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .and_then(|text| Identifier::new(text).ok())
        };
        Service {
            tier: name(&self.service_tier),
            speed: name(&self.speed),
            inference_geo: name(&self.inference_geo),
        }
    }

    /// The counts, when the four that every response reports are there; the writes by lifetime,
    /// when both are given, are the writes. With one of them missing, how the writes split is not
    /// known, and the flat count is taken.
    fn counts(&self) -> Option<Usage> {
        let split = self.cache_creation.as_ref().and_then(|split| {
            split
                .ephemeral_5m_input_tokens
                .zip(split.ephemeral_1h_input_tokens)
        });
        // INVARIANT: measured-claims (a count the transcript does not give is never taken as zero:
        // the line is not understood)
        let written = self.cache_creation_input_tokens?;
        let cache_write = match split {
            Some((five_minutes, one_hour)) => five_minutes.saturating_add(one_hour),
            None => written,
        };
        Some(Usage {
            input: self.input_tokens?,
            cache_write,
            cache_write_1h: split.map(|(_, one_hour)| one_hour),
            cache_read: self.cache_read_input_tokens?,
            output: self.output_tokens?,
            reasoning: self
                .output_tokens_details
                .as_ref()
                .and_then(|details| details.thinking_tokens),
        })
    }
}
