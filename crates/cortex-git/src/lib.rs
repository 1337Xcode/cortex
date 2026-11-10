//! A repository's git history as Cortex reads it to learn which files change together: the
//! commits on the first-parent line from HEAD, newest first, each with the files it changed.
//! [`HistorySource`] is what a source of that history gives; [`GixHistory`] reads it in-process,
//! and [`CliHistory`] by running the `git` program, the fallback when `gix` cannot. [`cochange`]
//! turns the commits read into which files change together, and [`mine`] decides what to read
//! and writes what it found to the store. [`worktree`] reads which files the work tree holds
//! changed and not committed. [`blame::Blamer`] finds which commit last changed a span of lines,
//! and [`card::CommitReader`] and [`card::read_card`] read a commit's message as a change card:
//! numbers and a kind, with the two lines of the message worth showing held by the caller in
//! memory only. Both open a repository with [`OpenError`] saying why they may not.

#![warn(clippy::pedantic)]

pub mod blame;
pub mod card;
mod cli_history;
pub mod cochange;
mod git_log;
mod git_program;
mod gix_history;
mod history;
pub mod mine;
mod object_id;
mod renames;
pub mod worktree;

pub use cli_history::CliHistory;
pub use git_program::OpenError;
pub use gix_history::GixHistory;
pub use history::{
    GitError, HistorySource, Mined, MinedCommit, PathChange, Window, has_repository,
};
