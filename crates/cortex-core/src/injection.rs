//! What an injection is made of, as each crate that handles one sees it: the numbers the gate
//! weighs, and the kinds of reasons a file is listed for. The store keeps both with each injection,
//! and the calibrated model learns from them.

use serde::{Deserialize, Serialize};

use crate::named::named_kind;

/// What the gate weighs, from one retrieval. The cold-start rule reads the top lexical score, the
/// kinds agreeing and the session's continuity; every feature is kept with the retrieval, for the
/// model calibrated on the repository's own injections once there are enough of them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GateFeatures {
    /// How well the best-matching file matches the prompt's words, from 0 to 1.
    pub top_lexical: f64,
    /// The share of all activation that the top five files hold, from 0 to 1.
    pub top_mass: f64,
    /// The most independent kinds of evidence that support any one of the top three files.
    pub agreeing_kinds: u8,
    /// How many independent kinds of evidence support at least one of the top three files.
    pub supporting_kinds: u8,
    /// How alike the prompt is to the most alike cluster of earlier tasks, from 0 to 1.
    pub similar_task: f64,
    /// The share of the repository's latest delivered injections that were used, when there are
    /// any.
    pub recent_precision: Option<f64>,
    /// Whether the session is plainly continuing its own work: a top file was edited earlier in it,
    /// or is changed and not committed, and the prompt's words match that file.
    pub is_continuing: bool,
}

named_kind! {
    /// Why a file was listed, as a kind: what the reason says without the words it says it in,
    /// which hold the prompt's words or another file's name.
    ReasonKind {
        /// The prompt names it, or a symbol in it, exactly.
        Named => "named",
        /// The session edited it.
        EditedHere => "edited_here",
        /// It holds words of the prompt.
        Matches => "matches",
        /// It changes in the same commits as another file the prompt points at.
        ChangesWith => "changes_with",
        /// Code in it refers to another file the prompt points at.
        RefersTo => "refers_to",
        /// Code in another file the prompt points at refers to it.
        ReferredToBy => "referred_to_by",
        /// Earlier tasks like this one changed it.
        LikeEarlierWork => "like_earlier_work",
        /// Agents worked on it together with another file the prompt points at.
        WorkedOnWith => "worked_on_with",
        /// The session read it.
        ReadHere => "read_here",
    }
}
