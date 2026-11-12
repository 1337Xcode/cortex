//! Looking things up by name: rows per word, BM25 lookups, files by name, symbols by name.

mod common;

use common::{Scratch, TestResult};
use cortex_core::{ContentHash, FileFlags, Identifier, Language, RepoPath, ShortHash, SymbolKind};
use cortex_store::{FileIndex, FileRecord, NodeId, SymbolRecord, WriteTx};

/// Stores `path` with one function per name of `names`, each on a line of its own, and returns the
/// file's node and then each function's, in that order.
fn store(transaction: &mut WriteTx<'_>, path: &str, names: &[&str]) -> TestResult<Vec<NodeId>> {
    let path = RepoPath::new(path)?;
    let symbols = names
        .iter()
        .zip(1..)
        .map(|(name, line)| {
            Ok(SymbolRecord {
                parent: None,
                kind: SymbolKind::Function,
                name: Identifier::new(name)?,
                qual_name: Identifier::new(name)?,
                start_byte: u64::from(line) * 10,
                end_byte: u64::from(line) * 10 + 9,
                start_line: line,
                end_line: line,
                sig_hash: ShortHash::of(name.as_bytes()),
                body_hash: ShortHash::of(name.as_bytes()),
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    transaction.replace_file(&FileIndex {
        file: FileRecord {
            path: path.clone(),
            language: Some(Language::Python),
            size_bytes: 100,
            line_count: Some(10),
            content_hash: Some(ContentHash::of(path.as_str().as_bytes())),
            mtime_ns: 1,
            flags: FileFlags::PARSED,
            indexed_at_ms: 1,
        },
        symbols,
        refs: Vec::new(),
        passes_on: Vec::new(),
    })?;
    let queries = transaction.queries();
    let file = queries.file(&path)?.ok_or("a stored file")?.id;
    let mut nodes = vec![queries.file_node_at(&path)?.ok_or("a file node")?];
    nodes.extend(
        queries
            .symbol_spans_in_file(file)?
            .into_iter()
            .map(|span| span.node),
    );
    Ok(nodes)
}

/// Rows per word, counted by hand. `shop/orders.py` is `shop orders py`, its `create_order` is
/// `create order create order` (its name, then its qualified name) and `cancel_order` `cancel
/// order cancel order`; `shop/users.py` is `shop users py` and its `create_user` `create user
/// create user`. So `shop` and `py` are in 2 rows, `create` and `order` in 2, `cancel` in 1, and
/// `nothing` in none.
#[test]
fn rows_per_word_are_counted() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    store(
        &mut transaction,
        "shop/orders.py",
        &["create_order", "cancel_order"],
    )?;
    store(&mut transaction, "shop/users.py", &["create_user"])?;
    transaction.commit()?;
    let queries = writer.queries();
    for (word, rows) in [
        ("shop", 2),
        ("py", 2),
        ("create", 2),
        ("order", 2),
        ("cancel", 1),
        ("user", 1),
        ("nothing", 0),
    ] {
        assert_eq!(queries.term_rows(word)?, rows, "{word}");
    }
    Ok(())
}

/// A lookup finds the rows that hold any of its words, best first: `create_order`, which holds
/// both `create` and `order` twice, ranks above the rows that hold one of them. The limit keeps
/// the best; no word, or a limit of 0, finds nothing; a quote in a word changes nothing it asks.
#[test]
fn a_lookup_ranks_rows_by_bm25() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let orders = store(
        &mut transaction,
        "shop/orders.py",
        &["create_order", "cancel_order"],
    )?;
    let users = store(&mut transaction, "shop/users.py", &["create_user"])?;
    transaction.commit()?;
    let queries = writer.queries();
    let found = queries.search(&["create", "order"], 10)?;
    let nodes: Vec<NodeId> = found.iter().map(|(node, _)| *node).collect();
    assert_eq!(nodes.first(), Some(&orders[1]));
    let mut rest = nodes.get(1..).unwrap_or_default().to_vec();
    rest.sort();
    let mut expected = vec![orders[2], users[1]];
    expected.sort();
    assert_eq!(rest, expected);
    assert!(
        found
            .windows(2)
            .all(|pair| matches!(pair, [left, right] if left.1 >= right.1))
    );
    assert!(found.iter().all(|(_, score)| *score > 0.0));
    assert_eq!(queries.search(&["create", "order"], 1)?.len(), 1);
    assert_eq!(queries.search(&[], 10)?, []);
    assert_eq!(queries.search(&["create"], 0)?, []);
    assert_eq!(queries.search(&["create\" OR \"cancel"], 10)?, []);
    assert_eq!(queries.search(&["cancel OR create"], 10)?, []);
    Ok(())
}

/// Files by name: every file whose path is the name or ends in `/` and the name, by path. The
/// wildcards of `LIKE` are characters like any other here: `a_b.py` does not find `axb.py`, and
/// `%.py` finds nothing.
#[test]
fn files_are_found_by_name() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let orders = store(&mut transaction, "shop/orders.py", &[])?;
    let other = store(&mut transaction, "lib/shop/orders.py", &[])?;
    let underscored = store(&mut transaction, "a_b.py", &[])?;
    store(&mut transaction, "axb.py", &[])?;
    transaction.commit()?;
    let queries = writer.queries();
    let named: Vec<(String, NodeId)> = queries
        .files_named("orders.py")?
        .into_iter()
        .map(|(path, _, node)| (path.as_str().to_owned(), node))
        .collect();
    assert_eq!(
        named,
        [
            ("lib/shop/orders.py".to_owned(), other[0]),
            ("shop/orders.py".to_owned(), orders[0])
        ]
    );
    let exact: Vec<NodeId> = queries
        .files_named("a_b.py")?
        .into_iter()
        .map(|(_, _, node)| node)
        .collect();
    assert_eq!(exact, [underscored[0]]);
    assert_eq!(queries.files_named("%.py")?, []);
    Ok(())
}

/// Symbols by name, by node, up to the limit.
#[test]
fn symbols_are_found_by_name() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let first = store(&mut transaction, "a.py", &["run", "stop"])?;
    let second = store(&mut transaction, "b.py", &["run"])?;
    transaction.commit()?;
    let queries = writer.queries();
    let run = Identifier::new("run")?;
    assert_eq!(queries.symbol_nodes_named(&run, 5)?, [first[1], second[1]]);
    assert_eq!(queries.symbol_nodes_named(&run, 1)?, [first[1]]);
    assert_eq!(
        queries.symbol_nodes_named(&Identifier::new("walk")?, 5)?,
        []
    );
    Ok(())
}

/// Symbols whose name holds a text, ASCII letters in either case: `src/orders.py` defines `Order`
/// (line 1), `order_book` (2) and `reorder` (3), `src/billing.py` defines `OrderTotal` (1) and
/// `pay` (2). For `order`: the exact name `Order` first, then the names that start with it by
/// path, `OrderTotal` in billing.py before `order_book` in orders.py, then `reorder`, which holds
/// it; `pay` never. With a limit of 3 the last is cut. The `%` of a text is no wildcard.
#[test]
fn symbols_are_found_by_part_of_a_name() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    store(
        &mut transaction,
        "src/orders.py",
        &["Order", "order_book", "reorder"],
    )?;
    store(&mut transaction, "src/billing.py", &["OrderTotal", "pay"])?;
    transaction.commit()?;
    let queries = writer.queries();
    let found: Vec<(String, String, u32)> = queries
        .symbols_containing("order", 10)?
        .into_iter()
        .map(|(name, path, line)| (name, path.as_str().to_owned(), line))
        .collect();
    let expected = [
        ("Order", "src/orders.py", 1),
        ("OrderTotal", "src/billing.py", 1),
        ("order_book", "src/orders.py", 2),
        ("reorder", "src/orders.py", 3),
    ]
    .map(|(name, path, line)| (name.to_owned(), path.to_owned(), line));
    assert_eq!(found, expected);
    assert_eq!(queries.symbols_containing("order", 3)?.len(), 3);
    assert_eq!(queries.symbols_containing("%", 10)?, []);
    Ok(())
}
