//! The co-change counts and the miner's state: written whole, added to, read back for chosen
//! files, gone with their files; and which files count. Expected rows are written by hand.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{EdgeType, FileFlags};
use cortex_store::{EdgeRecord, FileCommitsRow, FileId, GitStateKey, NodeId, PairRow};

/// Indexes the Python files at `paths` in one transaction and returns their ids, in that order.
fn indexed(scratch: &Scratch, paths: &[&str]) -> TestResult<Vec<FileId>> {
    let mut writer = scratch.writer(1000)?;
    let mut transaction = writer.transaction()?;
    for path in paths {
        transaction.replace_file(&index(path, Vec::new(), Vec::new())?)?;
    }
    let ids = paths
        .iter()
        .map(|path| {
            transaction
                .queries()
                .file(&common::path(path)?)?
                .map(|row| row.id)
                .ok_or_else(|| format!("{path} is not stored").into())
        })
        .collect::<TestResult<Vec<FileId>>>()?;
    transaction.commit()?;
    Ok(ids)
}

/// A pair's counts.
fn pair(file_a: FileId, file_b: FileId, count: u32, weighted: f64, seq: u64) -> PairRow {
    PairRow {
        file_a,
        file_b,
        count,
        weighted,
        last_commit_seq: seq,
    }
}

/// A file's counts.
fn commits(file: FileId, count: u32, weighted: f64) -> FileCommitsRow {
    FileCommitsRow {
        file,
        count,
        weighted,
    }
}

/// Counts added to none read back as added; cleared and added again, only the second remain; added
/// to, they sum, the newer shared commit kept: `a` 2 + 1 commits weighing 1.5 + 0.5, the pair
/// 2 + 1 co-changes weighing 1.5 + 0.5, its newest shared commit 7 then 9.
#[test]
fn counts_are_cleared_and_added_to() -> TestResult {
    let scratch = Scratch::new()?;
    let [a, b, c] = indexed(&scratch, &["a.py", "b.py", "c.py"])?[..] else {
        return Err("three files".into());
    };
    let mut writer = scratch.writer(1000)?;
    let mut transaction = writer.transaction()?;
    transaction.add_cochange(
        &[commits(a, 1, 1.0), commits(c, 1, 1.0)],
        &[pair(a, c, 1, 1.0, 3)],
    )?;
    assert_eq!(
        transaction.queries().cochange_pairs_of(&[c])?,
        [pair(a, c, 1, 1.0, 3)]
    );
    transaction.clear_cochange()?;
    transaction.add_cochange(
        &[commits(a, 2, 1.5), commits(b, 2, 1.5)],
        &[pair(a, b, 2, 1.5, 7)],
    )?;
    let queries = transaction.queries();
    assert_eq!(
        queries.file_commits_of(&[a, b, c])?,
        [commits(a, 2, 1.5), commits(b, 2, 1.5)]
    );
    assert_eq!(queries.cochange_pairs_of(&[c])?, []);
    transaction.add_cochange(
        &[commits(a, 1, 0.5), commits(b, 1, 0.5), commits(c, 1, 0.5)],
        &[pair(a, b, 1, 0.5, 9), pair(b, c, 1, 0.5, 9)],
    )?;
    let queries = transaction.queries();
    assert_eq!(
        queries.file_commits_of(&[a, b, c])?,
        [commits(a, 3, 2.0), commits(b, 3, 2.0), commits(c, 1, 0.5)]
    );
    assert_eq!(queries.cochange_pairs_of(&[a])?, [pair(a, b, 3, 2.0, 9)]);
    assert_eq!(queries.cochange_pairs_of(&[c])?, [pair(b, c, 1, 0.5, 9)]);
    transaction.commit()?;
    Ok(())
}

/// A file's counts and pairs go with the file.
#[test]
fn counts_go_with_their_file() -> TestResult {
    let scratch = Scratch::new()?;
    let [a, b] = indexed(&scratch, &["a.py", "b.py"])?[..] else {
        return Err("two files".into());
    };
    let mut writer = scratch.writer(1000)?;
    let mut transaction = writer.transaction()?;
    transaction.add_cochange(
        &[commits(a, 2, 2.0), commits(b, 2, 2.0)],
        &[pair(a, b, 2, 2.0, 2)],
    )?;
    transaction.remove_file(&common::path("b.py")?)?;
    let queries = transaction.queries();
    assert_eq!(queries.file_commits_of(&[a, b])?, [commits(a, 2, 2.0)]);
    assert_eq!(queries.cochange_pairs_of(&[a, b])?, []);
    transaction.commit()?;
    Ok(())
}

/// The miner's state reads back what was set last, nothing before it is set, and nothing once it
/// is removed, the other values staying.
#[test]
fn the_git_state_keeps_the_last_value() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1000)?;
    let mut transaction = writer.transaction()?;
    assert_eq!(
        transaction.queries().git_state(GitStateKey::LastCommit)?,
        None
    );
    transaction.set_git_state(GitStateKey::LastCommit, "abc")?;
    transaction.set_git_state(GitStateKey::LastCommit, "def")?;
    transaction.set_git_state(GitStateKey::LastSeq, "12")?;
    transaction.commit()?;
    let reader = scratch.reader()?;
    let queries = reader.queries();
    assert_eq!(
        queries.git_state(GitStateKey::LastCommit)?.as_deref(),
        Some("def")
    );
    assert_eq!(
        queries.git_state(GitStateKey::LastSeq)?.as_deref(),
        Some("12")
    );
    assert_eq!(queries.git_state(GitStateKey::FullAt)?, None);
    let mut transaction = writer.transaction()?;
    transaction.remove_git_state(GitStateKey::LastCommit)?;
    let queries = transaction.queries();
    assert_eq!(queries.git_state(GitStateKey::LastCommit)?, None);
    assert_eq!(
        queries.git_state(GitStateKey::LastSeq)?.as_deref(),
        Some("12")
    );
    transaction.commit()?;
    Ok(())
}

/// The files that count are those neither generated nor vendored, by path, each with its node.
#[test]
fn generated_and_vendored_files_do_not_count() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1000)?;
    let mut transaction = writer.transaction()?;
    for (path, flags) in [
        ("src/b.py", FileFlags::empty()),
        ("gen/g.py", FileFlags::GENERATED),
        ("vendor/v.py", FileFlags::VENDORED),
        ("src/a.py", FileFlags::empty()),
    ] {
        let mut file = index(path, Vec::new(), Vec::new())?;
        file.file.flags = file.file.flags.with(flags);
        transaction.replace_file(&file)?;
    }
    let counted = transaction.queries().cochange_files()?;
    let paths: Vec<&str> = counted.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["src/a.py", "src/b.py"]);
    for file in &counted {
        assert_eq!(transaction.queries().file_node(file.file)?, Some(file.node));
    }
    transaction.commit()?;
    Ok(())
}

/// Removing every co-change edge leaves the edges of other types.
#[test]
fn co_change_edges_go_and_others_stay() -> TestResult {
    let scratch = Scratch::new()?;
    indexed(&scratch, &["a.py", "b.py"])?;
    let mut writer = scratch.writer(1000)?;
    let mut transaction = writer.transaction()?;
    let nodes: Vec<NodeId> = transaction
        .queries()
        .cochange_files()?
        .into_iter()
        .map(|file| file.node)
        .collect();
    let [a, b] = nodes[..] else {
        return Err("two nodes".into());
    };
    let edge = |dst| EdgeRecord {
        dst,
        weight: 0.5,
        count: 2,
        last_seq: 0,
        is_directed: true,
    };
    transaction.replace_edges(a, EdgeType::CoChange, &[edge(b)])?;
    transaction.replace_edges(b, EdgeType::CoChange, &[edge(a)])?;
    transaction.replace_edges(a, EdgeType::Structural, &[edge(b)])?;
    transaction.remove_edges_of_type(EdgeType::CoChange)?;
    let left: Vec<(NodeId, EdgeType)> = [a, b]
        .into_iter()
        .map(|node| transaction.queries().edges_from(node))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .map(|row| (row.src, row.edge_type))
        .collect();
    assert_eq!(left, [(a, EdgeType::Structural)]);
    transaction.commit()?;
    Ok(())
}
