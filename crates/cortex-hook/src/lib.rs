//! The hook client: what runs when an agent calls `cortex hook`, and the one client of the
//! daemon's protocol, which the CLI uses too.
//!
//! This crate depends on `cortex-core` alone among Cortex's crates, so that a hook starts without
//! loading anything it does not need.

#![warn(clippy::pedantic)]

mod adapter;
mod amp;
mod antigravity;
mod augment;
mod claude;
mod client;
mod cline;
mod codex;
mod copilot;
mod crush;
mod cursor;
mod devin;
mod droid;
mod fail_open;
mod gemini;
mod hermes;
mod junie;
mod kiro;
mod log;
mod openclaw;
mod opencode;
mod openhands;
mod plugin;
mod qwen;
mod relaunch;
mod root;
mod run;
mod shaped;
mod spawn;
mod stdin;
mod stop;
mod trae;

pub use adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions, adapter};
pub use client::{Answer, ClientError, ask, ask_served};
pub use plugin::PluginFile;
pub use relaunch::RELAUNCH_FLAG;
#[cfg(windows)]
pub use relaunch::relaunch_daemon;
pub use root::{locate, locate_marked};
pub use run::run;
pub use spawn::{Started, start_daemon, start_daemon_now};
pub use stop::{Stop, stop_any, stop_if_older};
