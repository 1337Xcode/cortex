//! What Cortex learns from agents' work. For now, the record of it: [`Recorder`] turns each
//! normalized hook event into the session, task and event rows the store keeps and keeps each open
//! task's [`TaskState`], what it did, which it hands out when the task ends. It holds each edit
//! only the file watcher saw ([`HeldEdit`]) until it can tell whether a hook reported it and whose
//! it is. [`Commands`] tells what a shell command line ran: the checks of the runners it knows and
//! git restores. [`judge`] tells what became of a task that has ended, from its state, and
//! [`interest_of`] what its work adds to the interest of each node it touched, and
//! [`links_of`] what it does to the links between them. [`label_injections`] says of each
//! injection the task's prompt got whether the task used what it gave. [`episode_of`] is one
//! record of what the task did, and [`archetypes_of`] clusters those records into the shapes of
//! task a repository has. [`Learner`] does all of that as each task ends, and looks again at the
//! task before it in the same session, which only the next task's end can show was put back.

#![warn(clippy::pedantic)]

mod activity;
mod archetype;
mod commands;
mod decay;
mod episode;
mod given;
mod interest;
mod labels;
mod links;
mod outcome;
mod places;
mod record;
mod task_end;
mod task_state;
mod watcher_edits;

pub use archetype::{MAX_CLUSTERED_TASKS, archetypes_of};
pub use commands::{CONFIGURED, Check, Commands, PatternError, Ran, Restore, Told};
pub use decay::decayed;
pub use episode::episode_of;
pub use interest::{EarnedInterest, add_interest, interest_of, tier_now};
pub use labels::{LABELLED_FILES, label_injections};
pub use links::{EarnedLinks, Linking, MAX_LINK_WEIGHT, add_links, links_of};
pub use outcome::{FileNow, Judgement, judge, reverted_after};
pub use record::Recorder;
pub use task_end::{Learned, Learner, MAX_SESSIONS_KEPT, cluster_tasks};
pub use task_state::{
    FileNode, MAX_TASK_HIT_BYTES, MAX_TASK_ROWS, MAX_TASK_STATES, RunCheck, Step, TaskEnd,
    TaskState,
};
pub use watcher_edits::{ATTRIBUTION_WINDOW_MS, HOLD_MS, HeldEdit, WatcherEdit};
