//! The rows that fade, paged for upkeep and removed once faded: links of use and nodes' interest,
//! worked out by hand.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{EdgeType, InterestTier};
use cortex_store::{EdgeRecord, FadingLink, InterestRow, NodeId};

/// An undirected edge to `dst` of `weight`, last changed at task `last_seq`.
fn edge(dst: NodeId, weight: f64, last_seq: i64) -> EdgeRecord {
    EdgeRecord {
        dst,
        weight,
        count: 1,
        last_seq,
        is_directed: false,
    }
}

/// Worked by hand. Links of use `a`-`b` 0.5 at task 1, `a`-`c` 0.3 at 2, `b`-`c` 0.2 at 3, and a
/// link by code `a`-`b`. Paged two at a time: (`a`, `b`) and (`a`, `c`), then (`b`, `c`), then none.
/// Removing (`a`, `b`) removes the link of use alone, one; the link by code stays. Interest of `a`,
/// `b` and `c`, paged two at a time: `a` and `b`, then `c`; removing `a`'s, one, leaves two.
#[test]
fn links_of_use_and_interest_are_paged_and_removed() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let mut node = |path: &str| -> TestResult<NodeId> {
        Ok(transaction
            .replace_file(&index(path, Vec::new(), Vec::new())?)?
            .file_node)
    };
    let (a, b, c) = (node("a.py")?, node("b.py")?, node("c.py")?);
    transaction.set_edges(
        EdgeType::CoAccess,
        &[
            (a, edge(b, 0.5, 1)),
            (a, edge(c, 0.3, 2)),
            (b, edge(c, 0.2, 3)),
        ],
    )?;
    transaction.set_edges(EdgeType::Structural, &[(a, edge(b, 0.9, 0))])?;
    let link = |src, dst, weight, last_seq| FadingLink {
        src,
        dst,
        weight,
        last_seq,
    };
    let queries = transaction.queries();
    assert_eq!(
        queries.coaccess_links_after(None, 2)?,
        [link(a, b, 0.5, 1), link(a, c, 0.3, 2)]
    );
    assert_eq!(
        queries.coaccess_links_after(Some((a, c)), 2)?,
        [link(b, c, 0.2, 3)]
    );
    assert_eq!(queries.coaccess_links_after(Some((b, c)), 2)?, []);
    assert_eq!(transaction.remove_coaccess_links(&[(a, b)])?, 1);
    assert_eq!(
        transaction.queries().coaccess_links_after(None, 9)?,
        [link(a, c, 0.3, 2), link(b, c, 0.2, 3)]
    );
    assert_eq!(
        transaction
            .queries()
            .edges_among(&[a, b], EdgeType::Structural)?
            .len(),
        1
    );
    let interest = |node, score| InterestRow {
        node,
        score,
        last_seq: 1,
        tier: InterestTier::Dark,
    };
    transaction.replace_interest(&[interest(a, 1.0), interest(b, 2.0), interest(c, 3.0)])?;
    let nodes =
        |rows: Vec<InterestRow>| -> Vec<NodeId> { rows.iter().map(|row| row.node).collect() };
    assert_eq!(
        nodes(transaction.queries().interest_after(None, 2)?),
        [a, b]
    );
    assert_eq!(
        nodes(transaction.queries().interest_after(Some(b), 2)?),
        [c]
    );
    assert_eq!(transaction.remove_interest(&[a])?, 1);
    assert_eq!(
        nodes(transaction.queries().interest_after(None, 9)?),
        [b, c]
    );
    transaction.commit()?;
    Ok(())
}
