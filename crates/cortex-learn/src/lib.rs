//! What Cortex learns from agents' work. For now, the record of it: [`Recorder`] turns each
//! normalized hook event into the session, task and event rows the store keeps and keeps each open
//! task's [`TaskState`], what it did, which it hands out when the task ends. It holds each edit
//! only the file watcher saw ([`HeldEdit`]) until it can tell whether a hook reported it and whose
//! it is. [`Commands`] tells what a shell command line ran: the checks of the runners it knows and
//! git restores.

#![warn(clippy::pedantic)]

mod activity;
mod commands;
mod record;
mod task_state;
mod watcher_edits;

pub use commands::{CONFIGURED, Check, Commands, PatternError, Ran, Restore};
pub use record::Recorder;
pub use task_state::{
    FileNode, MAX_TASK_HIT_BYTES, MAX_TASK_ROWS, MAX_TASK_STATES, RunCheck, Step, TaskState,
};
pub use watcher_edits::{ATTRIBUTION_WINDOW_MS, HOLD_MS, HeldEdit, WatcherEdit};
