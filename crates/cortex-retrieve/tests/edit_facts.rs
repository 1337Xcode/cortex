//! An edited file's facts as the store holds them, every value worked out by hand from the store
//! built here: its commits and co-change partners, the links faded to the latest task, the calls
//! to its changed symbol from files still as the index read them, and its tests.

mod common;

use common::{Def, Repo, TestResult, function};
use cortex_core::{
    ContentHash, EdgeType, FileFlags, Identifier, Language, RefKind, RepoPath, Resolution,
    SymbolKind,
};
use cortex_retrieve::{
    CallSite, Drift, EditFacts, EditQuery, Linked, NamedFile, Partner, TestFile, edit_facts,
};
use cortex_store::{FileIndex, FileRecord, NodeId, RefRecord, Resolved, SymbolId, Target};

/// The text of the callers: three lines and more, each the same length, so that the lines named
/// below exist.
const CALLER: &str = "import orders\n\norders.create_order(1)\n\norders.create_order(2)\n";

/// Stores the file at `path` holding `text`, written to disk, with a use of `kind` of `create_order`
/// on each of `lines`, each tied to `target` as `resolution` ties it; returns the file's node.
fn user(
    repo: &mut Repo,
    (path, text): (&str, &str),
    (lines, kind): (&[u32], RefKind),
    (target, resolution): (SymbolId, Resolution),
) -> TestResult<NodeId> {
    let node = *repo
        .file(path, text, &[], FileFlags::empty())?
        .first()
        .ok_or("a file node")?;
    let refs = lines
        .iter()
        .map(|line| -> TestResult<RefRecord> {
            Ok(RefRecord {
                symbol: None,
                dst_name: Identifier::new("create_order")?,
                kind,
                line: *line,
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    let mut transaction = repo.writer.transaction()?;
    let written = transaction.replace_file(&FileIndex {
        file: FileRecord {
            path: RepoPath::new(path)?,
            language: Some(Language::Python),
            size_bytes: u64::try_from(text.len())?,
            line_count: Some(cortex_index::count_lines(text.as_bytes())),
            content_hash: Some(ContentHash::of(text.as_bytes())),
            mtime_ns: 1,
            flags: FileFlags::PARSED,
            indexed_at_ms: 1,
        },
        symbols: Vec::new(),
        refs,
        passes_on: Vec::new(),
    })?;
    let tied: Vec<Resolved> = written
        .refs
        .iter()
        .map(|reference| Resolved {
            reference: *reference,
            target: Some(Target {
                symbol: target,
                resolution,
            }),
            goes_through_import: false,
        })
        .collect();
    transaction.set_resolutions(&tied)?;
    transaction.commit()?;
    Ok(node)
}

/// Stores the file at `path` as [`user`] does, with a call on each of `lines`.
fn caller(
    repo: &mut Repo,
    file: (&str, &str),
    lines: &[u32],
    target: (SymbolId, Resolution),
) -> TestResult<NodeId> {
    user(repo, file, (lines, RefKind::Call), target)
}

/// A named file.
fn named(node: NodeId, path: &str) -> TestResult<NamedFile> {
    Ok(NamedFile {
        node,
        path: RepoPath::new(path)?,
    })
}

/// The edit of `src/orders.py`, whose `create_order` changed its signature. By hand:
/// - 11 commits changed it; a co-change link to `prisma/schema.prisma` of confidence 0.73 from 8,
///   of the 10 commits that changed the schema: 8 / 10 = 0.8 seen from the schema.
/// - A co-access link to `src/billing.py` of weight 2.0 drawn by 4 tasks, last at task 1; the
///   latest task is 5, and at a decay of 0.5 a task it holds 2.0 x 0.5^4 = 0.125.
/// - Calls: `api/routes.py` lines 3 and 5 (tied by import path, 0.8), `lib/old.py` line 3 (by name
///   only, 0.3), and `api/admin.py` line 3, whose file changed on disk since it was indexed, so it
///   is not told. In path order: routes 3, routes 5, old 3. `api/imports.py` only imports it, on
///   line 1: no call.
/// - `tests/test_orders.py` holds a test and an import edge into it of weight 0.8.
#[test]
fn an_edit_s_facts_are_read_from_the_store_and_the_disk() -> TestResult {
    let mut repo = Repo::new()?;
    let orders_text = "def create_order(customer, items):\n    return 1\n";
    let orders = repo.file(
        "src/orders.py",
        orders_text,
        &[function("create_order", 1, 2)],
        FileFlags::empty(),
    )?;
    let [orders_node, create_order_node] = orders[..] else {
        return Err("a file and its function".into());
    };
    let create_order = repo
        .writer
        .queries()
        .symbols_in_file(
            repo.writer
                .queries()
                .file(&RepoPath::new("src/orders.py")?)?
                .ok_or("stored")?
                .id,
        )?
        .first()
        .ok_or("a symbol")?
        .id;
    let schema = *repo
        .file(
            "prisma/schema.prisma",
            "model Order {}\n",
            &[],
            FileFlags::empty(),
        )?
        .first()
        .ok_or("node")?;
    let billing = *repo
        .file("src/billing.py", "x = 1\n", &[], FileFlags::empty())?
        .first()
        .ok_or("node")?;
    // `lib/old.py` is stored first, so that the store's order is not the paths' order.
    let old = caller(
        &mut repo,
        ("lib/old.py", CALLER),
        &[3],
        (create_order, Resolution::NameOnly),
    )?;
    let tied = (create_order, Resolution::ImportPath);
    let routes = caller(&mut repo, ("api/routes.py", CALLER), &[3, 5], tied)?;
    let admin = caller(&mut repo, ("api/admin.py", CALLER), &[3], tied)?;
    user(
        &mut repo,
        ("api/imports.py", CALLER),
        (&[1], RefKind::Import),
        tied,
    )?;
    std::fs::write(
        repo.root.path().join("api").join("admin.py"),
        "changed since\n",
    )?;
    let test_def = Def {
        kind: SymbolKind::Test,
        ..function("test_create", 1, 2)
    };
    let test = *repo
        .file(
            "tests/test_orders.py",
            "def test_create():\n    pass\n",
            &[test_def],
            FileFlags::empty(),
        )?
        .first()
        .ok_or("node")?;
    repo.link((test, orders_node), EdgeType::Structural, (0.8, 1, 0))?;
    repo.link((orders_node, schema), EdgeType::CoChange, (0.73, 8, 0))?;
    repo.link((orders_node, billing), EdgeType::CoAccess, (2.0, 4, 1))?;
    repo.cochange(&[("src/orders.py", 11), ("prisma/schema.prisma", 10)], &[])?;
    repo.tasks(5)?;
    let _ = admin;
    let edited = RepoPath::new("src/orders.py")?;
    let asked = EditQuery {
        root: repo.root.path(),
        edited: &edited,
        drifted: &[create_order],
        link_decay: 0.5,
        max_file_bytes: 1_048_576,
    };
    let reader = repo.reader()?;
    let facts = edit_facts(reader.queries(), &asked)?;
    let site = |node, path: &str, line, confidence| -> TestResult<CallSite> {
        Ok(CallSite {
            file: named(node, path)?,
            line,
            confidence,
        })
    };
    assert_eq!(
        facts,
        Some(EditFacts {
            edited: named(orders_node, "src/orders.py")?,
            commits: 11,
            partners: vec![Partner {
                file: named(schema, "prisma/schema.prisma")?,
                co_changes: 8,
                confidence: 0.73,
                reverse_confidence: 0.8,
            }],
            linked: vec![Linked {
                file: named(billing, "src/billing.py")?,
                weight: 0.125,
                tasks: 4,
            }],
            drifts: vec![Drift {
                symbol: create_order_node,
                name: "create_order".to_owned(),
                call_sites: vec![
                    site(routes, "api/routes.py", 3, 0.8)?,
                    site(routes, "api/routes.py", 5, 0.8)?,
                    site(old, "lib/old.py", 3, 0.3)?,
                ],
                unchecked: 0,
            }],
            tests: vec![TestFile {
                file: named(test, "tests/test_orders.py")?,
                confidence: 0.8,
            }],
            origin: None,
        })
    );
    Ok(())
}

/// A file the store does not hold has no facts; a changed symbol the store no longer holds in the
/// edited file has no drift.
#[test]
fn what_the_store_does_not_hold_gives_nothing() -> TestResult {
    let mut repo = Repo::new()?;
    let orders = repo.file(
        "src/orders.py",
        "def a():\n    pass\n",
        &[function("a", 1, 2)],
        FileFlags::empty(),
    )?;
    let other = repo.file(
        "src/other.py",
        "def b():\n    pass\n",
        &[function("b", 1, 2)],
        FileFlags::empty(),
    )?;
    let queries = repo.writer.queries();
    let other_symbol = queries
        .symbols_in_file(
            queries
                .file(&RepoPath::new("src/other.py")?)?
                .ok_or("stored")?
                .id,
        )?
        .first()
        .ok_or("a symbol")?
        .id;
    let _ = (orders, other);
    let reader = repo.reader()?;
    let unknown = RepoPath::new("src/missing.py")?;
    let drifted = [other_symbol];
    let ask = |edited| EditQuery {
        root: repo.root.path(),
        edited,
        drifted: &drifted,
        link_decay: 0.5,
        max_file_bytes: 1_048_576,
    };
    assert_eq!(edit_facts(reader.queries(), &ask(&unknown))?, None);
    let edited = RepoPath::new("src/orders.py")?;
    let facts = edit_facts(reader.queries(), &ask(&edited))?.ok_or("facts")?;
    assert_eq!(facts.drifts, []);
    Ok(())
}

/// A symbol called from more files than are checked: the first 50 files the store holds are read
/// and their calls told; the calls in the 2 files past them are counted as not checked.
#[test]
fn calls_past_the_checked_files_are_counted() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file(
        "src/orders.py",
        "def create_order():\n    pass\n",
        &[function("create_order", 1, 2)],
        FileFlags::empty(),
    )?;
    let queries = repo.writer.queries();
    let create_order = queries
        .symbols_in_file(
            queries
                .file(&RepoPath::new("src/orders.py")?)?
                .ok_or("stored")?
                .id,
        )?
        .first()
        .ok_or("a symbol")?
        .id;
    for at in 0..52 {
        let path = format!("api/caller_{at:02}.py");
        caller(
            &mut repo,
            (&path, CALLER),
            &[3],
            (create_order, Resolution::ImportPath),
        )?;
    }
    let edited = RepoPath::new("src/orders.py")?;
    let drifted = [create_order];
    let reader = repo.reader()?;
    let facts = edit_facts(
        reader.queries(),
        &EditQuery {
            root: repo.root.path(),
            edited: &edited,
            drifted: &drifted,
            link_decay: 0.5,
            max_file_bytes: 1_048_576,
        },
    )?
    .ok_or("facts")?;
    let drift = facts.drifts.first().ok_or("a drift")?;
    assert_eq!(
        (drift.call_sites.len(), drift.unchecked),
        (cortex_retrieve::MAX_CHECKED_CALLER_FILES, 2)
    );
    assert_eq!(
        drift.call_sites.last().map(|site| site.file.path.as_str()),
        Some("api/caller_49.py")
    );
    Ok(())
}
