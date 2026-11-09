//! The fact requests a `cortex` command sends a daemon and what it answers: a fact the user notes,
//! the list of facts, and the user's word on one of them (protocol 6.0 added them).

use serde::{Deserialize, Serialize};

use crate::named::named_kind;
use crate::{FactKind, FactStatus};

/// A fact the user notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The rule, in words.
    pub text: String,
    /// What kind of rule it is.
    pub kind: FactKind,
    /// What it is about: each a repository path with `/` separators, optionally followed by `:`
    /// and the name of a symbol defined in that file.
    pub at: Vec<String>,
}

/// The user's word on the facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub enum FactsAsk {
    /// The current facts: active, candidate and suspect, and retired too when `all` is set.
    List {
        /// Whether retired facts are listed too.
        all: bool,
    },
    /// Fact `id` is true of the code as it is now: active, its anchors stamped anew.
    Confirm {
        /// The fact.
        id: i64,
    },
    /// Fact `id` no longer holds: retired, never injected again.
    Retire {
        /// The fact.
        id: i64,
    },
    /// Fact `id` is corrected by `text`: a new fact on the same anchors supersedes it.
    Edit {
        /// The fact.
        id: i64,
        /// The corrected rule.
        text: String,
    },
}

named_kind! {
    /// What became of a fact noted.
    NoteOutcome {
        /// A new fact.
        Added => "added",
        /// A fact already known, said again, as it was.
        Merged => "merged",
        /// A fact already known, said again, now active.
        Activated => "activated",
    }
}

/// What a daemon answers a note or the user's word on the facts with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum FactsAnswer {
    /// The facts asked for, oldest first.
    Listed {
        /// The facts.
        facts: Vec<FactLine>,
    },
    /// What became of a note.
    Noted {
        /// The fact the note is.
        id: i64,
        /// Whether it is new, known, or known and now active.
        outcome: NoteOutcome,
    },
    /// Fact `id` is confirmed.
    Confirmed {
        /// The fact.
        id: i64,
    },
    /// Fact `id` is retired.
    Retired {
        /// The fact.
        id: i64,
    },
    /// Fact `id` is corrected by fact `by`.
    Edited {
        /// The fact corrected.
        id: i64,
        /// The fact correcting it.
        by: i64,
    },
}

/// One fact as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactLine {
    /// Its id.
    pub id: i64,
    /// Where it stands.
    pub status: FactStatus,
    /// What kind of rule it is.
    pub kind: FactKind,
    /// The rule.
    pub text: String,
    /// What it is about, as a note names it: a path, or a path, `:` and a symbol's qualified name.
    pub anchors: Vec<String>,
    /// How many times it was injected.
    pub uses: u64,
}
