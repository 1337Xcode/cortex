//! The writer: one per database, what it records about a file, how it keeps symbol ids across
//! edits, references, edges, and what it refuses.

mod common;

use common::{Scratch, TestResult, index, name, path, reference, symbol};
use cortex_core::{EdgeType, Language, RefKind, RepoId, Resolution, SymbolKind};
use cortex_store::{
    ChangeKind, EdgeRecord, EdgeRow, IndexCounts, MetaKey, RefName, Resolved, StoreError,
    SymbolChange, SymbolName, SymbolRecord, Target,
};

/// The version the workspace manifest declares; a version bump updates this literal on purpose.
const CORTEX_VERSION: &str = "2.0.0-dev";

/// The first version of `src/parser.py`: a class with a method, two functions, an import at the
/// top and a call from the method to `helper` on line 5.
fn parser_v1() -> TestResult<cortex_store::FileIndex> {
    index(
        "src/parser.py",
        vec![
            symbol(SymbolKind::Class, "Parser", None, ("Parser", "Parser"))?,
            symbol(
                SymbolKind::Method,
                "Parser.parse",
                Some(0),
                ("parse", "parse"),
            )?,
            symbol(SymbolKind::Function, "helper", None, ("helper", "helper"))?,
            symbol(SymbolKind::Function, "gone", None, ("gone", "gone"))?,
        ],
        vec![
            reference(Some(1), "helper", RefKind::Call, 5)?,
            reference(None, "os", RefKind::Import, 1)?,
        ],
    )
}

/// The counts of an index holding the given numbers of rows.
fn counts(files: u64, symbols: u64, refs: u64, resolved_refs: u64, nodes: u64) -> IndexCounts {
    IndexCounts {
        files,
        symbols,
        refs,
        resolved_refs,
        nodes,
        edges: 0,
    }
}

/// While a writer is open no second one can open the database; once it closes, the next one can.
#[test]
fn a_second_writer_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let first = scratch.writer(1)?;
    let refused = scratch.writer(2).err();
    let lock = scratch.folder.join("writer.lock");
    assert!(
        matches!(&refused, Some(StoreError::WriterBusy { lock: held }) if *held == lock),
        "{refused:?}"
    );
    drop(first);
    scratch.writer(3)?;
    Ok(())
}

/// A new database records its schema, repository, Cortex version and creation time; reopening
/// it later keeps the creation time. A new database needs no backup.
#[test]
fn identity_is_recorded() -> TestResult {
    let scratch = Scratch::new()?;
    let writer = scratch.writer(1000)?;
    let expected_id = RepoId::of(scratch.root.path());
    let queries = writer.queries();
    assert_eq!(queries.meta(MetaKey::SchemaVersion)?.as_deref(), Some("1"));
    assert_eq!(
        queries.meta(MetaKey::RepoId)?.as_deref(),
        Some(expected_id.as_str())
    );
    assert_eq!(
        queries.meta(MetaKey::CortexVersion)?.as_deref(),
        Some(CORTEX_VERSION)
    );
    assert_eq!(queries.meta(MetaKey::CreatedAt)?.as_deref(), Some("1000"));
    assert_eq!(queries.meta(MetaKey::ScaleMode)?, None);
    drop(writer);
    let writer = scratch.writer(2000)?;
    assert_eq!(
        writer.queries().meta(MetaKey::CreatedAt)?.as_deref(),
        Some("1000")
    );
    assert!(!scratch.folder.join("backups").try_exists()?);
    Ok(())
}

/// A file's row, symbols, references and nodes are written as given and read back the same.
#[test]
fn a_file_is_written_with_its_symbols_and_references() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let v1 = parser_v1()?;
    let mut transaction = writer.transaction()?;
    let change = transaction.replace_file(&v1)?;
    transaction.commit()?;

    assert!(change.is_new);
    assert_eq!(change.symbols.len(), 4);
    assert_eq!(change.symbol_nodes.len(), 4);
    assert_eq!(change.refs.len(), 2);
    let added: Vec<(&str, ChangeKind)> = change
        .changes
        .iter()
        .map(|change| (change.name.as_str(), change.kind))
        .collect();
    assert_eq!(
        added,
        [
            ("Parser", ChangeKind::Added),
            ("parse", ChangeKind::Added),
            ("helper", ChangeKind::Added),
            ("gone", ChangeKind::Added),
        ]
    );

    let reader = scratch.reader()?;
    let queries = reader.queries();
    let row = queries
        .file(&path("src/parser.py")?)?
        .ok_or("file missing")?;
    assert_eq!(row.id, change.file);
    assert_eq!(row.file, v1.file);

    let symbols = queries.symbols_in_file(change.file)?;
    let shapes: Vec<(SymbolKind, &str, &str)> = symbols
        .iter()
        .map(|symbol| (symbol.kind, symbol.name.as_str(), symbol.qual_name.as_str()))
        .collect();
    assert_eq!(
        shapes,
        [
            (SymbolKind::Class, "Parser", "Parser"),
            (SymbolKind::Method, "parse", "Parser.parse"),
            (SymbolKind::Function, "helper", "helper"),
            (SymbolKind::Function, "gone", "gone"),
        ]
    );
    let ids: Vec<_> = symbols.iter().map(|symbol| symbol.id).collect();
    assert_eq!(ids, change.symbols);
    let parents: Vec<_> = symbols.iter().map(|symbol| symbol.parent).collect();
    assert_eq!(parents, [None, Some(change.symbols[0]), None, None]);
    assert_eq!(symbols[1].sig_hash, v1.symbols[1].sig_hash);
    assert_eq!(symbols[1].body_hash, v1.symbols[1].body_hash);
    assert_eq!(
        (
            symbols[1].start_byte,
            symbols[1].end_byte,
            symbols[1].start_line,
            symbols[1].end_line
        ),
        (0, 10, 1, 2)
    );

    let refs = queries.refs_in_file(change.file)?;
    let shapes: Vec<_> = refs
        .iter()
        .map(|reference| {
            (
                reference.symbol,
                reference.dst_name.as_str(),
                reference.kind,
                reference.line,
                reference.target,
                reference.confidence,
            )
        })
        .collect();
    assert_eq!(
        shapes,
        [
            (
                Some(change.symbols[1]),
                "helper",
                RefKind::Call,
                5,
                None,
                0.0
            ),
            (None, "os", RefKind::Import, 1, None, 0.0),
        ]
    );
    let ref_ids: Vec<_> = refs.iter().map(|reference| reference.id).collect();
    assert_eq!(ref_ids, change.refs);

    assert_eq!(queries.file_node(change.file)?, Some(change.file_node));
    for (symbol, node) in change.symbols.iter().zip(&change.symbol_nodes) {
        assert_eq!(queries.symbol_node(*symbol)?, Some(*node));
    }
    // One file node and four symbol nodes.
    assert_eq!(queries.counts()?, counts(1, 4, 2, 0, 5));
    Ok(())
}

/// Writing a file again keeps the ids and nodes of symbols with the same qualified name and kind,
/// and reports what changed: a body edit, a header edit, a new symbol and a removed one.
#[test]
fn rewriting_a_file_keeps_matching_symbols() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let first = transaction.replace_file(&parser_v1()?)?;
    let v2 = index(
        "src/parser.py",
        vec![
            symbol(SymbolKind::Class, "Parser", None, ("Parser", "Parser"))?,
            symbol(
                SymbolKind::Method,
                "Parser.parse",
                Some(0),
                ("parse", "parse 2"),
            )?,
            symbol(
                SymbolKind::Function,
                "helper",
                None,
                ("helper 2", "helper 2"),
            )?,
            symbol(SymbolKind::Function, "extra", None, ("extra", "extra"))?,
        ],
        vec![reference(Some(1), "extra", RefKind::Call, 6)?],
    )?;
    let second = transaction.replace_file(&v2)?;
    transaction.commit()?;

    assert!(!second.is_new);
    assert_eq!(second.file, first.file);
    assert_eq!(second.file_node, first.file_node);
    assert_eq!(second.symbols[..3], first.symbols[..3]);
    assert_eq!(second.symbol_nodes[..3], first.symbol_nodes[..3]);
    assert!(!first.symbols.contains(&second.symbols[3]));
    assert_eq!(
        second.changes,
        [
            SymbolChange {
                symbol: first.symbols[1],
                name: name("parse")?,
                kind: ChangeKind::Body,
            },
            SymbolChange {
                symbol: first.symbols[2],
                name: name("helper")?,
                kind: ChangeKind::Signature,
            },
            SymbolChange {
                symbol: second.symbols[3],
                name: name("extra")?,
                kind: ChangeKind::Added,
            },
            SymbolChange {
                symbol: first.symbols[3],
                name: name("gone")?,
                kind: ChangeKind::Removed,
            },
        ]
    );
    let queries = writer.queries();
    assert_eq!(queries.symbol_node(first.symbols[3])?, None);
    let refs = queries.refs_in_file(second.file)?;
    let names: Vec<&str> = refs
        .iter()
        .map(|reference| reference.dst_name.as_str())
        .collect();
    assert_eq!(names, ["extra"]);
    assert_eq!(queries.counts()?, counts(1, 4, 1, 0, 5));
    Ok(())
}

/// Symbols that share a qualified name and kind, such as overloads, are matched in order: the
/// second keeps the second's id, and dropping one removes the last.
#[test]
fn same_named_symbols_match_in_order() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let overloads = |second_body: &str| -> TestResult<cortex_store::FileIndex> {
        index(
            "src/f.py",
            vec![
                symbol(SymbolKind::Function, "f", None, ("first", "first"))?,
                symbol(SymbolKind::Function, "f", None, ("second", second_body))?,
            ],
            Vec::new(),
        )
    };
    let first = transaction.replace_file(&overloads("second")?)?;
    let second = transaction.replace_file(&overloads("second, edited")?)?;
    assert_eq!(second.symbols, first.symbols);
    let kinds: Vec<_> = second
        .changes
        .iter()
        .map(|change| (change.symbol, change.kind))
        .collect();
    assert_eq!(kinds, [(first.symbols[1], ChangeKind::Body)]);
    let one = index(
        "src/f.py",
        vec![symbol(SymbolKind::Function, "f", None, ("first", "first"))?],
        Vec::new(),
    )?;
    let third = transaction.replace_file(&one)?;
    let kinds: Vec<_> = third
        .changes
        .iter()
        .map(|change| (change.symbol, change.kind))
        .collect();
    assert_eq!(kinds, [(first.symbols[1], ChangeKind::Removed)]);
    Ok(())
}

/// A node id is never given out twice, even after the node with the highest id is deleted, so
/// what was learned about a deleted node cannot attach to a new one.
#[test]
fn node_ids_are_never_reused() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let with = |qual_name: &str| -> TestResult<cortex_store::FileIndex> {
        index(
            "src/a.py",
            vec![symbol(SymbolKind::Function, qual_name, None, ("s", "b"))?],
            Vec::new(),
        )
    };
    let old = transaction.replace_file(&with("old")?)?;
    transaction.replace_file(&index("src/a.py", Vec::new(), Vec::new())?)?;
    let new = transaction.replace_file(&with("new")?)?;
    assert!(new.symbol_nodes[0] > old.symbol_nodes[0]);
    Ok(())
}

/// Every symbol's and every reference's name is listed by id, with its file and that file's
/// language. A symbol says whether it is defined directly in its file: `helper` and `main` are;
/// `main.inner` sits in `main`, and `Order.new` carries the qualifier `Order` with no parent, as a
/// method in an `impl` block does. A reference comes with its kind, its line and what it is tied
/// to.
#[test]
fn names_are_listed_with_their_files() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let a = transaction.replace_file(&index(
        "src/a.py",
        vec![
            symbol(SymbolKind::Function, "helper", None, ("s", "b"))?,
            symbol(SymbolKind::Function, "main", None, ("s", "b"))?,
            symbol(SymbolKind::Function, "main.inner", Some(1), ("s", "b"))?,
            symbol(SymbolKind::Method, "Order.new", None, ("s", "b"))?,
        ],
        vec![
            reference(None, "os", RefKind::Import, 1)?,
            reference(Some(1), "helper", RefKind::Call, 5)?,
            reference(Some(1), "print", RefKind::Call, 6)?,
        ],
    )?)?;
    let target = Target {
        symbol: a.symbols[0],
        resolution: Resolution::SameFile,
    };
    transaction.set_resolutions(&[Resolved {
        reference: a.refs[1],
        target: Some(target),
    }])?;
    transaction.commit()?;
    let queries = writer.queries();
    let python = Some(Language::Python);
    let symbol_name = |position: usize, text: &str, is_top_level| -> TestResult<SymbolName> {
        Ok(SymbolName {
            id: a.symbols[position],
            file: a.file,
            language: python,
            name: name(text)?,
            is_top_level,
        })
    };
    assert_eq!(
        queries.symbol_names()?,
        [
            symbol_name(0, "helper", true)?,
            symbol_name(1, "main", true)?,
            symbol_name(2, "inner", false)?,
            symbol_name(3, "new", false)?,
        ]
    );
    let ref_name = |position: usize, text: &str, kind, line, target| -> TestResult<RefName> {
        Ok(RefName {
            id: a.refs[position],
            file: a.file,
            language: python,
            name: name(text)?,
            kind,
            line,
            target,
        })
    };
    assert_eq!(
        queries.ref_names()?,
        [
            ref_name(0, "os", RefKind::Import, 1, None)?,
            ref_name(1, "helper", RefKind::Call, 5, Some(target))?,
            ref_name(2, "print", RefKind::Call, 6, None)?,
        ]
    );
    Ok(())
}

/// Resolving a reference stores its target and its resolution's confidence; removing the target
/// leaves the reference unresolved again.
#[test]
fn references_resolve_and_unresolve() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let target = transaction.replace_file(&index(
        "src/a.py",
        vec![symbol(SymbolKind::Function, "helper", None, ("s", "b"))?],
        Vec::new(),
    )?)?;
    let user = transaction.replace_file(&index(
        "src/b.py",
        vec![symbol(SymbolKind::Function, "main", None, ("s", "b"))?],
        vec![
            reference(Some(0), "helper", RefKind::Call, 3)?,
            reference(Some(0), "helper", RefKind::Call, 4)?,
            reference(Some(0), "helper", RefKind::Call, 5)?,
            reference(Some(0), "helper", RefKind::Call, 6)?,
        ],
    )?)?;
    let resolutions = [
        Resolution::Scip,
        Resolution::ImportPath,
        Resolution::SameFile,
        Resolution::NameOnly,
    ];
    let resolved: Vec<Resolved> = user
        .refs
        .iter()
        .zip(resolutions)
        .map(|(reference, resolution)| Resolved {
            reference: *reference,
            target: Some(Target {
                symbol: target.symbols[0],
                resolution,
            }),
        })
        .collect();
    transaction.set_resolutions(&resolved)?;
    transaction.commit()?;

    let reader = scratch.reader()?;
    let refs = reader.queries().refs_in_file(user.file)?;
    let stored: Vec<_> = refs
        .iter()
        .map(|reference| {
            (
                reference.target.map(|target| target.resolution),
                reference.confidence,
            )
        })
        .collect();
    assert_eq!(
        stored,
        [
            (Some(Resolution::Scip), 1.0),
            (Some(Resolution::ImportPath), 0.8),
            (Some(Resolution::SameFile), 0.6),
            (Some(Resolution::NameOnly), 0.3),
        ]
    );
    assert_eq!(reader.queries().counts()?.resolved_refs, 4);

    let mut transaction = writer.transaction()?;
    transaction.replace_file(&index("src/a.py", Vec::new(), Vec::new())?)?;
    transaction.commit()?;
    let refs = reader.queries().refs_in_file(user.file)?;
    let stored: Vec<_> = refs
        .iter()
        .map(|reference| (reference.target, reference.confidence))
        .collect();
    assert_eq!(stored, [(None, 0.0); 4]);
    assert_eq!(reader.queries().counts()?.resolved_refs, 0);
    Ok(())
}

/// Resolving a reference that does not exist is refused, naming it.
#[test]
fn resolving_a_missing_reference_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let change = transaction.replace_file(&index(
        "src/c.py",
        Vec::new(),
        vec![reference(None, "os", RefKind::Import, 1)?],
    )?)?;
    transaction.remove_file(&path("src/c.py")?)?;
    let error = transaction
        .set_resolutions(&[Resolved {
            reference: change.refs[0],
            target: None,
        }])
        .err();
    let expected = change.refs[0].get();
    assert!(
        matches!(error, Some(StoreError::UnknownRow { table: "refs", id }) if id == expected),
        "{error:?}"
    );
    Ok(())
}

/// Removing a file deletes its row, symbols, references, nodes and every edge at its nodes, and
/// leaves references to its symbols unresolved. Removing it again finds nothing.
#[test]
fn removing_a_file_deletes_everything_about_it() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let target = transaction.replace_file(&index(
        "src/a.py",
        vec![symbol(SymbolKind::Function, "helper", None, ("s", "b"))?],
        Vec::new(),
    )?)?;
    let user = transaction.replace_file(&index(
        "src/b.py",
        vec![symbol(SymbolKind::Function, "main", None, ("s", "b"))?],
        vec![reference(Some(0), "helper", RefKind::Call, 3)?],
    )?)?;
    transaction.set_resolutions(&[Resolved {
        reference: user.refs[0],
        target: Some(Target {
            symbol: target.symbols[0],
            resolution: Resolution::NameOnly,
        }),
    }])?;
    let edge = |dst| EdgeRecord {
        dst,
        weight: 0.3,
        count: 1,
        last_seq: 0,
        is_directed: true,
    };
    transaction.replace_edges(
        user.file_node,
        EdgeType::Structural,
        &[edge(target.file_node)],
    )?;
    transaction.replace_edges(
        target.file_node,
        EdgeType::CoChange,
        &[edge(user.file_node)],
    )?;
    let removal = transaction
        .remove_file(&path("src/a.py")?)?
        .ok_or("nothing removed")?;
    assert_eq!(removal.file, target.file);
    assert_eq!(
        removal.removed,
        [SymbolChange {
            symbol: target.symbols[0],
            name: name("helper")?,
            kind: ChangeKind::Removed,
        }]
    );
    assert_eq!(transaction.remove_file(&path("src/a.py")?)?, None);
    transaction.commit()?;

    let queries = writer.queries();
    assert_eq!(queries.file(&path("src/a.py")?)?, None);
    assert_eq!(queries.file_node(target.file)?, None);
    assert_eq!(queries.edges_from(user.file_node)?, Vec::<EdgeRow>::new());
    assert_eq!(queries.refs_in_file(user.file)?[0].target, None);
    // What is left is b.py: one file, one symbol, one unresolved reference, two nodes.
    assert_eq!(queries.counts()?, counts(1, 1, 1, 0, 2));
    Ok(())
}

/// A folder gets one node, found again by its path, and listed with the others by path; removing
/// it takes the edges at it, finds nothing the second time, and its id is never given again. Files
/// are listed with their nodes, by path.
#[test]
fn folders_have_nodes_of_their_own() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let file = transaction.replace_file(&index("src/a.py", Vec::new(), Vec::new())?)?;
    let src = transaction.folder_node(&path("src")?)?;
    let root = transaction.folder_node(&path("")?)?;
    assert_eq!(transaction.folder_node(&path("src")?)?, src);
    assert_ne!(root, src);
    let edge = EdgeRecord {
        dst: file.file_node,
        weight: 1.0,
        count: 1,
        last_seq: 0,
        is_directed: false,
    };
    transaction.replace_edges(src, EdgeType::Containment, &[edge])?;
    let queries = transaction.queries();
    assert_eq!(
        queries.folder_nodes()?,
        [(path("")?, root), (path("src")?, src)]
    );
    assert_eq!(queries.file_nodes()?, [(path("src/a.py")?, file.file_node)]);
    assert_eq!(queries.edges_from(src)?.len(), 1);
    assert!(transaction.remove_folder_node(&path("src")?)?);
    assert!(!transaction.remove_folder_node(&path("src")?)?);
    let again = transaction.folder_node(&path("src")?)?;
    transaction.commit()?;

    let queries = writer.queries();
    assert_eq!(queries.edges_from(src)?, Vec::<EdgeRow>::new());
    assert!(again > src);
    assert_eq!(
        queries.folder_nodes()?,
        [(path("")?, root), (path("src")?, again)]
    );
    // a.py's file node and the two folders.
    assert_eq!(queries.counts()?, counts(1, 0, 0, 0, 3));
    Ok(())
}

/// A node's search row holds the tokens of its path (a file or folder) or of its name and
/// qualified name (a symbol), split into lower-case words, and goes with the node; the root
/// folder, with no path, has none. The rows are searchable by any of their words.
#[test]
fn search_rows_follow_their_nodes() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let class = symbol(SymbolKind::Class, "OrderLine", None, ("s", "b"))?;
    let method = symbol(SymbolKind::Method, "OrderLine.addItem", Some(0), ("s", "b"))?;
    let file = transaction.replace_file(&index(
        "src/shop/order_lines.py",
        vec![class.clone(), method],
        Vec::new(),
    )?)?;
    let folder = transaction.folder_node(&path("src/shop")?)?;
    let root = transaction.folder_node(&path("")?)?;
    let (class_node, method_node) = (file.symbol_nodes[0], file.symbol_nodes[1]);
    let queries = transaction.queries();
    let tokens = |node| queries.search_tokens(node);
    assert_eq!(
        tokens(file.file_node)?.as_deref(),
        Some("src shop order lines py")
    );
    assert_eq!(
        tokens(class_node)?.as_deref(),
        Some("order line order line")
    );
    assert_eq!(
        tokens(method_node)?.as_deref(),
        Some("add item order line add item")
    );
    assert_eq!(tokens(folder)?.as_deref(), Some("src shop"));
    assert_eq!(tokens(root)?, None);
    transaction.commit()?;
    drop(writer);

    let matching = |word: &str| -> TestResult<Vec<i64>> {
        let connection = rusqlite::Connection::open(scratch.folder.join("cortex.db"))?;
        let mut statement = connection
            .prepare("SELECT rowid FROM search_index WHERE search_index MATCH ?1 ORDER BY rowid")?;
        let rows = statement.query_map([word], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
    };
    assert_eq!(matching("item")?, [method_node.get()]);
    assert_eq!(matching("shop")?, [file.file_node.get(), folder.get()]);

    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    // The class keeps its id across the rewrite (same qualified name and kind) under a new name,
    // and its row follows the name; the method is gone, and so is its row.
    let renamed = SymbolRecord {
        name: name("Line")?,
        ..class
    };
    transaction.replace_file(&index(
        "src/shop/order_lines.py",
        vec![renamed],
        Vec::new(),
    )?)?;
    let queries = transaction.queries();
    assert_eq!(
        queries.search_tokens(class_node)?.as_deref(),
        Some("line order line")
    );
    assert_eq!(queries.search_tokens(method_node)?, None);
    // A failed write leaves no row behind: the parent position is past the file's symbols.
    let orphan = symbol(SymbolKind::Method, "Lost.method", Some(5), ("s", "b"))?;
    let refused = transaction.replace_file(&index("src/orphan.py", vec![orphan], Vec::new())?);
    assert!(refused.is_err());
    // Words outside ASCII are kept, a decomposed accent with its letter.
    let accented =
        transaction.replace_file(&index("docs/cafe\u{301}s.py", Vec::new(), Vec::new())?)?;
    assert_eq!(
        transaction
            .queries()
            .search_tokens(accented.file_node)?
            .as_deref(),
        Some("docs cafe\u{301}s py")
    );
    transaction.remove_file(&path("src/shop/order_lines.py")?)?;
    transaction.remove_folder_node(&path("src/shop")?)?;
    let queries = transaction.queries();
    for node in [file.file_node, class_node, folder] {
        assert_eq!(queries.search_tokens(node)?, None, "{node:?}");
    }
    transaction.commit()?;
    drop(writer);
    assert_eq!(matching("order")?, Vec::<i64>::new());
    assert_eq!(matching("orphan")?, Vec::<i64>::new());
    Ok(())
}

/// A folder node that has lost its search row, as one written before folders had rows would have,
/// gets it back the next time its node is asked for.
#[test]
fn a_folder_gets_its_search_row_back() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let folder = transaction.folder_node(&path("src/shop")?)?;
    transaction.commit()?;
    drop(writer);
    rusqlite::Connection::open(scratch.folder.join("cortex.db"))?
        .execute("DELETE FROM search_index WHERE rowid = ?1", [folder.get()])?;
    let mut writer = scratch.writer(1)?;
    assert_eq!(writer.queries().search_tokens(folder)?, None);
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.folder_node(&path("src/shop")?)?, folder);
    let tokens = transaction.queries().search_tokens(folder)?;
    assert_eq!(tokens.as_deref(), Some("src shop"));
    Ok(())
}

/// Every stored path is listed in byte order, as paths sort in Rust: capitals before small
/// letters, and `-` (0x2D) before `/` (0x2F), so `a-b.py` comes before the folder `a`. A removed
/// file is no longer listed.
#[test]
fn stored_paths_are_listed_in_byte_order() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let mut ids = std::collections::BTreeMap::new();
    for written in ["b.py", "a/x.py", "gone.py", "B.py", "a-b.py"] {
        let file = transaction.replace_file(&index(written, Vec::new(), Vec::new())?)?;
        ids.insert(written, file.file);
    }
    transaction.remove_file(&path("gone.py")?)?;
    transaction.commit()?;
    let order = ["B.py", "a-b.py", "a/x.py", "b.py"];
    let expected = order
        .into_iter()
        .map(path)
        .collect::<TestResult<Vec<_>>>()?;
    let reader = scratch.reader()?;
    let queries = reader.queries();
    assert_eq!(queries.file_paths()?, expected);
    // The same order with each file's row, as written.
    let with_ids = order
        .into_iter()
        .map(|written| Ok((path(written)?, ids[written])))
        .collect::<TestResult<Vec<_>>>()?;
    assert_eq!(queries.file_ids()?, with_ids);
    Ok(())
}

/// Replacing a node's edges of one type leaves its other types alone; edges read back by type,
/// then target.
#[test]
fn edges_are_replaced_one_type_at_a_time() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let a = transaction
        .replace_file(&index("a.py", Vec::new(), Vec::new())?)?
        .file_node;
    let b = transaction
        .replace_file(&index("b.py", Vec::new(), Vec::new())?)?
        .file_node;
    let c = transaction
        .replace_file(&index("c.py", Vec::new(), Vec::new())?)?
        .file_node;
    let structural_b = EdgeRecord {
        dst: b,
        weight: 0.8,
        count: 2,
        last_seq: 0,
        is_directed: true,
    };
    let structural_c = EdgeRecord {
        dst: c,
        weight: 0.3,
        count: 1,
        last_seq: 0,
        is_directed: true,
    };
    let cochange_b = EdgeRecord {
        dst: b,
        weight: 0.5,
        count: 4,
        last_seq: 7,
        is_directed: false,
    };
    transaction.replace_edges(a, EdgeType::CoChange, &[cochange_b])?;
    transaction.replace_edges(a, EdgeType::Structural, &[structural_c, structural_b])?;
    let row = |edge_type, edge| EdgeRow {
        src: a,
        edge_type,
        edge,
    };
    assert_eq!(
        transaction.queries().edges_from(a)?,
        [
            row(EdgeType::Structural, structural_b),
            row(EdgeType::Structural, structural_c),
            row(EdgeType::CoChange, cochange_b),
        ]
    );
    let replacement = EdgeRecord {
        weight: 0.6,
        ..structural_c
    };
    transaction.replace_edges(a, EdgeType::Structural, &[replacement])?;
    assert_eq!(
        transaction.queries().edges_from(a)?,
        [
            row(EdgeType::Structural, replacement),
            row(EdgeType::CoChange, cochange_b),
        ]
    );
    Ok(())
}

/// A symbol whose parent does not come before it, or a reference inside a symbol the file does not
/// have, is refused with its file and positions. A refused write leaves nothing behind, and the
/// transaction keeps what was written before it.
#[test]
fn positions_outside_the_file_are_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&index(
        "src/good.py",
        vec![symbol(SymbolKind::Function, "ok", None, ("s", "b"))?],
        Vec::new(),
    )?)?;
    let cases = [
        (
            index(
                "src/x.py",
                vec![
                    symbol(SymbolKind::Method, "A.m", Some(1), ("s", "b"))?,
                    symbol(SymbolKind::Class, "A", None, ("s", "b"))?,
                ],
                Vec::new(),
            )?,
            ("symbol", 0, 1),
        ),
        (
            index(
                "src/x.py",
                vec![symbol(SymbolKind::Class, "A", Some(0), ("s", "b"))?],
                Vec::new(),
            )?,
            ("symbol", 0, 0),
        ),
        (
            index(
                "src/x.py",
                vec![symbol(SymbolKind::Class, "A", None, ("s", "b"))?],
                vec![
                    reference(Some(0), "f", RefKind::Call, 2)?,
                    reference(Some(1), "g", RefKind::Call, 3)?,
                ],
            )?,
            ("reference", 1, 1),
        ),
    ];
    for (file_index, (what, at, points_at)) in cases {
        let error = transaction.replace_file(&file_index).err();
        assert!(
            matches!(
                &error,
                Some(StoreError::Position { path, what: found, index, position })
                    if path.as_str() == "src/x.py" && *found == what && *index == at
                        && *position == points_at
            ),
            "{error:?}"
        );
    }
    transaction.commit()?;
    let queries = writer.queries();
    assert_eq!(queries.file(&path("src/x.py")?)?, None);
    // Only src/good.py: one file, one symbol, no references, two nodes.
    assert_eq!(queries.counts()?, counts(1, 1, 0, 0, 2));
    Ok(())
}

/// Nothing a transaction wrote is kept when it is dropped without a commit.
#[test]
fn an_uncommitted_transaction_leaves_nothing() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&parser_v1()?)?;
    drop(transaction);
    assert_eq!(writer.queries().counts()?, counts(0, 0, 0, 0, 0));
    Ok(())
}

/// The kind of a SQLite failure, if `error` is one.
fn sqlite_code(error: Option<&StoreError>) -> Option<rusqlite::ErrorCode> {
    match error {
        Some(StoreError::Sqlite {
            source: rusqlite::Error::SqliteFailure(failure, _),
            ..
        }) => Some(failure.code),
        _ => None,
    }
}

/// An id kept from before a symbol was deleted never names a newer symbol, and resolving a
/// reference to it is refused: the reference stays unresolved instead of pointing at unrelated
/// code.
#[test]
fn a_stale_symbol_id_resolves_nothing() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let helper = transaction.replace_file(&index(
        "src/a.py",
        vec![symbol(SymbolKind::Function, "helper", None, ("s", "b"))?],
        Vec::new(),
    )?)?;
    let user = transaction.replace_file(&index(
        "src/b.py",
        Vec::new(),
        vec![reference(None, "helper", RefKind::Call, 3)?],
    )?)?;
    transaction.remove_file(&path("src/a.py")?)?;
    let unrelated = transaction.replace_file(&index(
        "src/c.py",
        vec![symbol(SymbolKind::Function, "unrelated", None, ("s", "b"))?],
        Vec::new(),
    )?)?;
    assert_ne!(unrelated.symbols[0], helper.symbols[0]);
    let error = transaction
        .set_resolutions(&[Resolved {
            reference: user.refs[0],
            target: Some(Target {
                symbol: helper.symbols[0],
                resolution: Resolution::Scip,
            }),
        }])
        .err();
    assert_eq!(
        sqlite_code(error.as_ref()),
        Some(rusqlite::ErrorCode::ConstraintViolation),
        "{error:?}"
    );
    transaction.commit()?;
    let refs = writer.queries().refs_in_file(user.file)?;
    assert_eq!((refs[0].target, refs[0].confidence), (None, 0.0));
    Ok(())
}

/// When one resolution in a batch is refused, none of the batch is written.
#[test]
fn a_refused_resolution_writes_none_of_its_batch() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let target = transaction.replace_file(&index(
        "src/a.py",
        vec![symbol(SymbolKind::Function, "helper", None, ("s", "b"))?],
        Vec::new(),
    )?)?;
    let user = transaction.replace_file(&index(
        "src/b.py",
        Vec::new(),
        vec![reference(None, "helper", RefKind::Call, 3)?],
    )?)?;
    let gone = transaction.replace_file(&index(
        "src/c.py",
        Vec::new(),
        vec![reference(None, "os", RefKind::Import, 1)?],
    )?)?;
    transaction.remove_file(&path("src/c.py")?)?;
    let resolved = Resolved {
        reference: user.refs[0],
        target: Some(Target {
            symbol: target.symbols[0],
            resolution: Resolution::SameFile,
        }),
    };
    let missing = Resolved {
        reference: gone.refs[0],
        target: None,
    };
    let error = transaction.set_resolutions(&[resolved, missing]).err();
    assert!(
        matches!(error, Some(StoreError::UnknownRow { table: "refs", .. })),
        "{error:?}"
    );
    transaction.commit()?;
    let refs = writer.queries().refs_in_file(user.file)?;
    assert_eq!(refs[0].target, None);
    Ok(())
}

/// An edge to a node that no longer exists is refused, so no edge outlives its nodes.
#[test]
fn an_edge_to_a_missing_node_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let kept = transaction.replace_file(&index("src/b.py", Vec::new(), Vec::new())?)?;
    let gone = transaction.replace_file(&index("src/a.py", Vec::new(), Vec::new())?)?;
    transaction.remove_file(&path("src/a.py")?)?;
    let edge = EdgeRecord {
        dst: gone.file_node,
        weight: 0.5,
        count: 1,
        last_seq: 0,
        is_directed: false,
    };
    let error = transaction
        .replace_edges(kept.file_node, EdgeType::CoChange, &[edge])
        .err();
    assert_eq!(
        sqlite_code(error.as_ref()),
        Some(rusqlite::ErrorCode::ConstraintViolation),
        "{error:?}"
    );
    transaction.commit()?;
    assert_eq!(writer.queries().counts()?.edges, 0);
    Ok(())
}

/// A database Cortex did not write is refused and left exactly as it was, journal mode included.
#[test]
fn a_foreign_database_is_left_alone() -> TestResult {
    let scratch = Scratch::new()?;
    std::fs::create_dir_all(&scratch.folder)?;
    let database = scratch.folder.join("cortex.db");
    let foreign = rusqlite::Connection::open(&database)?;
    foreign.execute_batch("CREATE TABLE notes (text TEXT); INSERT INTO notes VALUES ('kept');")?;
    drop(foreign);
    let error = scratch.writer(1).err();
    assert!(
        matches!(error, Some(StoreError::UnknownSchema)),
        "{error:?}"
    );
    let foreign = rusqlite::Connection::open(&database)?;
    let mode: String = foreign.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    assert_eq!(mode, "delete");
    let note: String = foreign.query_row("SELECT text FROM notes", [], |row| row.get(0))?;
    assert_eq!(note, "kept");
    Ok(())
}

/// The variable that makes [`hold_the_writer_for_another_test`] hold a writer: the `.cortex`
/// folder to open.
const HOLD_WRITER_VARIABLE: &str = "CORTEX_STORE_TEST_HOLD_WRITER";

/// The line the holding process prints once it has the writer.
const HOLDING_LINE: &str = "holding the writer";

/// Run as a child process by [`a_writer_in_another_process_is_refused`]: opens the writer, says so
/// on stdout and holds it until stdin closes. In an ordinary test run it does nothing.
#[test]
fn hold_the_writer_for_another_test() -> TestResult {
    use std::io::{Read, Write};
    let Some(folder) = std::env::var_os(HOLD_WRITER_VARIABLE) else {
        return Ok(());
    };
    let repo_id = RepoId::of(std::path::Path::new(&folder));
    let writer = cortex_store::Writer::open(&cortex_store::WriterOptions {
        folder: std::path::Path::new(&folder),
        repo_id: &repo_id,
        mmap_bytes: common::MMAP_BYTES,
        now_ms: 1,
    })?;
    writeln!(std::io::stdout(), "{HOLDING_LINE}")?;
    std::io::stdout().flush()?;
    let mut rest = Vec::new();
    std::io::stdin().read_to_end(&mut rest)?;
    drop(writer);
    Ok(())
}

/// A writer held by another process keeps this process from opening one, until that process lets
/// it go.
#[test]
fn a_writer_in_another_process_is_refused() -> TestResult {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    use std::time::Duration;
    let scratch = Scratch::new()?;
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", "hold_the_writer_for_another_test", "--nocapture"])
        .env(HOLD_WRITER_VARIABLE, &scratch.folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line == HOLDING_LINE && sender.send(()).is_err() {
                break;
            }
        }
    });
    receiver.recv_timeout(Duration::from_secs(60))?;
    let refused = scratch.writer(2).err();
    assert!(
        matches!(refused, Some(StoreError::WriterBusy { .. })),
        "{refused:?}"
    );
    drop(child.stdin.take());
    assert!(child.wait()?.success());
    scratch.writer(3)?;
    Ok(())
}
