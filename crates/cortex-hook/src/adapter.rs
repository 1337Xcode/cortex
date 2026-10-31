//! The agent adapters: one per agent, each turning that agent's hook payloads into normalized
//! events and listing the hooks Cortex registers with it. Adding an agent means one module and one
//! line in [`ADAPTERS`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, Ulid};

use crate::claude::Claude;

/// Every agent Cortex has an adapter for.
const ADAPTERS: &[&dyn Adapter] = &[&Claude];

/// The adapter of the agent named `name` on the command line (`claude`), if there is one.
#[must_use]
pub fn adapter(name: &str) -> Option<&'static dyn Adapter> {
    ADAPTERS
        .iter()
        .copied()
        .find(|adapter| adapter.agent().name() == name)
}

/// What an agent's adapter knows.
pub trait Adapter: Sync {
    /// The agent.
    fn agent(&self) -> Agent;

    /// The hooks Cortex registers with the agent, one per event name.
    fn hooks(&self) -> &'static [HookEntry];

    /// The event a hook call of `event_name` with `payload` becomes, or `None` when Cortex does not
    /// record that event or the payload's working folder is in no repository Cortex serves.
    /// `locate` finds the repository from the payload's working folder; `stamp` names the event.
    ///
    /// # Errors
    ///
    /// The payload is not the JSON the agent sends, or lacks what every event needs (a session).
    fn normalize(
        &self,
        event_name: &str,
        payload: &[u8],
        locate: &dyn Fn(&Path) -> Option<Located>,
        stamp: Stamp,
    ) -> Result<Option<Event>, NormalizeError>;
}

/// One hook Cortex registers with an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookEntry {
    /// The agent's name for the event, also given to `cortex hook --event`.
    pub event_name: &'static str,
    /// The tools the hook is registered for, as the agent matches them, when it filters by tool.
    pub matcher: Option<&'static str>,
    /// The point in the agent's loop it is.
    pub hook: HookKind,
    /// How long the whole hook may take: the deadline for the daemon's answer is this less a
    /// margin, and the agent's own timeout is set at or above it.
    pub hard_timeout: Duration,
}

/// Where a hook call's working folder is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// The working folder, canonical.
    pub cwd: PathBuf,
    /// The repository's root folder, canonical.
    pub root: PathBuf,
    /// The repository's root as the agent spells it: its working folder, less as many folders as
    /// lie between the canonical working folder and the canonical root. The agent writes tool paths
    /// under this spelling.
    pub agent_root: PathBuf,
}

/// What names an event: its id and the time the hook process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// The event's id.
    pub event_id: Ulid,
    /// UTC milliseconds.
    pub ts_ms: i64,
}

/// Why a payload makes no event.
#[derive(Debug, thiserror::Error)]
pub enum NormalizeError {
    /// The payload is not the JSON the agent sends.
    #[error("the payload is not the agent's hook JSON")]
    Json(#[from] serde_json::Error),
    /// The payload names no session, or a session id that is not one line of at most 256 bytes.
    #[error("the payload names no usable session")]
    NoSession,
}
