//! The indexer: which files a repository has, what they are, and what they define and use.
//!
//! [`discover`] walks a repository the way git sees it, minus what the user asked to leave out,
//! and returns every file with its language and the flags its path, its `.gitattributes` and its
//! first bytes earn: may hold secrets (never opened), too large (never read), binary, generated or
//! vendored.
//!
//! [`Extractor`] parses one file with its language's grammar and reads its definitions, the names
//! it uses and the modules it imports. [`Grammars`] holds every language's grammar and queries,
//! compiled once and shared between extractors.

#![warn(clippy::pedantic)]

mod deadline;
mod detect;
mod discover;
mod extract;
mod gitattributes;
mod grammar;
mod hashes;
mod head;
mod languages;
mod lines;
mod path_rules;
mod patterns;
mod references;
mod rules;
mod safe_read;
mod specifiers;
mod symbols;
mod tags;
#[cfg(test)]
mod test_support;

pub use discover::{
    DiscoverError, DiscoverOptions, DiscoveredFile, Discovery, MAX_FOLDER_DEPTH, Problem,
    ProblemKind, discover,
};
pub use extract::{ExtractError, Extraction, Extractor};
pub use grammar::{GrammarError, Grammars};
pub use rules::MAX_REPORTED_LINES;
pub use specifiers::{Import, Imported};
