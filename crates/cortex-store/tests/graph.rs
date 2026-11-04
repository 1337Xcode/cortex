//! Reading the whole graph: nodes with their kinds and files, every edge, the latest task.

mod common;

use common::{Scratch, TestResult};
use cortex_core::{
    ContentHash, EdgeType, FileFlags, Identifier, InterestTier, Language, NodeKind, RepoPath,
    ShortHash, SymbolKind, Ulid,
};
use cortex_store::{
    EdgeRecord, EdgeRow, FileIndex, FileRecord, GraphNodeRow, InterestRow, SessionRecord,
    SymbolRecord, TaskRecord,
};

/// `path`'s rows, with one function `name` on lines 1 and 2.
fn file_with(path: &str, name: &str) -> TestResult<FileIndex> {
    Ok(FileIndex {
        file: FileRecord {
            path: RepoPath::new(path)?,
            language: Some(Language::Python),
            size_bytes: 10,
            line_count: Some(2),
            content_hash: Some(ContentHash::of(path.as_bytes())),
            mtime_ns: 1,
            flags: FileFlags::PARSED,
            indexed_at_ms: 1,
        },
        symbols: vec![SymbolRecord {
            parent: None,
            kind: SymbolKind::Function,
            name: Identifier::new(name)?,
            qual_name: Identifier::new(name)?,
            start_byte: 0,
            end_byte: 10,
            start_line: 1,
            end_line: 2,
            sig_hash: ShortHash::of(name.as_bytes()),
            body_hash: ShortHash::of(name.as_bytes()),
        }],
        refs: Vec::new(),
        passes_on: Vec::new(),
    })
}

/// Two files with a function each and a folder: the graph's nodes come in node order, each with
/// its kind, a symbol with its file's node and nothing else with a file, the one node given
/// interest (`f`) with its row and the rest with none, each file with its flags and nothing else
/// with any; the edges come in source, target and type order; the latest task is 0 with no task,
/// then the number of the last one.
#[test]
fn the_whole_graph_reads_back() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&file_with("src/a.py", "f")?)?;
    transaction.replace_file(&file_with("src/b.py", "g")?)?;
    let folder = transaction.folder_node(&RepoPath::new("src")?)?;
    let queries = transaction.queries();
    let a = queries
        .file_node_at(&RepoPath::new("src/a.py")?)?
        .ok_or("a")?;
    let b = queries
        .file_node_at(&RepoPath::new("src/b.py")?)?
        .ok_or("b")?;
    let a_id = queries
        .file(&RepoPath::new("src/a.py")?)?
        .ok_or("a's row")?
        .id;
    let b_id = queries
        .file(&RepoPath::new("src/b.py")?)?
        .ok_or("b's row")?
        .id;
    let f = queries.symbol_spans_in_file(a_id)?.first().ok_or("f")?.node;
    let g = queries.symbol_spans_in_file(b_id)?.first().ok_or("g")?.node;
    let edge = |dst, weight, is_directed| EdgeRecord {
        dst,
        weight,
        count: 2,
        last_seq: 3,
        is_directed,
    };
    transaction.set_edges(EdgeType::Structural, &[(f, edge(g, 0.6, true))])?;
    transaction.set_edges(
        EdgeType::CoChange,
        &[(b, edge(a, 0.25, true)), (a, edge(b, 0.5, true))],
    )?;
    transaction.set_edges(EdgeType::Containment, &[(folder, edge(a, 1.0, false))])?;
    let interest = InterestRow {
        node: f,
        score: 0.7,
        last_seq: 3,
        tier: InterestTier::Interesting,
    };
    transaction.replace_interest(std::slice::from_ref(&interest))?;
    transaction.commit()?;
    let queries = writer.queries();
    let mut nodes = Vec::new();
    queries.each_graph_node(|row| {
        nodes.push(row);
        Ok(())
    })?;
    let mut expected = vec![
        GraphNodeRow {
            node: a,
            kind: NodeKind::File,
            file: None,
            interest: None,
            flags: Some(FileFlags::PARSED),
        },
        GraphNodeRow {
            node: f,
            kind: NodeKind::Symbol,
            file: Some(a),
            interest: Some(interest),
            flags: None,
        },
        GraphNodeRow {
            node: b,
            kind: NodeKind::File,
            file: None,
            interest: None,
            flags: Some(FileFlags::PARSED),
        },
        GraphNodeRow {
            node: g,
            kind: NodeKind::Symbol,
            file: Some(b),
            interest: None,
            flags: None,
        },
        GraphNodeRow {
            node: folder,
            kind: NodeKind::Dir,
            file: None,
            interest: None,
            flags: None,
        },
    ];
    expected.sort_by_key(|row| row.node);
    assert_eq!(nodes, expected);
    let mut edges: Vec<EdgeRow> = Vec::new();
    queries.each_edge(|row| {
        edges.push(row);
        Ok(())
    })?;
    let row = |src, edge_type, edge| EdgeRow {
        src,
        edge_type,
        edge,
    };
    let mut expected = vec![
        row(f, EdgeType::Structural, edge(g, 0.6, true)),
        row(b, EdgeType::CoChange, edge(a, 0.25, true)),
        row(a, EdgeType::CoChange, edge(b, 0.5, true)),
        row(folder, EdgeType::Containment, edge(a, 1.0, false)),
    ];
    expected.sort_by_key(|row| (row.src, row.edge.dst, row.edge_type));
    assert_eq!(edges, expected);
    assert_eq!(queries.latest_task_seq()?, 0);
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: Identifier::new("s1")?,
        agent: cortex_core::Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 1,
    })?;
    for at in 1..=3_u64 {
        transaction.start_task(&TaskRecord {
            task_id: Ulid::from_seed(at, b"task"),
            session_id: Identifier::new("s1")?,
            started_at_ms: i64::try_from(at)?,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: None,
        })?;
    }
    transaction.commit()?;
    assert_eq!(writer.queries().latest_task_seq()?, 3);
    Ok(())
}

/// A read stops at the first error the caller returns, and gives it back.
#[test]
fn a_read_stops_at_the_caller_s_error() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&file_with("a.py", "f")?)?;
    transaction.commit()?;
    let mut seen = 0;
    let read = writer.queries().each_graph_node(|_| {
        seen += 1;
        Err(cortex_store::StoreError::Corrupt {
            column: "test",
            row: 0,
        })
    });
    assert!(matches!(
        read,
        Err(cortex_store::StoreError::Corrupt { column: "test", .. })
    ));
    assert_eq!(seen, 1);
    Ok(())
}
