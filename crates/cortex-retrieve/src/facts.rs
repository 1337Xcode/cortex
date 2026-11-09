//! The project facts a work order carries: the active facts about the files it lists, the closest
//! to the code it shows first, each written only while it is true of the files on disk.
//!
//! A fact is about a listed file when one of its anchors is on that file or on a symbol the file
//! defines. Closest first: a fact on a symbol whose code is shown, then one on another symbol of a
//! listed file, then one on a listed file itself, an anchor elsewhere counting for nothing; then
//! the fact about the better-ranked file; then the fact given to agents more often; then the older.
//! When the work order is written, a fact is kept only while each of its anchors holds the hash the
//! fact kept, as the index says, and each anchor's file on disk is the one the index read, read and
//! hashed then: a fact whose code changed since is withheld, never shown on trust. A fact whose
//! text or anchor names could not be shown as they are is withheld too.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use cortex_core::{ContentHash, FactStatus, RepoPath, Ulid, is_plain};
use cortex_store::{AnchorState, FactId, FactRow, NodeId, Queries, StoreError};

use crate::disk::CheckedFile;

/// Most project facts a work order carries.
pub const MAX_ORDER_FACTS: usize = 5;

/// Most facts weighed for a work order: twice what it carries, so that a few withheld when it is
/// written still leave it its five.
const WEIGHED_FACTS: usize = 2 * MAX_ORDER_FACTS;

/// Most facts about the listed files read to be weighed: far more than a file is about, and few
/// enough that a file noted to excess costs a prompt little.
const MAX_FACTS_READ: usize = 64;

/// Where a fact ranks: how close its closest anchor is to the code shown, the position of that
/// anchor's file, more uses first, then the older fact.
type Rank = (u8, usize, Reverse<u64>, FactId);

/// A file a work order lists, as the facts about it are weighed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedFile {
    /// The file.
    pub path: RepoPath,
    /// Its node.
    pub node: NodeId,
    /// The nodes of the symbols of the spans the work order lists of it: the code it shows.
    pub shown: Vec<NodeId>,
}

/// A project fact a work order may carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderFact {
    /// The fact.
    pub id: FactId,
    /// The rule.
    pub text: String,
    /// What it is about.
    pub anchors: Vec<FactAt>,
}

/// One anchor of a fact, and what must hold for the fact to be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactAt {
    /// How a note names it: a path, or a path, `:` and a symbol.
    pub name: String,
    /// The file it is in.
    pub path: RepoPath,
    /// What the index holds for that file: the fact is shown only while the file holds that.
    pub content: Option<ContentHash>,
    /// Whether the hash the fact kept is the hash the index holds for the anchor now.
    pub is_current: bool,
}

/// The active facts about `files`, the best-ranked file first, closest to the code shown first, at
/// most twice [`MAX_ORDER_FACTS`]. A fact with an anchor that is gone is left out.
///
/// # Errors
///
/// The store's errors.
pub fn order_facts(
    queries: Queries<'_>,
    files: &[ListedFile],
) -> Result<Vec<OrderFact>, StoreError> {
    let nodes: Vec<NodeId> = files.iter().map(|file| file.node).collect();
    let ids = queries.facts_in_files(&nodes, FactStatus::Active)?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows: Vec<FactRow> = Vec::new();
    for id in ids.into_iter().take(MAX_FACTS_READ) {
        rows.extend(queries.fact(id)?);
    }
    let anchored: Vec<NodeId> = rows
        .iter()
        .flat_map(|row| row.record.anchors.iter().map(|anchor| anchor.node))
        .collect();
    let states: BTreeMap<NodeId, AnchorState> = queries
        .anchor_states(&anchored)?
        .into_iter()
        .map(|state| (state.node, state))
        .collect();
    let mut ranked: Vec<(Rank, OrderFact)> = rows
        .iter()
        .filter_map(|row| {
            let fact = order_fact(row, &states)?;
            let (tier, position) = row
                .record
                .anchors
                .iter()
                .filter_map(|anchor| closeness(anchor.node, states.get(&anchor.node)?, files))
                .min()?;
            Some(((tier, position, Reverse(row.uses), row.id), fact))
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    Ok(ranked
        .into_iter()
        .take(WEIGHED_FACTS)
        .map(|(_, fact)| fact)
        .collect())
}

/// The facts task `task` was given that are active and true now of the files in the repository at
/// `root`, read up to `max_bytes`, in id order, at most [`MAX_ORDER_FACTS`]: what a recap after a
/// compaction carries again.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn given_and_true(
    queries: Queries<'_>,
    task: Ulid,
    (root, max_bytes): (&Path, u64),
) -> Result<Vec<OrderFact>, StoreError> {
    let mut rows: Vec<FactRow> = Vec::new();
    for id in queries
        .facts_given_in(task)?
        .into_iter()
        .take(MAX_FACTS_READ)
    {
        rows.extend(
            queries.fact(id)?.filter(|row| {
                row.record.status == FactStatus::Active && row.superseded_by.is_none()
            }),
        );
    }
    let anchored: Vec<NodeId> = rows
        .iter()
        .flat_map(|row| row.record.anchors.iter().map(|anchor| anchor.node))
        .collect();
    let states: BTreeMap<NodeId, AnchorState> = queries
        .anchor_states(&anchored)?
        .into_iter()
        .map(|state| (state.node, state))
        .collect();
    let facts: Vec<OrderFact> = rows
        .iter()
        .filter_map(|row| order_fact(row, &states))
        .collect();
    let kept = true_now(&facts, root, max_bytes);
    Ok(kept
        .into_iter()
        .filter_map(|at| facts.get(at).cloned())
        .collect())
}

/// `row` as a work order carries it, each anchor with where it stands in `states`; none when an
/// anchor is gone.
fn order_fact(row: &FactRow, states: &BTreeMap<NodeId, AnchorState>) -> Option<OrderFact> {
    let anchors = row
        .record
        .anchors
        .iter()
        .map(|anchor| {
            let state = states.get(&anchor.node)?;
            Some(FactAt {
                name: state.name(),
                path: state.path.clone(),
                content: state.content,
                is_current: state.hash == Some(anchor.hash),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(OrderFact {
        id: row.id,
        text: row.record.text.clone(),
        anchors,
    })
}

/// How close the anchor on `node`, standing at `state`, is to the code of `files`: 0 for a symbol
/// whose code is shown, 1 for another symbol of a listed file, 2 for a listed file itself, with
/// the position of the file; none for an anchor on no listed file.
fn closeness(node: NodeId, state: &AnchorState, files: &[ListedFile]) -> Option<(u8, usize)> {
    let position = files.iter().position(|file| file.path == state.path)?;
    let file = files.get(position)?;
    let tier = match &state.symbol {
        Some(_) if file.shown.contains(&node) => 0,
        Some(_) => 1,
        None => 2,
    };
    Some((tier, position))
}

/// The positions of the facts of `facts` true now, in order, at most [`MAX_ORDER_FACTS`]: each
/// anchor holds the hash the fact kept and its file in the repository at `root`, read up to
/// `max_bytes`, is the one the index read; the text and every anchor's name can be shown as they
/// are.
#[must_use]
pub(crate) fn true_now(facts: &[OrderFact], root: &Path, max_bytes: u64) -> Vec<usize> {
    let mut read: BTreeMap<(RepoPath, ContentHash), bool> = BTreeMap::new();
    let mut is_as_indexed = |path: &RepoPath, content: &ContentHash| {
        *read
            .entry((path.clone(), *content))
            .or_insert_with(|| CheckedFile::read(root, path, content, max_bytes).is_ok())
    };
    let mut kept = Vec::new();
    for (at, fact) in facts.iter().enumerate() {
        if kept.len() >= MAX_ORDER_FACTS {
            break;
        }
        // INVARIANT: true-at-render (a fact is shown only while every anchor holds the hash it
        // kept and every anchor's file on disk is the one the index read; otherwise it is withheld)
        let is_true = !fact.anchors.is_empty()
            && fact.anchors.iter().all(|anchor| {
                anchor.is_current
                    && is_plain(&anchor.name)
                    && anchor
                        .content
                        .is_some_and(|content| is_as_indexed(&anchor.path, &content))
            });
        if is_true && is_plain(&fact.text) && !fact.text.trim().is_empty() {
            kept.push(at);
        }
    }
    kept
}

/// Writes the "Project facts" part of a work order to `text`: the facts of `facts` at positions
/// `kept`, each on a line of its own with what it is about; nothing when none is kept.
pub(crate) fn write_facts(text: &mut String, facts: &[OrderFact], kept: &[usize]) {
    let lines: Vec<&OrderFact> = kept.iter().filter_map(|at| facts.get(*at)).collect();
    if lines.is_empty() {
        return;
    }
    text.push_str("Project facts:\n");
    for fact in lines {
        let _ = writeln!(text, "  - {}", fact_line(fact));
    }
}

/// `fact` as one line: its text and what it is about, `(anchored to path[:symbol], ...)`, the
/// anchors in name order, each once.
pub(crate) fn fact_line(fact: &OrderFact) -> String {
    let names: BTreeSet<&str> = fact
        .anchors
        .iter()
        .map(|anchor| anchor.name.as_str())
        .collect();
    let names: Vec<&str> = names.into_iter().collect();
    format!("{} (anchored to {})", fact.text, names.join(", "))
}
