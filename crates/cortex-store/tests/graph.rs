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
    // The counts the store reports are exactly what those two reads hand over, which is what lets
    // a reader take its room once rather than grow into it row by row.
    let counts = queries.counts()?;
    assert_eq!(
        (nodes.len(), edges.len()),
        (
            usize::try_from(counts.nodes)?,
            usize::try_from(counts.edges)?
        )
    );
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

/// The files that file nodes stand for read back by node, with their rows; a symbol's node (even
/// one whose row number is a file's), a folder's and a node asked for twice stand for nothing more. A file's symbol spans carry their
/// rows, the symbol around each, their kinds and names: `Order` (a class on lines 1 to 9) holds
/// `total` (a method on lines 2 to 4).
#[test]
fn files_of_nodes_and_symbol_spans_read_back() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&file_with("src/a.py", "f")?)?;
    let mut b = file_with("src/b.py", "Order")?;
    b.symbols = vec![
        SymbolRecord {
            kind: SymbolKind::Class,
            end_line: 9,
            ..b.symbols.first().ok_or("a symbol")?.clone()
        },
        SymbolRecord {
            parent: Some(0),
            kind: SymbolKind::Method,
            name: Identifier::new("total")?,
            qual_name: Identifier::new("Order.total")?,
            start_line: 2,
            end_line: 4,
            ..b.symbols.first().ok_or("a symbol")?.clone()
        },
    ];
    transaction.replace_file(&b)?;
    let folder = transaction.folder_node(&RepoPath::new("src")?)?;
    transaction.commit()?;
    let queries = writer.queries();
    let node = |path: &str| -> TestResult<cortex_store::NodeId> {
        Ok(queries
            .file_node_at(&RepoPath::new(path)?)?
            .ok_or("a file node")?)
    };
    let (a, b) = (node("src/a.py")?, node("src/b.py")?);
    let b_row = queries
        .file(&RepoPath::new("src/b.py")?)?
        .ok_or("b's row")?;
    let spans = queries.symbol_spans_in_file(b_row.id)?;
    let read: Vec<(SymbolKind, Option<cortex_store::SymbolId>, String, u32, u32)> = spans
        .iter()
        .map(|span| {
            (
                span.kind,
                span.parent,
                span.name.as_str().to_owned(),
                span.start_line,
                span.end_line,
            )
        })
        .collect();
    let class = spans.first().ok_or("the class")?.symbol;
    assert_eq!(
        read,
        [
            (SymbolKind::Class, None, "Order".to_owned(), 1, 9),
            (SymbolKind::Method, Some(class), "total".to_owned(), 2, 4),
        ]
    );
    let method = spans.get(1).ok_or("the method")?.node;
    // The class's row number is also `b.py`'s, so its node would name that file if symbols were
    // not told from files.
    assert_eq!(class.get(), b_row.id.get());
    let class_node = spans.first().ok_or("the class")?.node;
    let files = queries.files_of_nodes(&[b, method, class_node, folder, a, b])?;
    let paths: Vec<(cortex_store::NodeId, String)> = files
        .iter()
        .map(|(node, row)| (*node, row.file.path.as_str().to_owned()))
        .collect();
    let mut expected = vec![(a, "src/a.py".to_owned()), (b, "src/b.py".to_owned())];
    expected.sort();
    assert_eq!(paths, expected);
    assert_eq!(
        files
            .iter()
            .find(|(node, _)| *node == b)
            .map(|(_, row)| row),
        Some(&b_row)
    );
    assert!(queries.files_of_nodes(&[])?.is_empty());
    Ok(())
}
