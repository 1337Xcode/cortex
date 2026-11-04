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

mod disk;
mod error;
mod exploration;
mod features;
mod gate;
mod graph;
mod lexical;
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

pub use disk::{CheckedFile, Unchecked};
pub use error::RetrieveError;
pub use exploration::{EXPLORATION_WEIGHT, MAX_EXPLORED, exploration};
pub use features::{Found, GATED_FILES, MASSED_FILES, features_of};
pub use gate::{AGREEING_KINDS, Features, LEXICAL_BAR, Rule, Shut, Verdict, decide};
pub use graph::{Graph, GraphError, LinkInput, NodeInput, Standing, Weighing};
pub use lexical::{FileMatch, LexicalMatch, MAX_HITS, lexical};
pub use prompt::{Mention, PromptTerms};
pub use push::{Activation, L1_TOLERANCE, MAX_PUSHES, SPREAD_THRESHOLD, spread};
pub use read::{GraphSettings, read_graph};
pub use reasons::{ItemEvidence, MAX_REASON_CHARS, Reason, reason_of};
pub use render::{
    Basis, Item, ItemSpan, Limits, MAX_WORK_ORDER_CHARS, Rendered, WorkOrder, render,
};
pub use score::{INTERESTING_PRIOR, LANDMARK_PRIOR, ScoredFile, scored_files};
pub use seeds::{SeedQuery, Seeds, seeds, shares, summed};
pub use session::{EDITED_WEIGHT, READ_WEIGHT, ReadSpan, SEEDED_READS, SessionMatch, session};
pub use similar::{SIMILAR_BAR, SIMILAR_WEIGHT, SimilarMatch, similar};
pub use spans::{MAX_SPANS_PER_FILE, Outlined, Span, is_in_context, outline_of, spans_of};
pub use support::{COACCESS_FLOOR, Evidence, STRUCTURAL_FLOOR, Support};
