//! The claims requests a `cortex claims` command sends a daemon and what it answers: the claims
//! holding now, files claimed for the session at work, and claims released (protocol 9.0 added
//! them). A claim is advisory: it locks nothing, it only tells other sessions who works where.

use serde::{Deserialize, Serialize};

/// What a `cortex claims` command asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub enum ClaimsAsk {
    /// The claims holding now.
    List,
    /// Claim files for the session at work.
    Add {
        /// The files, each a repository path with `/` separators.
        at: Vec<String>,
    },
    /// Release the claims on files, whoever holds them; every claim when `at` is empty.
    Release {
        /// The files, each a repository path with `/` separators.
        at: Vec<String>,
    },
}

/// What a daemon answers a claims request with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum ClaimsAnswer {
    /// The claims holding now, by file, then by session.
    Listed {
        /// The claims.
        claims: Vec<ClaimLine>,
    },
    /// The files are claimed for the session at work.
    Added {
        /// Its agent, as people write it.
        agent: String,
        /// The session.
        session: String,
        /// How many files it claimed.
        files: u32,
    },
    /// Claims were released.
    Released {
        /// How many.
        count: u64,
    },
}

/// One claim as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimLine {
    /// The file claimed, its repository path.
    pub path: String,
    /// The agent of the session holding it, as people write it.
    pub agent: String,
    /// The session holding it.
    pub session: String,
    /// When it was made, in UTC milliseconds.
    pub since_ms: i64,
}
