//! What tasks read next, kept per pair of files: set and set again, read from a list of files,
//! paged for upkeep, removed, and gone with a file's node.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::RepoPath;
use cortex_store::{NodeId, TransitionRow};

/// The transition from `src` to `dst` of `weight` and `count`, last updated at task `last_seq`.
fn row(src: NodeId, dst: NodeId, (weight, count, last_seq): (f64, u32, u64)) -> TransitionRow {
    TransitionRow {
        src,
        dst,
        weight,
        count,
        last_seq,
    }
}

/// Worked by hand. `a` to `b` 1.0 once at task 1, `a` to `c` 0.3 once at 2, `b` to `c` 1.0 once at
/// 3; then `a` to `b` set again, 1.5 twice at 4. From `a`: `b` (1.5, twice, 4) then `c`; from `b`
/// and `c` together: `b` to `c` alone. Paged two at a time: (`a`, `b`), (`a`, `c`), then (`b`,
/// `c`), then nothing. Removing (`a`, `c`) and a pair never kept removes one. Removing `b.py`
/// takes the transitions to and from it: none is left.
#[test]
fn transitions_are_set_read_paged_and_removed() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let mut node = |path: &str| -> TestResult<NodeId> {
        Ok(transaction
            .replace_file(&index(path, Vec::new(), Vec::new())?)?
            .file_node)
    };
    let (a, b, c) = (node("a.py")?, node("b.py")?, node("c.py")?);
    transaction.set_transitions(&[
        row(a, b, (1.0, 1, 1)),
        row(a, c, (0.3, 1, 2)),
        row(b, c, (1.0, 1, 3)),
    ])?;
    transaction.set_transitions(&[row(a, b, (1.5, 2, 4))])?;
    transaction.commit()?;
    let queries = writer.queries();
    assert_eq!(
        queries.transitions_from(&[a])?,
        [row(a, b, (1.5, 2, 4)), row(a, c, (0.3, 1, 2))]
    );
    assert_eq!(queries.transitions_from(&[c, b])?, [row(b, c, (1.0, 1, 3))]);
    assert_eq!(queries.transitions_from(&[])?, []);
    let first = queries.transitions_after(None, 2)?;
    assert_eq!(first, [row(a, b, (1.5, 2, 4)), row(a, c, (0.3, 1, 2))]);
    let next = queries.transitions_after(Some((a, c)), 2)?;
    assert_eq!(next, [row(b, c, (1.0, 1, 3))]);
    assert_eq!(queries.transitions_after(Some((b, c)), 2)?, []);

    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.remove_transitions(&[(a, c), (c, a)])?, 1);
    transaction.remove_file(&RepoPath::new("b.py")?)?;
    transaction.commit()?;
    assert_eq!(writer.queries().transitions_after(None, 10)?, []);
    Ok(())
}
