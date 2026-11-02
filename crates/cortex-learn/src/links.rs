//! The links a task's work draws between the nodes it used together.
//!
//! What a task edited, read and had to look at belongs together: that is what a co-access link
//! says. Every pair of nodes in a task's working set gains weight when the task succeeded, less
//! when nobody can say how it went, nothing when it failed, and the files a reverted task edited
//! together lose a little, since what they had in common was undone.
//!
//! Links are drawn at two levels, between files and between symbols, never between a file and a
//! symbol, and they point both ways. A weight is kept as of the task it was last changed at and
//! decays by a factor for every task since, as interest does.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{EdgeType, LineRange, TaskOutcome};
use cortex_store::{EdgeRecord, NodeId, Queries, StoreError, WriteTx};

use crate::decay::decayed;
use crate::places::Places;
use crate::task_state::{FileNode, Step, TaskState};

/// What a node the task edited counts for. The work itself.
const ROLE_EDITED: f64 = 1.0;

/// What a node the task read counts for when an edited file names it: the task did not only look
/// at it, it is part of what changed.
const ROLE_READ_AND_USED: f64 = 0.6;

/// What a node the task only read counts for.
const ROLE_READ_ONLY: f64 = 0.2;

/// What a successful task's pairs gain, as a share of the learning rate.
const GAIN_SUCCESS: f64 = 1.0;

/// What a task nobody can vouch for gains: something, since the nodes were used together, but far
/// less than work that was seen to hold up.
const GAIN_UNKNOWN: f64 = 0.3;

/// What the files a reverted task edited together lose.
const WEAKEN_REVERTED: f64 = 0.05;

/// Most nodes a task's working set keeps, the rest dropped by role and then by interest: a task
/// that touched hundreds of files says little about any one pair of them, and the pairs grow with
/// the square of the set.
const MAX_WORKING_SET: usize = 40;

/// The most a link can weigh, however many tasks draw it.
const MAX_WEIGHT: f64 = 5.0;

/// What a task's work does to the links between the nodes it used, by pair, the smaller node
/// first. A pair's number is what to add to its weight, which can be negative.
pub type EarnedLinks = BTreeMap<(NodeId, NodeId), f64>;

/// What a task's end brings to the links among the nodes it used: its place in the repository's
/// order of tasks, what became of it, and the settings that say how much a task changes a link and
/// how fast links and interest fade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Linking {
    /// The task's number in the repository, which drives decay.
    pub seq: u64,
    /// What became of the task.
    pub outcome: TaskOutcome,
    /// `learning.eta`: how much one task's work changes a link.
    pub eta: f64,
    /// `learning.link_decay`: what a link keeps of its weight per task.
    pub link_decay: f64,
    /// `learning.interest_decay`: what a node keeps of its interest per task, for ranking the
    /// working set when it is too large.
    pub interest_decay: f64,
}

/// What the work `state` shows does to the links between the nodes it used, read with `queries`
/// for the symbols its lines touch, the files an edited file names and the interest of each node.
///
/// # Errors
///
/// The store's errors.
pub fn links_of(
    queries: Queries<'_>,
    state: &TaskState,
    linking: &Linking,
) -> Result<EarnedLinks, StoreError> {
    let gain = gain(linking.outcome);
    if gain == 0.0 && linking.outcome != TaskOutcome::Reverted {
        return Ok(EarnedLinks::new());
    }
    let mut set = WorkingSet::of(queries, state)?;
    set.cap(queries, linking)?;
    let mut links = EarnedLinks::new();
    if linking.outcome == TaskOutcome::Reverted {
        // What the files of a reverted task had in common was undone, so their link loses weight;
        // the files it only read say nothing either way.
        for (left, right) in pairs(&set.files, Role::Edited) {
            let weight = -WEAKEN_REVERTED * ROLE_EDITED * ROLE_EDITED;
            links.insert((left, right), weight);
        }
        return Ok(links);
    }
    for level in [&set.files, &set.symbols] {
        for (left, right) in every_pair(level) {
            let roles = weight_of(level[&left]) * weight_of(level[&right]);
            links.insert((left, right), linking.eta * roles * gain);
        }
    }
    Ok(links)
}

/// Adds `links` to the stored links in `transaction`, each weight first brought forward to task
/// `linking.seq` by `linking.link_decay` for each task since the link last changed, and held
/// between 0 and [`MAX_WEIGHT`]. A pair with no link yet and nothing to gain is left unwritten.
///
/// # Errors
///
/// The store's errors.
pub fn add_links(
    transaction: &mut WriteTx<'_>,
    linking: &Linking,
    links: &EarnedLinks,
) -> Result<(), StoreError> {
    let nodes: Vec<NodeId> = links
        .keys()
        .flat_map(|(left, right)| [*left, *right])
        .collect::<BTreeSet<NodeId>>()
        .into_iter()
        .collect();
    let held: BTreeMap<(NodeId, NodeId), EdgeRecord> = transaction
        .queries()
        .edges_among(&nodes, EdgeType::CoAccess)?
        .into_iter()
        .map(|row| ((row.src, row.edge.dst), row.edge))
        .collect();
    let mut edges: Vec<(NodeId, EdgeRecord)> = Vec::new();
    for (pair, add) in links {
        let before = held.get(pair);
        if before.is_none() && *add <= 0.0 {
            continue;
        }
        let weight = before.map_or(0.0, |edge| {
            decayed(
                edge.weight,
                u64::try_from(edge.last_seq).unwrap_or(0),
                linking.seq,
                linking.link_decay,
            )
        });
        let count = before.map_or(0, |edge| edge.count).saturating_add(1);
        edges.push((
            pair.0,
            EdgeRecord {
                dst: pair.1,
                weight: (weight + add).clamp(0.0, MAX_WEIGHT),
                count,
                last_seq: i64::try_from(linking.seq).unwrap_or(i64::MAX),
                is_directed: false,
            },
        ));
    }
    transaction.set_edges(EdgeType::CoAccess, &edges)
}

/// What a task's outcome does to the weight of every pair in its working set, as a share of the
/// learning rate: a task that was seen to work teaches most, one nobody can vouch for a little,
/// and one that failed, was reverted or was left nothing at all.
fn gain(outcome: TaskOutcome) -> f64 {
    match outcome {
        TaskOutcome::Success => GAIN_SUCCESS,
        TaskOutcome::Unknown => GAIN_UNKNOWN,
        TaskOutcome::Failure | TaskOutcome::Reverted | TaskOutcome::Abandoned => 0.0,
    }
}

/// What a node's part in the task counts for.
fn weight_of(role: Role) -> f64 {
    match role {
        Role::Edited => ROLE_EDITED,
        Role::ReadAndUsed => ROLE_READ_AND_USED,
        Role::ReadOnly => ROLE_READ_ONLY,
    }
}

/// What a node was to the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    /// Only read.
    ReadOnly,
    /// Read, and named by a file the task edited.
    ReadAndUsed,
    /// Edited.
    Edited,
}

/// The nodes a task used, by level, each with what it was to the task.
struct WorkingSet {
    /// The files, by node.
    files: BTreeMap<NodeId, Role>,
    /// The symbols, by node.
    symbols: BTreeMap<NodeId, Role>,
}

impl WorkingSet {
    /// The nodes `state` shows the task used, each with its role: a node it edited counts as
    /// edited however often it also read it, and a node it read counts as used once an edited
    /// file names it.
    fn of(queries: Queries<'_>, state: &TaskState) -> Result<Self, StoreError> {
        let mut places = Places::new(queries);
        let mut set = Self {
            files: BTreeMap::new(),
            symbols: BTreeMap::new(),
        };
        for step in state.steps() {
            let (file, range, role) = match step {
                Step::Edit { file, range, .. } => (file, *range, Role::Edited),
                Step::Read { file, range, .. } => (file, *range, Role::ReadOnly),
                Step::Search { .. } | Step::Run { .. } => continue,
            };
            set.touch(&mut places, (file, range), role)?;
        }
        set.find_used(queries)?;
        Ok(set)
    }

    /// Notes that the task did `role` to `file` and to the symbols the lines `range` touch.
    fn touch(
        &mut self,
        places: &mut Places<'_>,
        (file, range): (&FileNode, Option<LineRange>),
        role: Role,
    ) -> Result<(), StoreError> {
        if let Some(node) = file.node {
            raise(&mut self.files, node, role);
        }
        if let Some(range) = range {
            for symbol in places.covering(&file.path, range)? {
                raise(&mut self.symbols, symbol, role);
            }
        }
        Ok(())
    }

    /// Raises every node the task only read to read and used when a node it edited, of the same
    /// level, names it: the index's structural edges run from the file or symbol that names
    /// another to the one it names.
    fn find_used(&mut self, queries: Queries<'_>) -> Result<(), StoreError> {
        for level in [&mut self.files, &mut self.symbols] {
            let nodes: Vec<NodeId> = level.keys().copied().collect();
            let named: Vec<NodeId> = queries
                .edges_among(&nodes, EdgeType::Structural)?
                .into_iter()
                .filter(|row| level.get(&row.src) == Some(&Role::Edited))
                .map(|row| row.edge.dst)
                .collect();
            for node in named {
                if let Some(role) = level.get_mut(&node)
                    && *role == Role::ReadOnly
                {
                    *role = Role::ReadAndUsed;
                }
            }
        }
        Ok(())
    }

    /// Drops the nodes past [`MAX_WORKING_SET`], the least of them by role and then by interest,
    /// ties going to the smaller node so that the same work always keeps the same nodes.
    fn cap(&mut self, queries: Queries<'_>, linking: &Linking) -> Result<(), StoreError> {
        let count = self.files.len() + self.symbols.len();
        if count <= MAX_WORKING_SET {
            return Ok(());
        }
        let nodes: Vec<NodeId> = self
            .files
            .keys()
            .chain(self.symbols.keys())
            .copied()
            .collect();
        let interest: BTreeMap<NodeId, f64> = queries
            .interest_of(&nodes)?
            .into_iter()
            .map(|row| {
                (
                    row.node,
                    decayed(row.score, row.last_seq, linking.seq, linking.interest_decay),
                )
            })
            .collect();
        let mut ranked: Vec<(NodeId, Role, f64)> = self
            .files
            .iter()
            .chain(self.symbols.iter())
            .map(|(node, role)| (*node, *role, interest.get(node).copied().unwrap_or(0.0)))
            .collect();
        ranked.sort_by(
            |(left_node, left_role, left), (right_node, right_role, right)| {
                right_role
                    .cmp(left_role)
                    .then(right.total_cmp(left))
                    .then(left_node.get().cmp(&right_node.get()))
            },
        );
        let kept: BTreeSet<NodeId> = ranked
            .into_iter()
            .take(MAX_WORKING_SET)
            .map(|(node, _, _)| node)
            .collect();
        self.files.retain(|node, _| kept.contains(node));
        self.symbols.retain(|node, _| kept.contains(node));
        Ok(())
    }
}

/// Raises `node`'s role to `role` when that is more than it had, and adds it with that role when
/// it is not there.
fn raise(level: &mut BTreeMap<NodeId, Role>, node: NodeId, role: Role) {
    let held = level.entry(node).or_insert(role);
    *held = (*held).max(role);
}

/// Every pair of the nodes of `level`, the smaller node first.
fn every_pair(level: &BTreeMap<NodeId, Role>) -> Vec<(NodeId, NodeId)> {
    let nodes: Vec<NodeId> = level.keys().copied().collect();
    let mut pairs = Vec::new();
    for (at, left) in nodes.iter().enumerate() {
        for right in nodes.iter().skip(at + 1) {
            pairs.push((*left, *right));
        }
    }
    pairs
}

/// Every pair of the nodes of `level` whose role is `role`, the smaller node first.
fn pairs(level: &BTreeMap<NodeId, Role>, role: Role) -> Vec<(NodeId, NodeId)> {
    let of_role: BTreeMap<NodeId, Role> = level
        .iter()
        .filter(|(_, held)| **held == role)
        .map(|(node, held)| (*node, *held))
        .collect();
    every_pair(&of_role)
}
