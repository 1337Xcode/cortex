//! Codex's rollout files: one JSON object a line, under `~/.codex/sessions/YYYY/MM/DD/`.
//!
//! A response is written on a `token_usage_record` line: the provider's `response_id`, the line's
//! `timestamp`, and in its `usage` the whole input (`input_tokens`, which counts the input read from
//! the cache and the input written to it), the input read from the cache (`cached_input_tokens`),
//! the input written to it (`cache_write_input_tokens`), the output (`output_tokens`, its reasoning
//! included) and the reasoning (`reasoning_output_tokens`). The line names no model: the latest
//! `turn_context` line before it does, which the reading keeps in [`Earlier`]. A line missing any
//! count is not understood. Codex writes each response once; a subagent writes a rollout of its own.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::format::{Earlier, TranscriptFormat, contains};
use crate::response::{Response, Service, Usage};
use crate::timestamp::utc_millis;

/// The variable that moves Codex's home folder, `~/.codex`, and its rollout files with it.
const HOME_VARIABLE: &str = "CODEX_HOME";

/// Codex's rollout format.
#[derive(Debug, Clone, Copy, Default)]
pub struct Codex;

impl TranscriptFormat for Codex {
    fn folders(&self, home: &Path, variable: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
        // A relative path would depend on the folder Codex started in, which the daemon cannot
        // know.
        let moved = variable(HOME_VARIABLE)
            .map(PathBuf::from)
            .filter(|folder| folder.is_absolute());
        moved
            .into_iter()
            .chain([home.join(".codex")])
            .map(|folder| folder.join("sessions"))
            .collect()
    }

    fn response(&self, line: &[u8], earlier: &mut Earlier) -> Option<Response> {
        // Most lines are neither; only the few that may be are parsed.
        if contains(line, b"\"turn_context\"") {
            if let Ok(Line {
                kind,
                payload: Some(Payload { model, .. }),
                ..
            }) = serde_json::from_slice(line)
                && kind.as_deref() == Some("turn_context")
                && let Some(model) = model.filter(|model| !model.is_empty())
            {
                earlier.model = Some(model);
            }
            return None;
        }
        if !contains(line, b"\"token_usage_record\"") {
            return None;
        }
        let line: Line = serde_json::from_slice(line).ok()?;
        if line.kind.as_deref() != Some("token_usage_record") {
            return None;
        }
        let payload = line.payload?;
        Some(Response {
            message_id: payload.response_id.filter(|id| !id.is_empty())?,
            at_ms: utc_millis(line.timestamp.as_deref()?)?,
            model: earlier.model.clone()?,
            usage: payload.usage?.counts()?,
            service: Service::default(),
        })
    }
}

/// The fields of a rollout line a response or a model is read from; every other field is ignored.
#[derive(Deserialize)]
struct Line {
    /// When it was written.
    timestamp: Option<String>,
    /// What kind of line it is.
    #[serde(rename = "type")]
    kind: Option<String>,
    /// What it holds.
    payload: Option<Payload>,
}

/// What a `turn_context` or a `token_usage_record` line holds.
#[derive(Deserialize)]
struct Payload {
    /// The model of the turn, on a `turn_context` line.
    model: Option<String>,
    /// The provider's id for the response, on a `token_usage_record` line.
    response_id: Option<String>,
    /// The tokens the response took.
    usage: Option<RawUsage>,
}

/// The tokens a response took, as written.
#[derive(Deserialize)]
#[allow(
    clippy::struct_field_names,
    reason = "the fields are named as the rollout names them"
)]
struct RawUsage {
    /// The whole input: read from the cache, written to it, and neither.
    input_tokens: Option<u64>,
    /// Input read from the cache.
    cached_input_tokens: Option<u64>,
    /// Input written to the cache.
    cache_write_input_tokens: Option<u64>,
    /// Output, reasoning included.
    output_tokens: Option<u64>,
    /// Reasoning.
    reasoning_output_tokens: Option<u64>,
}

impl RawUsage {
    /// The counts, when every one is there: the input at the full price is the whole input less
    /// what was read from the cache and written to it.
    fn counts(&self) -> Option<Usage> {
        // INVARIANT: measured-claims (a count the rollout does not give is never taken as zero:
        // the line is not understood)
        let whole = self.input_tokens?;
        let cache_read = self.cached_input_tokens?;
        let cache_write = self.cache_write_input_tokens?;
        Some(Usage {
            input: whole.saturating_sub(cache_read).saturating_sub(cache_write),
            cache_write,
            cache_write_1h: None,
            cache_read,
            output: self.output_tokens?,
            reasoning: Some(self.reasoning_output_tokens?),
        })
    }
}
