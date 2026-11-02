//! Each node's interest: written, replaced, read back for chosen nodes and for the files that
//! could still be landmarks, and gone with its node. Expected rows are written by hand.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::InterestTier;
use cortex_store::{InterestRow, NodeId};

/// A node's interest.
fn interest(node: NodeId, score: f64, last_seq: u64, tier: InterestTier) -> InterestRow {
    InterestRow {
        node,
        score,
        last_seq,
        tier,
    }
}

/// Indexes the Python files at `paths` in one transaction and returns their file nodes, in that
/// order.
fn file_nodes(scratch: &Scratch, paths: &[&str]) -> TestResult<Vec<NodeId>> {
    let mut writer = scratch.writer(1000)?;
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

/// Interest is written for a node, read back for the nodes asked for, and replaced in place. Three
/// files are indexed and given scores by hand: `a.py` 4.5 as of task 7 as a landmark, `b.py` 0.6
/// as of task 7 as interesting, `c.py` 0.1 as of task 3 as dark. Reading two of them gives those
/// two, by node; writing `a.py` again replaces its row and leaves the others alone.
#[test]
fn interest_is_written_read_back_and_replaced() -> TestResult {
    let scratch = Scratch::new()?;
    let nodes = file_nodes(&scratch, &["a.py", "b.py", "c.py"])?;
    let (a, b, c) = (nodes[0], nodes[1], nodes[2]);
    let written = [
        interest(a, 4.5, 7, InterestTier::Landmark),
        interest(b, 0.6, 7, InterestTier::Interesting),
        interest(c, 0.1, 3, InterestTier::Dark),
    ];
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_interest(&written)?;
    transaction.commit()?;
    assert_eq!(
        writer.queries().interest_of(&[a, c])?,
        [written[0].clone(), written[2].clone()]
    );
    assert_eq!(writer.queries().interest_of(&[])?, []);
    let again = interest(a, 1.25, 9, InterestTier::Interesting);
    let mut transaction = writer.transaction()?;
    transaction.replace_interest(std::slice::from_ref(&again))?;
    transaction.commit()?;
    assert_eq!(
        writer.queries().interest_of(&[a, b, c])?,
        [again, written[1].clone(), written[2].clone()]
    );
    Ok(())
}

/// The files that could still be landmarks are those whose stored score is at least the threshold,
/// file nodes only: of `a.py` at 3.0, `b.py` at 2.999 and the folder node of `pkg/` at 9.0, only
/// `a.py` stands at 3.0, and the folder is no file however high its score.
#[test]
fn only_files_at_the_threshold_can_still_be_landmarks() -> TestResult {
    let scratch = Scratch::new()?;
    let nodes = file_nodes(&scratch, &["a.py", "b.py", "pkg/c.py"])?;
    let (a, b) = (nodes[0], nodes[1]);
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    let folder = transaction.folder_node(&common::path("pkg")?)?;
    transaction.commit()?;
    let written = [
        interest(a, 3.0, 4, InterestTier::Landmark),
        interest(b, 2.999, 4, InterestTier::Interesting),
        interest(folder, 9.0, 4, InterestTier::Landmark),
    ];
    let mut transaction = writer.transaction()?;
    transaction.replace_interest(&written)?;
    transaction.commit()?;
    assert_eq!(
        writer.queries().file_interest_from(3.0)?,
        [written[0].clone()]
    );
    assert_eq!(
        writer.queries().file_interest_from(0.0)?,
        [written[0].clone(), written[1].clone()]
    );
    Ok(())
}

/// A node's interest goes when its file goes: `a.py` keeps its row while `b.py` is removed, and
/// nothing is left for `b.py`'s node.
#[test]
fn interest_goes_with_its_node() -> TestResult {
    let scratch = Scratch::new()?;
    let nodes = file_nodes(&scratch, &["a.py", "b.py"])?;
    let (a, b) = (nodes[0], nodes[1]);
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_interest(&[
        interest(a, 1.0, 1, InterestTier::Interesting),
        interest(b, 2.0, 1, InterestTier::Interesting),
    ])?;
    transaction.commit()?;
    let mut transaction = writer.transaction()?;
    transaction.remove_file(&common::path("b.py")?)?;
    transaction.commit()?;
    assert_eq!(
        writer.queries().interest_of(&[a, b])?,
        [interest(a, 1.0, 1, InterestTier::Interesting)]
    );
    Ok(())
}
