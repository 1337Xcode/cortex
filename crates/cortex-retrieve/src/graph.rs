//! The graph activation spreads over: the nodes the store holds and the links between them, each
//! weighed for spreading, kept as compressed sparse rows, one set of rows per kind of link.
//!
//! A link's weight is its kind's weight from the settings times its own strength, held within 0
//! to 1: a reference's confidence, a co-change confidence, a co-access link's effective weight (its
//! stored weight faded by the tasks since it last changed) over the most a link can weigh, a
//! folder's hold on its files. Co-change links run the way history shows them, one direction at a
//! time, since each direction has its own confidence; every other link carries activation both
//! ways, since what code uses and what uses it both belong to a task about it. A file is joined to
//! each of its symbols both ways at the containment weight, the same relation one level down:
//! without it, a symbol a prompt names could never reach the co-change and co-access links of its
//! file. Two links of one kind between the same nodes add up, so the sum may pass 1; a link to
//! itself carries nothing and is left out.
//!
//! Each node also carries its standing: how much agents have worked on it, as of the latest task,
//! and for a file whether a work order may point at it.
//!
//! Nodes are numbered from 0 in the order of their ids, and each node's links are sorted by
//! target, so the same rows always make the same graph, whatever order they came in. Node numbers
//! and each kind's count of links are 32-bit, so a graph past that is refused rather than cut.

use cortex_core::{EdgeType, InterestTier, NodeKind, TypeWeights};
use cortex_learn::{MAX_LINK_WEIGHT, decayed};

use crate::by_files::{ByFiles, FileView};

/// No file: what a folder's entry in the table of files holds.
const NO_FILE: u32 = u32::MAX;

/// Why a graph cannot be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// More nodes, or more links of one kind, than 32-bit numbers can count.
    #[error("the graph holds more {what} than 32-bit numbers can count")]
    TooLarge {
        /// What there were too many of: nodes or links.
        what: &'static str,
    },
}

/// What the store says of a node besides its kind and file: how much agents have worked on it, and
/// whether a work order may point at it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Standing {
    /// Its interest brought forward to the latest task; 0 for a node with none.
    pub interest: f64,
    /// The tier that interest puts it in as of the latest task.
    pub tier: InterestTier,
    /// Whether a work order may point at it: not for a generated or vendored file, which the index
    /// holds at file level only.
    pub is_listed: bool,
}

impl Default for Standing {
    /// A node agents have not worked on, which a work order may point at.
    fn default() -> Self {
        Self {
            interest: 0.0,
            tier: InterestTier::Dark,
            is_listed: true,
        }
    }
}

/// One node as the graph takes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeInput<Id> {
    /// Its id.
    pub id: Id,
    /// What it is.
    pub kind: NodeKind,
    /// For a symbol, the file it is defined in.
    pub file: Option<Id>,
    /// What the store says of it.
    pub standing: Standing,
}

/// One stored link as the graph takes it, as the store keeps it: a strength in 0 to 1, or for a
/// co-access link its weight as of the task it last changed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkInput<Id> {
    /// The node it starts at.
    pub src: Id,
    /// The node it points to.
    pub dst: Id,
    /// Its kind.
    pub kind: EdgeType,
    /// Its stored weight.
    pub weight: f64,
    /// For a co-access link, the task it last changed at.
    pub last_seq: u64,
}

/// What weighs the links: the settings' weight of each kind, and what fades co-access links.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weighing {
    /// `retrieval.type_weights`.
    pub type_weights: TypeWeights,
    /// The repository's latest task, which co-access links are faded to.
    pub latest_seq: u64,
    /// `learning.link_decay`: what a co-access link keeps of its weight per task.
    pub link_decay: f64,
}

impl Weighing {
    /// The weight `link` carries while activation spreads: its kind's weight times its strength,
    /// that strength held within 0 to 1.
    fn of<Id>(&self, link: &LinkInput<Id>) -> f64 {
        let weights = self.type_weights;
        let (kind_weight, strength) = match link.kind {
            EdgeType::Structural => (weights.structural, link.weight),
            EdgeType::CoChange => (weights.cochange, link.weight),
            EdgeType::CoAccess => {
                let effective =
                    decayed(link.weight, link.last_seq, self.latest_seq, self.link_decay);
                (weights.coaccess, effective / MAX_LINK_WEIGHT)
            }
            EdgeType::About => (weights.about, link.weight),
            EdgeType::Containment => (weights.containment, link.weight),
        };
        let strength = if strength.is_finite() {
            strength.clamp(0.0, 1.0)
        } else {
            0.0
        };
        kind_weight * strength
    }
}

/// One kind's links, as compressed sparse rows: node `n`'s links are the targets and weights from
/// `offsets[n]` up to `offsets[n + 1]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Rows {
    /// Where each node's links start, and one past the last node's end.
    offsets: Vec<u32>,
    /// Each link's target.
    targets: Vec<u32>,
    /// Each link's weight: the links between the same two nodes added up.
    weights: Vec<f32>,
    /// Each link's heaviest part: the weight of the heaviest single link added up into it.
    peaks: Vec<f32>,
}

impl Rows {
    /// The rows of `links` over `nodes` nodes: each `(source, target, weight)`, sorted and with
    /// links between the same two nodes added up, the heaviest of them kept beside the sum.
    ///
    /// # Errors
    ///
    /// [`GraphError::TooLarge`] for more links than a 32-bit offset reaches.
    pub(crate) fn of(nodes: usize, mut links: Vec<(u32, u32, f64)>) -> Result<Self, GraphError> {
        // INVARIANT: deterministic (links between the same nodes are added in weight order, so
        // the sums do not depend on the order the rows came in)
        links.sort_by(|left, right| {
            (left.0, left.1)
                .cmp(&(right.0, right.1))
                .then(left.2.total_cmp(&right.2))
        });
        let mut rows = Self {
            offsets: Vec::with_capacity(nodes + 1),
            targets: Vec::with_capacity(links.len()),
            weights: Vec::with_capacity(links.len()),
            peaks: Vec::with_capacity(links.len()),
        };
        let mut merged: Vec<(u32, u32, f64, f64)> = Vec::with_capacity(links.len());
        for (source, target, weight) in links {
            match merged.last_mut() {
                Some(last) if (last.0, last.1) == (source, target) => {
                    last.2 += weight;
                    last.3 = last.3.max(weight);
                }
                _ => merged.push((source, target, weight, weight)),
            }
        }
        let offset = |length: usize| {
            u32::try_from(length).map_err(|_| GraphError::TooLarge { what: "links" })
        };
        let mut at = 0;
        for node in 0..nodes {
            rows.offsets.push(offset(rows.targets.len())?);
            while let Some(&(source, target, weight, peak)) = merged.get(at) {
                if usize::try_from(source).unwrap_or(usize::MAX) != node {
                    break;
                }
                rows.targets.push(target);
                rows.weights.push(narrowed(weight));
                rows.peaks.push(narrowed(peak));
                at += 1;
            }
        }
        rows.offsets.push(offset(rows.targets.len())?);
        Ok(rows)
    }

    /// Node `node`'s links: each target and weight.
    pub(crate) fn of_node(&self, node: u32) -> impl Iterator<Item = (u32, f64)> + '_ {
        self.paired(node, &self.weights)
    }

    /// Node `node`'s links: each target and the weight of its heaviest part.
    fn peaks_of_node(&self, node: u32) -> impl Iterator<Item = (u32, f64)> + '_ {
        self.paired(node, &self.peaks)
    }

    /// Node `node`'s links: each target with its value in `values`, which runs beside the targets.
    fn paired<'a>(&'a self, node: u32, values: &'a [f32]) -> impl Iterator<Item = (u32, f64)> + 'a {
        let (start, end) = span_of(&self.offsets, node);
        let targets = self.targets.get(start..end).unwrap_or_default();
        let values = values.get(start..end).unwrap_or_default();
        targets
            .iter()
            .zip(values)
            .map(|(target, value)| (*target, f64::from(*value)))
    }
}

/// Where node `node`'s entries run in the rows `offsets` index: from its offset up to the next.
fn span_of(offsets: &[u32], node: u32) -> (usize, usize) {
    let at = usize::try_from(node).unwrap_or(usize::MAX);
    let start = offsets
        .get(at)
        .map_or(0, |offset| usize::try_from(*offset).unwrap_or(0));
    let end = offsets
        .get(at.saturating_add(1))
        .map_or(start, |offset| usize::try_from(*offset).unwrap_or(start));
    (start, end)
}

/// `weight` as the 32-bit float the rows keep: spreading needs no more, and the floors a link is
/// held to are narrowed the same way, so a link at a floor stays at it.
#[must_use]
pub(crate) fn narrowed(weight: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "weights are kept as 32-bit floats, which spreading needs no more of"
    )]
    let narrowed = weight as f32;
    narrowed
}

/// The graph: its nodes, numbered from 0 in the order of their ids, and each kind's links.
#[derive(Debug, Clone, PartialEq)]
pub struct Graph<Id> {
    /// Each node's id, in order: a node's number is its place here.
    ids: Vec<Id>,
    /// What each node is.
    kinds: Vec<NodeKind>,
    /// Each node's file: a symbol's file, a file itself, [`NO_FILE`] for anything else.
    files: Vec<u32>,
    /// What the store says of each node.
    standings: Vec<Standing>,
    /// Where each node's symbols start in `symbols`, and one past the last node's end: only a file
    /// has any.
    symbol_offsets: Vec<u32>,
    /// Each file's symbols, file by file, each file's in node order.
    symbols: Vec<u32>,
    /// Each kind's links, in the order of [`EdgeType::number`].
    rows: Vec<Rows>,
    /// The sum of each node's link weights over every kind.
    out_weights: Vec<f64>,
    /// The weight of each kind of link it was weighed by.
    type_weights: TypeWeights,
    /// Its view by files, once built.
    by_files: ByFiles,
}

impl<Id: Ord + Copy> Graph<Id> {
    /// The graph of `nodes` and `links`, weighed by `weighing`. A node given twice is taken once,
    /// as the first of its kinds and files in their order; a symbol whose file is no file node has
    /// no file; a link to or from a node not given, of no weight, or to itself, is left out.
    ///
    /// # Errors
    ///
    /// [`GraphError::TooLarge`] for more nodes, or more links of one kind, than 32-bit numbers
    /// count.
    pub fn build(
        nodes: &[NodeInput<Id>],
        links: &[LinkInput<Id>],
        weighing: &Weighing,
    ) -> Result<Self, GraphError> {
        let mut sorted: Vec<NodeInput<Id>> = nodes.to_vec();
        // INVARIANT: deterministic (a node given twice keeps the same kind and file whatever
        // order the rows came in)
        sorted.sort_by_key(|node| (node.id, node.kind, node.file));
        sorted.dedup_by(|later, earlier| later.id == earlier.id);
        if sorted.len() >= usize::try_from(NO_FILE).unwrap_or(usize::MAX) {
            return Err(GraphError::TooLarge { what: "nodes" });
        }
        let ids: Vec<Id> = sorted.iter().map(|node| node.id).collect();
        let index = |id: Id| -> Option<u32> {
            ids.binary_search(&id)
                .ok()
                .and_then(|at| u32::try_from(at).ok())
        };
        let kinds: Vec<NodeKind> = sorted.iter().map(|node| node.kind).collect();
        let standings: Vec<Standing> = sorted.iter().map(|node| node.standing).collect();
        let mut files = vec![NO_FILE; ids.len()];
        let mut by_kind: Vec<Vec<(u32, u32, f64)>> = vec![Vec::new(); EdgeType::ALL.len()];
        let containment = usize::from(EdgeType::Containment.number());
        let membership = weighing.type_weights.containment;
        for (at, node) in sorted.iter().enumerate() {
            let Ok(this) = u32::try_from(at) else {
                continue;
            };
            let file_node = node.file.and_then(index).filter(|file| {
                usize::try_from(*file).ok().and_then(|file| kinds.get(file))
                    == Some(&NodeKind::File)
            });
            match (node.kind, file_node) {
                (NodeKind::File, _) => {
                    if let Some(slot) = files.get_mut(at) {
                        *slot = this;
                    }
                }
                (NodeKind::Symbol, Some(file)) => {
                    if let Some(slot) = files.get_mut(at) {
                        *slot = file;
                    }
                    if membership > 0.0
                        && let Some(rows) = by_kind.get_mut(containment)
                    {
                        rows.push((file, this, membership));
                        rows.push((this, file, membership));
                    }
                }
                _ => {}
            }
        }
        for link in links {
            let (Some(src), Some(dst)) = (index(link.src), index(link.dst)) else {
                continue;
            };
            let weight = weighing.of(link);
            if src == dst || weight <= 0.0 {
                continue;
            }
            let Some(rows) = by_kind.get_mut(usize::from(link.kind.number())) else {
                continue;
            };
            rows.push((src, dst, weight));
            if link.kind != EdgeType::CoChange {
                rows.push((dst, src, weight));
            }
        }
        let rows: Vec<Rows> = by_kind
            .into_iter()
            .map(|links| Rows::of(ids.len(), links))
            .collect::<Result<_, _>>()?;
        let (symbol_offsets, symbols) = members(&kinds, &files)?;
        let out_weights = (0..ids.len())
            .map(|node| {
                let node = u32::try_from(node).unwrap_or(u32::MAX);
                rows.iter()
                    .flat_map(|kind| kind.of_node(node))
                    .map(|(_, weight)| weight)
                    .sum()
            })
            .collect();
        Ok(Self {
            ids,
            kinds,
            files,
            standings,
            symbol_offsets,
            symbols,
            rows,
            out_weights,
            type_weights: weighing.type_weights,
            by_files: ByFiles::default(),
        })
    }

    /// How many nodes it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether it holds no node.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The number of the node `id`, if the graph holds it.
    #[must_use]
    pub fn index_of(&self, id: Id) -> Option<u32> {
        self.ids
            .binary_search(&id)
            .ok()
            .and_then(|at| u32::try_from(at).ok())
    }

    /// The id of node `node`, if there is such a node.
    #[must_use]
    pub fn id(&self, node: u32) -> Option<Id> {
        self.ids.get(usize::try_from(node).ok()?).copied()
    }

    /// What node `node` is, if there is such a node.
    #[must_use]
    pub fn kind(&self, node: u32) -> Option<NodeKind> {
        self.kinds.get(usize::try_from(node).ok()?).copied()
    }

    /// Node `node`'s file: the file a symbol is defined in, a file itself, none for anything else.
    #[must_use]
    pub fn file_of(&self, node: u32) -> Option<u32> {
        self.files
            .get(usize::try_from(node).ok()?)
            .copied()
            .filter(|file| *file != NO_FILE)
    }

    /// What the store says of node `node`, if there is such a node.
    #[must_use]
    pub fn standing(&self, node: u32) -> Option<Standing> {
        self.standings.get(usize::try_from(node).ok()?).copied()
    }

    /// The symbols defined in file `file`, in node order; none for a node that is no file.
    #[must_use]
    pub fn symbols_of(&self, file: u32) -> &[u32] {
        let (start, end) = span_of(&self.symbol_offsets, file);
        self.symbols.get(start..end).unwrap_or_default()
    }

    /// The weight of each kind of link it was weighed by.
    #[must_use]
    pub fn type_weights(&self) -> TypeWeights {
        self.type_weights
    }

    /// Node `node`'s links of `kind`, each target with the weight it carries, by target.
    pub fn links(&self, node: u32, kind: EdgeType) -> impl Iterator<Item = (u32, f64)> + '_ {
        self.rows
            .get(usize::from(kind.number()))
            .into_iter()
            .flat_map(move |rows| rows.of_node(node))
    }

    /// Node `node`'s links of `kind`, each target with the weight of the heaviest single link
    /// added up into it: what one reference, one co-change link or one co-access link carries
    /// alone, by target.
    pub fn peak_links(&self, node: u32, kind: EdgeType) -> impl Iterator<Item = (u32, f64)> + '_ {
        self.rows
            .get(usize::from(kind.number()))
            .into_iter()
            .flat_map(move |rows| rows.peaks_of_node(node))
    }

    /// The sum of node `node`'s link weights over every kind, by which each is divided while
    /// activation spreads; 0 for a node with no links.
    #[must_use]
    pub fn out_weight(&self, node: u32) -> f64 {
        self.out_weights
            .get(usize::try_from(node).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(0.0)
    }

    /// Its view by files, which the first stage of two-stage spreading runs over: built on first
    /// use and kept with the graph.
    ///
    /// # Errors
    ///
    /// [`GraphError::TooLarge`] for more links between files than a 32-bit offset reaches.
    pub(crate) fn by_files(&self) -> Result<&FileView, GraphError> {
        self.by_files.of(self)
    }

    /// Builds its view by files now, if it was not yet, so that the first two-stage spreading over
    /// it does not wait for it: the daemon does so after rebuilding a huge repository's graph.
    ///
    /// # Errors
    ///
    /// [`GraphError::TooLarge`] for more links between files than a 32-bit offset reaches.
    pub fn prepare_by_files(&self) -> Result<(), GraphError> {
        self.by_files().map(drop)
    }

    /// Whether its view by files is built.
    #[must_use]
    pub fn is_prepared_by_files(&self) -> bool {
        self.by_files.is_built()
    }

    /// How many links it holds, counting each direction of a link that runs both ways.
    #[must_use]
    pub fn link_count(&self) -> usize {
        self.rows.iter().map(|rows| rows.targets.len()).sum()
    }
}

/// Each file's symbols, as compressed rows over the nodes `kinds` and `files` describe: where each
/// node's symbols start, and the symbols, file by file.
///
/// # Errors
///
/// [`GraphError::TooLarge`] for more symbols than a 32-bit offset reaches.
fn members(kinds: &[NodeKind], files: &[u32]) -> Result<(Vec<u32>, Vec<u32>), GraphError> {
    let mut pairs: Vec<(u32, u32)> = kinds
        .iter()
        .zip(files)
        .enumerate()
        .filter(|(_, (kind, file))| **kind == NodeKind::Symbol && **file != NO_FILE)
        .filter_map(|(at, (_, file))| Some((*file, u32::try_from(at).ok()?)))
        .collect();
    pairs.sort_unstable();
    let offset =
        |length: usize| u32::try_from(length).map_err(|_| GraphError::TooLarge { what: "symbols" });
    let mut offsets = Vec::with_capacity(kinds.len() + 1);
    let mut symbols = Vec::with_capacity(pairs.len());
    let mut at = 0;
    for node in 0..kinds.len() {
        offsets.push(offset(symbols.len())?);
        while let Some(&(file, symbol)) = pairs.get(at) {
            if usize::try_from(file).unwrap_or(usize::MAX) != node {
                break;
            }
            symbols.push(symbol);
            at += 1;
        }
    }
    offsets.push(offset(symbols.len())?);
    Ok((offsets, symbols))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The settings' weights, the latest task 12, a link keeping 0.995 of its weight per task.
    fn weighing() -> Weighing {
        Weighing {
            type_weights: TypeWeights::default(),
            latest_seq: 12,
            link_decay: 0.995,
        }
    }

    /// A node.
    fn node(id: i64, kind: NodeKind, file: Option<i64>) -> NodeInput<i64> {
        NodeInput {
            id,
            kind,
            file,
            standing: Standing::default(),
        }
    }

    /// A link last changed at task 0.
    fn link(src: i64, dst: i64, kind: EdgeType, weight: f64) -> LinkInput<i64> {
        LinkInput {
            src,
            dst,
            kind,
            weight,
            last_seq: 0,
        }
    }

    /// A folder (1) holding files A (2) and B (3), symbol S (4) in A and T (5) in B.
    fn nodes() -> Vec<NodeInput<i64>> {
        vec![
            node(1, NodeKind::Dir, None),
            node(2, NodeKind::File, None),
            node(3, NodeKind::File, None),
            node(4, NodeKind::Symbol, Some(2)),
            node(5, NodeKind::Symbol, Some(3)),
        ]
    }

    /// A imports B at confidence 0.8 and S calls T at 0.6; A changes with B at 0.5 and B with A at
    /// 0.25; A and B were used together to a weight of 2.5 as of task 10; the folder holds both.
    fn links() -> Vec<LinkInput<i64>> {
        vec![
            link(2, 3, EdgeType::Structural, 0.8),
            link(4, 5, EdgeType::Structural, 0.6),
            link(2, 3, EdgeType::CoChange, 0.5),
            link(3, 2, EdgeType::CoChange, 0.25),
            LinkInput {
                last_seq: 10,
                ..link(2, 3, EdgeType::CoAccess, 2.5)
            },
            link(1, 2, EdgeType::Containment, 1.0),
            link(1, 3, EdgeType::Containment, 1.0),
        ]
    }

    /// Node `id`'s links of `kind`, by id, with their weights.
    fn of(graph: &Graph<i64>, id: i64, kind: EdgeType) -> Vec<(i64, f64)> {
        let Some(node) = graph.index_of(id) else {
            return Vec::new();
        };
        graph
            .links(node, kind)
            .filter_map(|(target, weight)| Some((graph.id(target)?, weight)))
            .collect()
    }

    /// Whether two lists of weighted links agree, the weights within a 32-bit float's rounding.
    fn agree(found: &[(i64, f64)], expected: &[(i64, f64)]) -> bool {
        found.len() == expected.len()
            && found
                .iter()
                .zip(expected)
                .all(|((id, weight), (want_id, want))| {
                    id == want_id && (weight - want).abs() < 1e-6
                })
    }

    /// Worked by hand. A's links: B structurally at 1.0 x 0.8 = 0.8; B by co-change at 0.6 x 0.5
    /// = 0.3; B by co-access at 0.8 x (2.5 x 0.995^2 / 5) = 0.8 x 0.4950125 = 0.39601; the folder
    /// and S by containment at 0.1 each; 1.69601 in all. B's: A at 0.8 the other way; A by
    /// co-change at 0.6 x 0.25 = 0.15, its own direction; A by co-access 0.39601; the folder and T
    /// at 0.1; 1.54601. S: T at 0.6 and A at 0.1, 0.7; T likewise; the folder its two files at
    /// 0.1, 0.2. A symbol's file is its defining file, a file's itself, a folder's none. Links:
    /// structural 4 (two each way), co-change 2, co-access 2, containment 8 (two folder links and
    /// two memberships, both ways).
    #[test]
    fn links_are_weighed_as_worked_out() -> Result<(), GraphError> {
        let graph = Graph::build(&nodes(), &links(), &weighing())?;
        assert_eq!(graph.len(), 5);
        assert!(agree(&of(&graph, 2, EdgeType::Structural), &[(3, 0.8)]));
        assert!(agree(&of(&graph, 2, EdgeType::CoChange), &[(3, 0.3)]));
        assert!(agree(&of(&graph, 2, EdgeType::CoAccess), &[(3, 0.396_01)]));
        assert!(agree(
            &of(&graph, 2, EdgeType::Containment),
            &[(1, 0.1), (4, 0.1)]
        ));
        let index = |id| graph.index_of(id).unwrap_or(u32::MAX);
        assert!((graph.out_weight(index(2)) - 1.696_01).abs() < 1e-6);
        assert!(agree(&of(&graph, 3, EdgeType::Structural), &[(2, 0.8)]));
        assert!(agree(&of(&graph, 3, EdgeType::CoChange), &[(2, 0.15)]));
        assert!(agree(&of(&graph, 3, EdgeType::CoAccess), &[(2, 0.396_01)]));
        assert!(agree(
            &of(&graph, 3, EdgeType::Containment),
            &[(1, 0.1), (5, 0.1)]
        ));
        assert!((graph.out_weight(index(3)) - 1.546_01).abs() < 1e-6);
        assert!(agree(&of(&graph, 4, EdgeType::Structural), &[(5, 0.6)]));
        assert!(agree(&of(&graph, 4, EdgeType::Containment), &[(2, 0.1)]));
        assert!((graph.out_weight(index(4)) - 0.7).abs() < 1e-6);
        assert!(agree(&of(&graph, 5, EdgeType::Structural), &[(4, 0.6)]));
        assert!(agree(
            &of(&graph, 1, EdgeType::Containment),
            &[(2, 0.1), (3, 0.1)]
        ));
        assert!((graph.out_weight(index(1)) - 0.2).abs() < 1e-6);
        let file_of = |id| graph.index_of(id).and_then(|node| graph.file_of(node));
        assert_eq!(file_of(1), None);
        assert_eq!(file_of(2), graph.index_of(2));
        assert_eq!(file_of(4), graph.index_of(2));
        assert_eq!(file_of(5), graph.index_of(3));
        let symbols_of = |id| -> Vec<u32> {
            graph
                .index_of(id)
                .map(|node| graph.symbols_of(node).to_vec())
                .unwrap_or_default()
        };
        assert_eq!(symbols_of(2), [index(4)]);
        assert_eq!(symbols_of(3), [index(5)]);
        assert_eq!(symbols_of(1), Vec::<u32>::new());
        assert_eq!(symbols_of(4), Vec::<u32>::new());
        assert_eq!(graph.type_weights(), TypeWeights::default());
        assert_eq!(graph.link_count(), 16);
        Ok(())
    }

    /// A kind weighed 0 is left out, a file's hold on its symbols included when containment is:
    /// with co-change at 0, A's out-weight is 1.69601 less its co-change 0.3; with containment at
    /// 0, the folder has no links at all and S only its call, though A still lists S as its symbol.
    #[test]
    fn a_kind_weighed_zero_is_left_out() -> Result<(), GraphError> {
        let mut no_cochange = weighing();
        no_cochange.type_weights.cochange = 0.0;
        let graph = Graph::build(&nodes(), &links(), &no_cochange)?;
        assert_eq!(of(&graph, 2, EdgeType::CoChange), []);
        let a = graph.index_of(2).unwrap_or(u32::MAX);
        assert!((graph.out_weight(a) - 1.396_01).abs() < 1e-6);
        let mut no_containment = weighing();
        no_containment.type_weights.containment = 0.0;
        let graph = Graph::build(&nodes(), &links(), &no_containment)?;
        assert_eq!(of(&graph, 1, EdgeType::Containment), []);
        assert_eq!(graph.out_weight(graph.index_of(1).unwrap_or(u32::MAX)), 0.0);
        assert!(agree(&of(&graph, 4, EdgeType::Structural), &[(5, 0.6)]));
        assert_eq!(of(&graph, 4, EdgeType::Containment), []);
        let a = graph.index_of(2).unwrap_or(u32::MAX);
        assert_eq!(graph.symbols_of(a), [graph.index_of(4).unwrap_or(u32::MAX)]);
        Ok(())
    }

    /// Two structural links between A and B, 0.5 one way and 0.3 the other, add up both ways to
    /// 0.8, the heavier of the two, 0.5, kept as the link's heaviest part. A link to a node not given, to itself, or of weight 0 or below is left out; a strength
    /// above 1 counts as 1; a co-access link of weight 5 last changed 200 tasks ago keeps
    /// 0.995^200 = 0.3669578 of it, so 0.8 x 5 x 0.3669578 / 5 = 0.2935663.
    #[test]
    fn links_add_up_and_strange_ones_are_left_out() -> Result<(), GraphError> {
        let rows = [
            link(2, 3, EdgeType::Structural, 0.5),
            link(3, 2, EdgeType::Structural, 0.3),
            link(2, 99, EdgeType::Structural, 1.0),
            link(4, 4, EdgeType::Structural, 1.0),
            link(4, 5, EdgeType::Structural, 0.0),
            link(5, 4, EdgeType::CoChange, -0.2),
            link(1, 2, EdgeType::About, 1.5),
            link(4, 5, EdgeType::CoAccess, 5.0),
        ];
        let mut faded = weighing();
        faded.latest_seq = 200;
        let graph = Graph::build(&nodes(), &rows, &faded)?;
        assert!(agree(&of(&graph, 2, EdgeType::Structural), &[(3, 0.8)]));
        assert!(agree(&of(&graph, 3, EdgeType::Structural), &[(2, 0.8)]));
        let a = graph.index_of(2).unwrap_or(u32::MAX);
        let peaks: Vec<(i64, f64)> = graph
            .peak_links(a, EdgeType::Structural)
            .filter_map(|(target, peak)| Some((graph.id(target)?, peak)))
            .collect();
        assert!(agree(&peaks, &[(3, 0.5)]));
        assert_eq!(of(&graph, 4, EdgeType::Structural), []);
        assert_eq!(of(&graph, 5, EdgeType::CoChange), []);
        assert!(agree(&of(&graph, 1, EdgeType::About), &[(2, 1.0)]));
        assert!(agree(
            &of(&graph, 4, EdgeType::CoAccess),
            &[(5, 0.293_566_26)]
        ));
        Ok(())
    }

    /// A node given twice keeps the same kind and file whichever came first: the first in their
    /// order, a file before a symbol. A symbol whose file is a folder has no file, and no link to
    /// it.
    #[test]
    fn nodes_given_twice_and_strange_files_are_settled() -> Result<(), GraphError> {
        let twice = [
            node(2, NodeKind::Symbol, Some(1)),
            node(1, NodeKind::Dir, None),
            node(2, NodeKind::File, None),
            node(3, NodeKind::Symbol, Some(1)),
        ];
        let mut reversed = twice;
        reversed.reverse();
        let graph = Graph::build(&twice, &[], &weighing())?;
        assert_eq!(graph, Graph::build(&reversed, &[], &weighing())?);
        assert_eq!(graph.len(), 3);
        let node_of = |id| graph.index_of(id).unwrap_or(u32::MAX);
        assert_eq!(graph.kind(node_of(2)), Some(NodeKind::File));
        assert_eq!(graph.file_of(node_of(3)), None);
        assert_eq!(graph.link_count(), 0);
        Ok(())
    }

    proptest! {
        /// The same rows in any order give the same graph.
        #[test]
        fn order_does_not_matter(
            rows in proptest::collection::vec(
                (1..20_i64, 1..20_i64, 0..5_u8, 0.0..1.0_f64),
                0..60,
            ),
            shift in 0..60_usize,
        ) {
            let nodes: Vec<NodeInput<i64>> = (1..20)
                .map(|id| {
                    let is_symbol = id % 3 == 0;
                    let kind = if is_symbol { NodeKind::Symbol } else { NodeKind::File };
                    node(id, kind, is_symbol.then_some(id - 1))
                })
                .collect();
            let links: Vec<LinkInput<i64>> = rows
                .iter()
                .filter_map(|(src, dst, kind, weight)| {
                    Some(link(*src, *dst, EdgeType::from_number(*kind)?, *weight))
                })
                .collect();
            let mut turned = links.clone();
            let turns = shift.min(turned.len());
            turned.rotate_left(turns);
            let mut reversed = nodes.clone();
            reversed.reverse();
            let graph = Graph::build(&nodes, &links, &weighing());
            prop_assert_eq!(graph, Graph::build(&reversed, &turned, &weighing()));
        }
    }
}
