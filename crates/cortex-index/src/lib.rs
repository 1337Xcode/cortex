//! The indexer: which files a repository has and what they are.
//!
//! [`discover`] walks a repository the way git sees it, minus what the user asked to leave out,
//! and returns every file with its language and the flags its path, its `.gitattributes` and its
//! first bytes earn: may hold secrets (never opened), too large (never read), binary, generated or
//! vendored.

#![warn(clippy::pedantic)]

mod detect;
mod discover;
mod gitattributes;
mod head;
mod languages;
mod path_rules;
mod patterns;
mod rules;
mod safe_read;

pub use discover::{
    DiscoverError, DiscoverOptions, DiscoveredFile, Discovery, MAX_FOLDER_DEPTH, Problem,
    ProblemKind, discover,
};
pub use rules::MAX_REPORTED_LINES;
