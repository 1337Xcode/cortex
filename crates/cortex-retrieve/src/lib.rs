//! Retrieval: what Cortex hands an agent before it starts searching. From a prompt and the session
//! it comes in, seeds; from the seeds, activation spread over the graph of the repository's code,
//! history and past work; from the activation, the files and spans to show, a gate that stays
//! silent unless the signals agree, and the work order and the session's primer, rendered from the
//! files as they are on disk.
//!
//! So far it holds what a prompt points at ([`PromptTerms`]) and the seeds of a retrieval
//! ([`seeds`]: what its words find, what the session has been doing, earlier work like it and the
//! dark code beside those), the graph activation spreads over ([`Graph`], read from the store by
//! [`read_graph`]), the spreading itself ([`spread`]), the gate's rule ([`decide`]) and the reading
//! of a file that lends lines only when it is the file the index read ([`CheckedFile`]).

#![warn(clippy::pedantic)]

mod by_files;
mod disk;
mod error;
mod exploration;
mod features;
mod gate;
mod graph;
mod items;
mod lexical;
mod node_hash;
mod pipeline;
mod plain;
mod primer;
mod prompt;
mod push;
mod read;
mod reasons;
mod render;
mod score;
mod seeds;
mod session;
mod similar;
mod spans;
mod support;
mod two_stage;
mod uncommitted;
mod warnings;

pub use disk::{CheckedFile, Unchecked};
pub use error::RetrieveError;
pub use exploration::{EXPLORATION_WEIGHT, MAX_EXPLORED, exploration};
pub use features::{Found, GATED_FILES, MASSED_FILES, features_of};
pub use gate::{AGREEING_KINDS, LEXICAL_BAR, Rule, Shut, Verdict, decide};
pub use graph::{Graph, GraphError, LinkInput, NodeInput, Standing, Weighing};
pub use items::{LOOKED_AT_PER_LISTED, Listed, Listing, listed};
pub use lexical::{FileMatch, LexicalMatch, MAX_HITS, lexical};
pub use pipeline::{
    Location, Outcome, Request, Retrieval, SMALL_MODE_MAX_TOKENS, Silence, locate, retrieve,
};
pub use primer::{MAX_PRIMER_LANDMARKS, MAX_PRIMER_TOKENS, primer};
pub use prompt::{Mention, PromptTerms};
pub use push::{Activation, L1_TOLERANCE, MAX_PUSHES, Network, SPREAD_THRESHOLD, spread};
pub use read::{GraphSettings, read_graph};
pub use reasons::{ItemEvidence, MAX_REASON_CHARS, Reason, reason_of};
pub use render::{
    Basis, Item, ItemSpan, Limits, MAX_WORK_ORDER_CHARS, Rendered, WorkOrder, render,
};
pub use score::{
    INTERESTING_PRIOR, LANDMARK_PRIOR, ScoredFile, named_first, scored_files, with_symbols,
};
pub use seeds::{SeedQuery, Seeds, seeds, shares, summed};
pub use session::{EDITED_WEIGHT, READ_WEIGHT, ReadSpan, SEEDED_READS, SessionMatch, session};
pub use similar::{SIMILAR_BAR, SIMILAR_WEIGHT, SimilarMatch, similar};
pub use spans::{MAX_SPANS_PER_FILE, Outlined, Span, is_in_context, outline_of, spans_of};
pub use support::{COACCESS_FLOOR, Evidence, STRUCTURAL_FLOOR, Support};
pub use two_stage::{STAGED_FILES, Staged, two_stage};
pub use uncommitted::{UNCOMMITTED_WEIGHT, UncommittedMatch, uncommitted};
pub use warnings::{
    CallSite, Drift, EditFacts, Linked, MAX_LISTED_CALL_SITES, MAX_LISTED_TESTS,
    MAX_WARNING_TOKENS, MIN_LINK_TASKS, MIN_LINK_WEIGHT, NamedFile, Partner, TaskSoFar, TestFile,
    Warned, Warning, warnings_after_edit,
};
