//! The life of a fact. A fact asserted on the same anchors as a current one, in words alike enough,
//! is that fact said again, not a new one: a candidate turns active when the user says it, or an
//! agent says it in a second task; a suspect one turns active again when the user or an agent says
//! it against the code as it is now, its anchors stamped with today's hashes. Otherwise the fact is
//! added, active when the user wrote it and a candidate when an agent or an import did. A fact about
//! the whole repository is anchored to its root alone, and the repository holds at most
//! [`MAX_GLOBAL_FACTS`] of them that are not retired.
//!
//! [`review_facts`] keeps statuses true to the code: a candidate or active fact with an anchor whose
//! code changed, or is gone, turns suspect; a fact suspect for 14 days retires. The user's own word
//! on a fact (confirm, retire, correct) is in `fact_word`.

use std::collections::BTreeSet;

use cortex_core::{FactKind, FactSource, FactStatus, RepoPath, Ulid, identifier_tokens, jaccard};
use cortex_store::{FactAnchor, FactId, FactRecord, FactRow, NodeId, Queries, StoreError, WriteTx};

/// How alike two facts' words must be for one to be the other said again: 80% of the words they
/// hold between them shared, as Jaccard measures it, after the case and the marks between words
/// are set aside.
pub const MERGES_AT: f64 = 0.8;

/// Most facts about the whole repository it holds that are not retired: a few rules every task
/// should know, which every session's start carries. A new one past this is refused until one is
/// retired.
pub const MAX_GLOBAL_FACTS: usize = 10;

/// How long a fact stays suspect before it retires on its own: 14 days, in milliseconds. Long
/// enough for the user to look at what changed and confirm the rule or retire it.
pub const SUSPECT_RETIRES_AFTER_MS: i64 = 14 * 24 * 60 * 60 * 1_000;

/// A fact as someone states it.
#[derive(Debug, Clone, Copy)]
pub struct Assertion<'a> {
    /// The rule, in words.
    pub text: &'a str,
    /// What kind of rule it is.
    pub kind: FactKind,
    /// Who states it.
    pub source: FactSource,
    /// The task it is stated in: none for the user's and for an import.
    pub task: Option<Ulid>,
    /// What it is about: files and symbols, each once.
    pub nodes: &'a [NodeId],
}

/// What became of an assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asserted {
    /// A new fact, in its source's first status.
    Added(FactId),
    /// A fact said again that stays as it was.
    Merged(FactId),
    /// A fact said again that is now active, its anchors stamped with the hashes they have now.
    Activated(FactId),
}

/// Why a change to the facts was refused.
#[derive(Debug, thiserror::Error)]
pub enum FactError {
    /// A node has no hash to keep: not a file whose contents are read, nor a symbol, or gone.
    #[error("node {} cannot anchor a fact: only a read file or a symbol can", node.get())]
    Unanchorable {
        /// The node.
        node: NodeId,
    },
    /// No fact has the id.
    #[error("there is no fact {}", fact.get())]
    Unknown {
        /// The fact asked for.
        fact: FactId,
    },
    /// A fact about the whole repository names something else too: it is anchored to the root
    /// alone.
    #[error("a fact about the whole repository is anchored to it alone")]
    GlobalWithOthers,
    /// The repository holds as many facts about the whole of it as it may.
    #[error(
        "the repository holds {} facts about the whole of it already: retire one first",
        MAX_GLOBAL_FACTS
    )]
    TooManyGlobal,
    /// The fact was corrected by another, which is the one to change.
    #[error("fact {} was corrected by fact {}", fact.get(), by.get())]
    Superseded {
        /// The fact asked for.
        fact: FactId,
        /// The fact correcting it.
        by: FactId,
    },
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// What a review of the facts changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reviewed {
    /// The facts that turned suspect, in id order.
    pub suspected: Vec<FactId>,
    /// The facts that retired, in id order.
    pub retired: Vec<FactId>,
}

/// States `assertion` at `now_ms` in `transaction`: the fact it says again, made active when the
/// assertion confirms it, or a new fact.
///
/// # Errors
///
/// [`FactError::Unanchorable`] for a node that cannot anchor a fact,
/// [`FactError::GlobalWithOthers`] for the root named with anything else,
/// [`FactError::TooManyGlobal`] for a new fact about the whole repository past
/// [`MAX_GLOBAL_FACTS`], and the store's errors (among them text that is too long and no node).
pub fn assert_fact(
    transaction: &mut WriteTx<'_>,
    assertion: &Assertion<'_>,
    now_ms: i64,
) -> Result<Asserted, FactError> {
    let nodes: BTreeSet<NodeId> = assertion.nodes.iter().copied().collect();
    let listed: Vec<NodeId> = nodes.iter().copied().collect();
    let anchors = anchors_now(transaction.queries(), &listed)?;
    let root = transaction.queries().folder_node_at(&RepoPath::root())?;
    let is_global = root.is_some_and(|root| nodes.contains(&root));
    if is_global && nodes.len() > 1 {
        return Err(FactError::GlobalWithOthers);
    }
    let Some(said) = said_before(transaction.queries(), assertion.text, &nodes)? else {
        if is_global && global_facts(transaction.queries(), &listed)? >= MAX_GLOBAL_FACTS {
            return Err(FactError::TooManyGlobal);
        }
        let id = transaction.add_fact(&FactRecord {
            text: assertion.text.to_owned(),
            kind: assertion.kind,
            source: assertion.source,
            status: assertion.source.first_status(),
            created_at_ms: now_ms,
            task: assertion.task,
            anchors,
        })?;
        return Ok(Asserted::Added(id));
    };
    if !confirms(&said, assertion) {
        if let Some(task) = assertion.task {
            transaction.note_fact_task(said.id, task)?;
        }
        return Ok(Asserted::Merged(said.id));
    }
    transaction.restamp_fact(said.id, &anchors)?;
    transaction.set_fact_status(said.id, FactStatus::Active, now_ms)?;
    Ok(Asserted::Activated(said.id))
}

/// Makes suspect, at `now_ms` in `transaction`, each candidate or active fact with an anchor whose
/// code changed or is gone, and retires each fact suspect for [`SUSPECT_RETIRES_AFTER_MS`] or
/// longer.
///
/// # Errors
///
/// The store's errors.
pub fn review_facts(transaction: &mut WriteTx<'_>, now_ms: i64) -> Result<Reviewed, StoreError> {
    let suspected = transaction.queries().facts_with_changed_anchors()?;
    for id in &suspected {
        transaction.set_fact_status(*id, FactStatus::Suspect, now_ms)?;
    }
    let due = now_ms.saturating_sub(SUSPECT_RETIRES_AFTER_MS);
    let retired = transaction
        .queries()
        .facts_with_status_since(FactStatus::Suspect, due)?;
    for id in &retired {
        transaction.set_fact_status(*id, FactStatus::Retired, now_ms)?;
    }
    Ok(Reviewed { suspected, retired })
}

/// The current fact, not retired, anchored to exactly `nodes`, whose words are at least
/// [`MERGES_AT`] alike to `text`'s: the most alike, the oldest of equals; none when no fact is.
fn said_before(
    queries: Queries<'_>,
    text: &str,
    nodes: &BTreeSet<NodeId>,
) -> Result<Option<FactRow>, StoreError> {
    let words = words_of(text);
    let listed: Vec<NodeId> = nodes.iter().copied().collect();
    let mut best: Option<(f64, FactRow)> = None;
    for id in queries.facts_anchored_to(&listed)? {
        let Some(row) = queries.fact(id)? else {
            continue;
        };
        let anchored: BTreeSet<NodeId> = row
            .record
            .anchors
            .iter()
            .map(|anchor| anchor.node)
            .collect();
        if row.record.status == FactStatus::Retired || anchored != *nodes {
            continue;
        }
        let likeness = jaccard(&words, &words_of(&row.record.text));
        if likeness >= MERGES_AT && best.as_ref().is_none_or(|(top, _)| likeness > *top) {
            best = Some((likeness, row));
        }
    }
    Ok(best.map(|(_, row)| row))
}

/// Whether `assertion`, saying `said` again, makes it active: a candidate the user states, or one
/// an agent states in a task other than the one that first asserted it; a suspect fact that was
/// active, the user or an agent states against the code as it is now; a suspect fact that was a
/// candidate (or of which that is not known), as a candidate is confirmed, so that an agent cannot
/// confirm its own rule by changing the code it is about. An import confirms nothing.
fn confirms(said: &FactRow, assertion: &Assertion<'_>) -> bool {
    match (said.record.status, said.before_suspect) {
        (FactStatus::Suspect, Some(FactStatus::Active)) => {
            matches!(assertion.source, FactSource::User | FactSource::Agent)
        }
        (FactStatus::Candidate | FactStatus::Suspect, _) => confirms_candidate(said, assertion),
        (FactStatus::Active | FactStatus::Retired, _) => false,
    }
}

/// Whether `assertion` confirms `said` as a candidate: the user's word, or an agent's in a task
/// other than the one that first asserted it.
fn confirms_candidate(said: &FactRow, assertion: &Assertion<'_>) -> bool {
    match assertion.source {
        FactSource::User => true,
        FactSource::Agent => {
            matches!((said.record.task, assertion.task), (Some(first), Some(now)) if first != now)
        }
        FactSource::Import => false,
    }
}

/// How many current facts not retired are anchored to `root`, the repository's root node alone.
fn global_facts(queries: Queries<'_>, root: &[NodeId]) -> Result<usize, StoreError> {
    let mut count = 0;
    for id in queries.facts_anchored_to(root)? {
        if queries
            .fact(id)?
            .is_some_and(|row| row.record.status != FactStatus::Retired)
        {
            count += 1;
        }
    }
    Ok(count)
}

/// The anchors on `nodes`, each once in node order, with the hashes they hold now.
///
/// # Errors
///
/// [`FactError::Unanchorable`] for the first node that holds no hash, and the store's errors.
pub(crate) fn anchors_now(
    queries: Queries<'_>,
    nodes: &[NodeId],
) -> Result<Vec<FactAnchor>, FactError> {
    let anchors = queries.anchor_hashes(nodes)?;
    match nodes
        .iter()
        .find(|node| !anchors.iter().any(|anchor| anchor.node == **node))
    {
        Some(node) => Err(FactError::Unanchorable { node: *node }),
        None => Ok(anchors),
    }
}

/// The words of `text`, lower case, as a set: what two facts are compared by.
fn words_of(text: &str) -> BTreeSet<String> {
    identifier_tokens(text).into_iter().collect()
}
