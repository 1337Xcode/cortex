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
//! [`assert_fact`] adds a project fact or recognizes one said again, [`review_facts`] keeps
//! facts' statuses true to the code they are anchored to, and [`confirm_fact`], [`retire_fact`]
//! and [`edit_fact`] carry the user's word on one. [`calibrate`] fits the confidence model the gate
//! uses once there are enough labelled injections, and [`model_in_use`] reads the one kept.

#![warn(clippy::pedantic)]

mod activity;
mod archetype;
mod calibration;
mod commands;
mod decay;
mod episode;
mod fact_word;
mod facts;
mod failures;
mod given;
mod handoff;
mod interest;
mod labels;
mod links;
mod outcome;
mod places;
mod record;
mod self_check;
mod task_end;
mod task_state;
mod transitions;
mod warnings;
mod watcher_edits;

pub use archetype::{MAX_CLUSTERED_TASKS, archetypes_of};
pub use calibration::{
    AUC_TOLERANCE, CALIBRATION_WINDOW, CHANCE_AUC, Calibrated, ConfidenceModel, Fitted,
    HELD_OUT_EVERY, KEPT_MODELS, L2_PENALTY, MIN_LABELLED, MODEL_INPUTS, ModelInUse,
    REFIT_AFTER_MS, Refusal, auc, calibrate, calibrated, model_in_use, model_inputs,
};
pub use commands::{
    CONFIGURED, Check, Cleaned, Commands, PatternError, REDACTED, Ran, Restore, Told, clean_command,
};
pub use decay::{FADED_OUT_BELOW, decayed, has_faded_out};
pub use episode::episode_of;
pub use fact_word::{confirm_fact, edit_fact, retire_fact};
pub use facts::{
    Asserted, Assertion, FactError, MAX_GLOBAL_FACTS, MERGES_AT, Reviewed,
    SUSPECT_RETIRES_AFTER_MS, assert_fact, review_facts,
};
pub use failures::{
    Failures, Frame, LineFailure, MAX_FAILED_TESTS, MAX_FRAMES, MAX_NAME_CHARS, parse_failures,
    read_line_failure,
};
pub use interest::{EarnedInterest, add_interest, interest_of, tier_now};
pub use labels::{LABELLED_FILES, count_next_file_use, label_injections};
pub use links::{EarnedLinks, Linking, MAX_LINK_WEIGHT, add_links, links_of};
pub use outcome::{FileNow, Judgement, judge, reverted_after};
pub use record::Recorder;
pub use self_check::{
    MAX_RAISE_STEPS, RAISE_STEP, SELF_CHECK_FLOOR, SELF_CHECK_WINDOW, check_self, raise_of,
    recent_precision, self_checked,
};
pub use task_end::{Learned, Learner, MAX_SESSIONS_KEPT, cluster_tasks};
pub use task_state::{
    FileNode, MAX_TASK_HIT_BYTES, MAX_TASK_ROWS, MAX_TASK_STATES, RunCheck, Step, TaskEnd,
    TaskState,
};
pub use transitions::{MAX_WALKED_READS, add_transitions, take_back_transitions, transitions_of};
pub use warnings::{resolve_warnings, resolves};
pub use watcher_edits::{ATTRIBUTION_WINDOW_MS, HOLD_MS, HeldEdit, WatcherEdit};
