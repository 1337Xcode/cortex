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
//!
//! [`index_repository`] does all of it for a whole repository and writes the result to the store:
//! every file the walk finds, with what it defines and uses, in path order; each use tied to the
//! definition its name settles; each file joined to the files its imports name; and a graph node
//! for each folder that holds files, joined to them.

#![warn(clippy::pedantic)]

mod bindings;
mod deadline;
mod detect;
mod discover;
mod extract;
mod file_edges;
mod file_imports;
mod folders;
mod gitattributes;
mod grammar;
mod hashes;
mod head;
mod imports;
mod languages;
mod lines;
mod locals;
mod name_text;
mod path_rules;
mod pattern_rules;
mod patterns;
mod pipeline;
mod references;
mod resolve;
mod rules;
mod safe_read;
mod specifiers;
mod symbol_edges;
mod symbols;
mod tags;
#[cfg(test)]
mod test_support;
mod test_targets;

pub use discover::{
    DiscoverError, DiscoverOptions, DiscoveredFile, Discovery, MAX_FOLDER_DEPTH, Problem,
    ProblemKind, discover,
};
pub use extract::{ExtractError, Extraction, Extractor, LeftOut};
pub use grammar::{GrammarError, Grammars};
pub use pipeline::{FileProblem, IndexError, IndexOptions, IndexReport, index_repository};
pub use references::Qualifier;
pub use rules::MAX_REPORTED_LINES;
pub use specifiers::{Import, Imported};
