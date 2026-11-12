//! Edges set one pair at a time, as a task's links are, and the edges of a type running between
//! chosen nodes. Expected rows are written by hand.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::EdgeType;
use cortex_store::{EdgeRecord, NodeId};

/// An edge to `dst` of `weight`, supported by `count` observations, last seen at task `last_seq`,
/// pointing both ways as a learned link does.
fn link(dst: NodeId, weight: f64, count: u32, last_seq: i64) -> EdgeRecord {
    EdgeRecord {
        dst,
        weight,
        count,
        last_seq,
        is_directed: false,
    }
}

/// Indexes the Python files at `paths` in one transaction and returns their file nodes, in that
/// order.
fn file_nodes(scratch: &Scratch, paths: &[&str]) -> TestResult<Vec<NodeId>> {
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    for path in paths {
        transaction.replace_file(&index(path, Vec::new(), Vec::new())?)?;
    }
    let nodes = paths
        .iter()
        .map(|path| {
            transaction
                .queries()
                .file_node_at(&common::path(path)?)?
                .ok_or_else(|| format!("{path} has no node").into())
        })
        .collect::<TestResult<Vec<NodeId>>>()?;
    transaction.commit()?;
    Ok(nodes)
}

/// An edge set one pair at a time is added if it is not there and changed in place if it is, and
/// the other edges of its node are left alone: `a` is linked to `b` at 0.3 and to `c` at 0.9, then
/// the link to `b` is set again at 1.4, which leaves the link to `c` as it was. Replacing a node's
/// edges, as the index does, would have dropped it.
#[test]
fn an_edge_set_on_its_own_leaves_the_others_alone() -> TestResult {
    let scratch = Scratch::new()?;
    let nodes = file_nodes(&scratch, &["a.py", "b.py", "c.py"])?;
    let (a, b, c) = (nodes[0], nodes[1], nodes[2]);
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    transaction.set_edges(
        EdgeType::CoAccess,
        &[(a, link(b, 0.3, 1, 4)), (a, link(c, 0.9, 2, 4))],
    )?;
    transaction.set_edges(EdgeType::CoAccess, &[(a, link(b, 1.4, 3, 9))])?;
    transaction.commit()?;
    let edges: Vec<(NodeId, f64, u32, i64, bool)> = writer
        .queries()
        .edges_from(a)?
        .into_iter()
        .map(|row| {
            (
                row.edge.dst,
                row.edge.weight,
                row.edge.count,
                row.edge.last_seq,
                row.edge.is_directed,
            )
        })
        .collect();
    assert_eq!(edges, [(b, 1.4, 3, 9, false), (c, 0.9, 2, 4, false)]);
    Ok(())
}

/// The edges of a type running between chosen nodes are those with both ends among them: of the
/// links `a` to `b`, `b` to `c` and `a` to `d`, asking about `a`, `b` and `c` gives the first two
/// and not the third, whose other end is outside; a structural edge from `a` to `b` is of another
/// type and is not among them; and asking about nothing gives nothing.
#[test]
fn the_edges_among_nodes_have_both_ends_among_them() -> TestResult {
    let scratch = Scratch::new()?;
    let nodes = file_nodes(&scratch, &["a.py", "b.py", "c.py", "d.py"])?;
    let (a, b, c, d) = (nodes[0], nodes[1], nodes[2], nodes[3]);
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    transaction.set_edges(
        EdgeType::CoAccess,
        &[
            (a, link(b, 0.3, 1, 4)),
            (b, link(c, 0.6, 2, 5)),
            (a, link(d, 0.9, 3, 6)),
        ],
    )?;
    transaction.set_edges(EdgeType::Structural, &[(a, link(b, 0.8, 1, 0))])?;
    transaction.commit()?;
    let among: Vec<(NodeId, NodeId, f64)> = writer
        .queries()
        .edges_among(&[a, b, c], EdgeType::CoAccess)?
        .into_iter()
        .map(|row| (row.src, row.edge.dst, row.edge.weight))
        .collect();
    assert_eq!(among, [(a, b, 0.3), (b, c, 0.6)]);
    assert_eq!(
        writer.queries().edges_among(&[], EdgeType::CoAccess)?.len(),
        0
    );
    let structural: Vec<(NodeId, NodeId)> = writer
        .queries()
        .edges_among(&[a, b, c], EdgeType::Structural)?
        .into_iter()
        .map(|row| (row.src, row.edge.dst))
        .collect();
    assert_eq!(structural, [(a, b)]);
    Ok(())
}

/// A link goes when either of its nodes goes: the link from `a` to `b` is gone once `b` is
/// removed, and `a` keeps its link to `c`.
#[test]
fn a_link_goes_with_either_of_its_nodes() -> TestResult {
    let scratch = Scratch::new()?;
    let nodes = file_nodes(&scratch, &["a.py", "b.py", "c.py"])?;
    let (a, b, c) = (nodes[0], nodes[1], nodes[2]);
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    transaction.set_edges(
        EdgeType::CoAccess,
        &[(a, link(b, 0.3, 1, 4)), (a, link(c, 0.9, 2, 4))],
    )?;
    transaction.commit()?;
    let mut transaction = writer.transaction()?;
    transaction.remove_file(&common::path("b.py")?)?;
    transaction.commit()?;
    let left: Vec<NodeId> = writer
        .queries()
        .edges_from(a)?
        .into_iter()
        .map(|row| row.edge.dst)
        .collect();
    assert_eq!(left, [c]);
    Ok(())
}

/// The edges of one type are read in the order of their ends and the others left out: `a` links
/// to `b` and `c` as co-change and to `b` as co-access, `b` links to `a` as co-change.
#[test]
fn the_edges_of_one_type_are_read_by_their_ends() -> TestResult {
    let scratch = Scratch::new()?;
    let nodes = file_nodes(&scratch, &["a.py", "b.py", "c.py"])?;
    let [a, b, c] = nodes[..] else {
        return Err("three nodes".into());
    };
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    let one_way = |dst, weight, count, last_seq| EdgeRecord {
        is_directed: true,
        ..link(dst, weight, count, last_seq)
    };
    transaction.replace_edges(
        a,
        EdgeType::CoChange,
        &[one_way(b, 0.5, 4, 0), one_way(c, 0.25, 2, 0)],
    )?;
    transaction.replace_edges(a, EdgeType::CoAccess, &[one_way(b, 1.5, 1, 7)])?;
    transaction.replace_edges(b, EdgeType::CoChange, &[one_way(a, 0.75, 4, 0)])?;
    transaction.commit()?;
    let mut seen = Vec::new();
    writer
        .queries()
        .each_edge_of_type(EdgeType::CoChange, |row| {
            seen.push((row.src, row.edge.dst, row.edge.weight, row.edge.count));
            Ok(())
        })?;
    assert_eq!(seen, [(a, b, 0.5, 4), (a, c, 0.25, 2), (b, a, 0.75, 4)]);
    Ok(())
}
