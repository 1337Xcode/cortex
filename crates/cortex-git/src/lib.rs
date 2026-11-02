//! A repository's git history as Cortex reads it to learn which files change together: the
//! commits on the first-parent line from HEAD, newest first, each with the files it changed.
//! [`HistorySource`] is what a source of that history gives; [`GixHistory`] reads it in-process,
//! and [`CliHistory`] by running the `git` program, the fallback when `gix` cannot. [`cochange`]
//! turns the commits read into which files change together.

#![warn(clippy::pedantic)]

mod cli_history;
pub mod cochange;
mod git_log;
mod git_program;
mod gix_history;
mod history;
mod renames;

pub use cli_history::CliHistory;
pub use gix_history::GixHistory;
pub use history::{GitError, HistorySource, Mined, MinedCommit, PathChange, Window};
