//! A repository's git history as Cortex reads it to learn which files change together: the
//! commits on the first-parent line from HEAD, newest first, each with the files it changed.
//! [`HistorySource`] is what a source of that history gives; [`GixHistory`] reads it in-process.

#![warn(clippy::pedantic)]

mod gix_history;
mod history;
mod renames;

pub use gix_history::GixHistory;
pub use history::{GitError, HistorySource, Mined, MinedCommit, PathChange, Window};
