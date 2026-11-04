//! Retrieval: what Cortex hands an agent before it starts searching. From a prompt and the session
//! it comes in, seeds; from the seeds, activation spread over the graph of the repository's code,
//! history and past work; from the activation, the files and spans to show, a gate that stays
//! silent unless the signals agree, and the work order and the session's primer, rendered from the
//! files as they are on disk.
//!
//! So far it holds the graph activation spreads over ([`Graph`]), the spreading itself
//! ([`spread`]), the gate's rule ([`decide`]) and the reading of a file that lends lines only when
//! it is the file the index read ([`CheckedFile`]).

#![warn(clippy::pedantic)]

mod disk;
mod gate;
mod graph;
mod push;

pub use disk::{CheckedFile, Unchecked};
pub use gate::{AGREEING_KINDS, Features, LEXICAL_BAR, Rule, Shut, Verdict, decide};
pub use graph::{Graph, GraphError, LinkInput, NodeInput, Weighing};
pub use push::{Activation, L1_TOLERANCE, MAX_PUSHES, SPREAD_THRESHOLD, spread};
