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
mod facts;
mod failure_map;
mod failure_seeds;
mod features;
mod gate;
mod graph;
mod grounded_failure;
mod hints;
mod innermost;
mod items;
mod keep_list;
mod lexical;
mod narrow_tests;
mod next_file;
mod node_hash;
mod open_warnings;
mod pipeline;
mod primer;
mod prompt;
mod push;
mod read;
mod reasons;
mod recap;
mod redundant;
mod render;
mod score;
mod seeds;
mod session;
mod similar;
mod spans;
mod stop;
mod support;
mod two_stage;
mod uncommitted;
mod warnings;
mod why;
mod why_lines;
mod working_set;

pub use disk::{CheckedFile, Unchecked};
pub use error::RetrieveError;
pub use exploration::{EXPLORATION_WEIGHT, MAX_EXPLORED, exploration};
pub use facts::{FactAt, ListedFile, MAX_ORDER_FACTS, OrderFact, order_facts};
pub use failure_map::{FailureMap, MAX_FAILURE_MAP_TOKENS, MAX_MAPPED_FRAMES, failure_map};
pub use failure_seeds::{FAILURE_AGREED_WEIGHT, FAILURE_ALONE_WEIGHT, FailureMatch, failure_seeds};
pub use features::{Found, GATED_FILES, MASSED_FILES, features_of};
pub use gate::{
    AGREEING_KINDS, Confidence, LEXICAL_BAR, MAX_RAISED_CONFIDENCE, Rule,
    SMALL_MODE_MIN_CONFIDENCE, STRONG_AGREEING_KINDS, STRONG_LEXICAL_BAR, Shut, Verdict,
    confidence_bar, decide,
};
pub use graph::{Graph, GraphError, LinkInput, NodeInput, Standing, Weighing};
pub use grounded_failure::{
    FailureQuery, GroundedFailure, GroundedFrame, MAX_GROUNDED_FRAMES, ground_failure,
};
pub use hints::{HINTED_ABOVE_LINES, HintQuery, MAX_HINT_TOKENS, MAX_HINTED_PARTS, read_hint};
pub use items::{LOOKED_AT_PER_LISTED, Listed, Listing, listed};
pub use keep_list::{MAX_KEEP_LIST_CHARS, keep_list};
pub use lexical::{FileMatch, LexicalMatch, MAX_HITS, lexical};
pub use narrow_tests::{
    MAX_NARROW_HINT_TOKENS, MAX_NARROW_TESTS, NarrowTest, narrow_hint, narrow_tests,
};
pub use next_file::{
    NEXT_FILE_MAJORITY, NEXT_FILE_MIN_WEIGHT, NextCandidate, NextFile, NextQuery, next_file,
};
pub use pipeline::{
    Location, Outcome, Request, Retrieval, SMALL_MODE_MAX_TOKENS, Silence, locate, retrieve,
};
pub use primer::{MAX_PRIMER_LANDMARKS, MAX_PRIMER_RULES, MAX_PRIMER_TOKENS, primer};
pub use prompt::{Mention, PromptTerms};
pub use push::{Activation, L1_TOLERANCE, MAX_PUSHES, Network, SPREAD_THRESHOLD, spread};
pub use read::{GraphSettings, read_graph};
pub use reasons::{ItemEvidence, MAX_REASON_CHARS, Reason, reason_of};
pub use recap::{MAX_RECAP_TOKENS, recap};
pub use redundant::{
    EarlierRun, MAX_NAMED_FAILING, MAX_REDUNDANT_NOTE_TOKENS, MAX_SHOWN_COMMAND_CHARS,
    redundant_note,
};
pub use render::{
    Basis, Item, ItemSpan, Limits, MAX_WORK_ORDER_CHARS, Rendered, WorkOrder, render, render_with,
};
pub use score::{
    INTERESTING_PRIOR, LANDMARK_PRIOR, ScoredFile, named_first, scored_files, with_symbols,
};
pub use seeds::{SeedQuery, Seeds, seeds, shares, summed};
pub use session::{EDITED_WEIGHT, READ_WEIGHT, ReadSpan, SEEDED_READS, SessionMatch, session};
pub use similar::{SIMILAR_BAR, SIMILAR_WEIGHT, SimilarMatch, similar};
pub use spans::{MAX_SPANS_PER_FILE, Outlined, Span, is_in_context, outline_of, spans_of};
pub use stop::{MAX_STOP_REASON_TOKENS, stop_reason};
pub use support::{COACCESS_FLOOR, Evidence, STRUCTURAL_FLOOR, Support};
pub use two_stage::{STAGED_FILES, Staged, two_stage};
pub use uncommitted::{UNCOMMITTED_WEIGHT, UncommittedMatch, uncommitted};
pub use warnings::{
    CallSite, Drift, EditFacts, EditQuery, Linked, MAX_CHECKED_CALLER_FILES, MAX_LISTED_CALL_SITES,
    MAX_LISTED_TESTS, MAX_WARNING_TOKENS, NamedFile, Partner, TaskSoFar, TestFile, Warned, Warning,
    edit_facts, task_so_far, warnings_after_edit,
};
pub use why::{
    MAX_LINKS_PER_KIND, MAX_WHY_SESSIONS, MAX_WHY_TOKENS, MAX_WHY_WARNINGS, Why, WhyAt, why,
};
pub use why_lines::{
    BlamedSpan, CommitText, MAX_REASON_TOKENS, MAX_TEXT_ASKS, ReasonCommit, WhyAsk, WhyHistory,
    WhyLines,
};
pub use working_set::{MAX_RECAPPED_FILES, MAX_RECAPPED_WARNINGS};
