//! The scripted repository the API's tests read: five files, two sessions of agents with reads,
//! edits and responses, a co-change history, links of use, work orders given and used, warnings,
//! facts in each status, a stored blame with its commits and the texts of two of them.
//!
//! Every number the tests expect follows from what is written here, counted in the tests.

use std::path::Path;
use std::sync::Arc;

use cortex_core::{
    Agent, ContentHash, EdgeType, EventKind, EventMeta, EventSource, FactKind, FactSource,
    FactStatus, FileFlags, GateFeatures, HookKind, Identifier, InterestTier, ReasonKind, RepoPath,
    ShortHash, SymbolKind, Ulid, WarningKind,
};
use cortex_store::{
    BlameRecord, BlameRunRecord, CardKind, ChangeCardRecord, EdgeRecord, EventRecord, FactAnchor,
    FactRecord, FileChange, FileCommitsRow, FileIndex, FileRecord, InjectedFile, InjectionRecord,
    InterestRow, NodeId, SessionRecord, SymbolRecord, TaskRecord, UsageRow, WarningRecord, Writer,
};

use crate::history::Jobs;
use crate::test_store::{TestResult, writer};

/// A commit's id the history of `src/orders.rs` names first.
pub(super) const CHARGE: &str = "1111111111111111111111111111111111111111";
/// A commit's id the history names second.
pub(super) const ROUND: &str = "2222222222222222222222222222222222222222";
/// A commit's id the blame names but whose subject the cache does not hold.
const FORGOTTEN: &str = "4444444444444444444444444444444444444444";

/// The nodes of the files and of the symbol `OrderBook`, by what they are.
struct Nodes {
    /// `docs/guide.md`.
    guide: NodeId,
    /// `src/app.rs`.
    app: NodeId,
    /// `src/orders.rs`.
    orders: NodeId,
    /// The symbol `OrderBook` in `src/orders.rs`.
    book: NodeId,
    /// `src/orders_test.rs`.
    orders_test: NodeId,
    /// `src/util.rs`.
    util: NodeId,
    /// The rows of the files in the index, in the order of the nodes above.
    rows: [cortex_store::FileId; 5],
}

/// A file of `lines` lines at `path` with `symbols`, its content hashed from its path.
pub(super) fn file(path: &str, lines: u64, symbols: Vec<SymbolRecord>) -> TestResult<FileIndex> {
    Ok(FileIndex {
        file: FileRecord {
            path: RepoPath::new(path)?,
            language: None,
            size_bytes: 10,
            line_count: Some(lines),
            content_hash: Some(ContentHash::of(path.as_bytes())),
            mtime_ns: 1,
            flags: FileFlags::empty(),
            indexed_at_ms: 1,
        },
        symbols,
        refs: Vec::new(),
        passes_on: Vec::new(),
    })
}

/// A function or type called `name` that starts on `line`.
pub(super) fn symbol(name: &str, kind: SymbolKind, line: u32) -> TestResult<SymbolRecord> {
    Ok(SymbolRecord {
        parent: None,
        kind,
        name: Identifier::new(name)?,
        qual_name: Identifier::new(name)?,
        start_byte: u64::from(line) * 10,
        end_byte: u64::from(line) * 10 + 9,
        start_line: line,
        end_line: line + 1,
        sig_hash: ShortHash::of(name.as_bytes()),
        body_hash: ShortHash::of(name.as_bytes()),
    })
}

/// Stores the five files, and returns what names them.
fn store_files(writer: &mut Writer) -> TestResult<Nodes> {
    let mut transaction = writer.transaction()?;
    let mut put =
        |index: FileIndex| -> TestResult<FileChange> { Ok(transaction.replace_file(&index)?) };
    let guide = put(file("docs/guide.md", 10, Vec::new())?)?;
    let app = put(file("src/app.rs", 120, Vec::new())?)?;
    let symbols = vec![
        symbol("place_order", SymbolKind::Function, 10)?,
        symbol("OrderBook", SymbolKind::Class, 40)?,
    ];
    let orders = put(file("src/orders.rs", 80, symbols)?)?;
    let orders_test = put(file("src/orders_test.rs", 30, Vec::new())?)?;
    let mut vendored = file("src/util.rs", 5, Vec::new())?;
    vendored.file.flags = FileFlags::VENDORED;
    let util = put(vendored)?;
    transaction.commit()?;
    Ok(Nodes {
        guide: guide.file_node,
        app: app.file_node,
        orders: orders.file_node,
        book: *orders.symbol_nodes.get(1).ok_or("the second symbol")?,
        orders_test: orders_test.file_node,
        util: util.file_node,
        rows: [
            guide.file,
            app.file,
            orders.file,
            orders_test.file,
            util.file,
        ],
    })
}

/// A session of Claude Code started at `started_at_ms`.
fn session(id: &str, started_at_ms: i64) -> TestResult<SessionRecord> {
    Ok(SessionRecord {
        session_id: Identifier::new(id)?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms,
    })
}

/// The first task, of session `s1`.
pub(super) fn first_task() -> Ulid {
    Ulid::from_seed(1_000, b"first")
}

/// The second task, of session `s2`.
fn second_task() -> Ulid {
    Ulid::from_seed(3_000, b"second")
}

/// An event of `kind` at `ts_ms` about the file at `node` and `path`, which session `session_id`
/// reported in `task`, as the watcher when `source` says so.
fn event(
    (session_id, task): (&str, Ulid),
    ts_ms: i64,
    (kind, node, path): (EventKind, NodeId, &str),
    source: Option<EventSource>,
) -> TestResult<EventRecord> {
    Ok(EventRecord {
        task_id: Some(task),
        session_id: Some(Identifier::new(session_id)?),
        ts_ms,
        kind,
        node_id: Some(node),
        path: Some(RepoPath::new(path)?),
        range: None,
        meta: EventMeta {
            source,
            ..EventMeta::default()
        },
    })
}

/// A response of session `session_id` at `at_ms` whose four counts are `counts`: input, cache
/// writes, cache reads and output.
fn usage((session_id, turn, at_ms): (&str, u32, i64), counts: [u64; 4]) -> TestResult<UsageRow> {
    let [input, cache_write, cache_read, output] = counts;
    Ok(UsageRow {
        session_id: Identifier::new(session_id)?,
        turn,
        at_ms,
        model: None,
        input,
        cache_write,
        cache_write_1h: None,
        cache_read,
        output,
        reasoning: None,
        service_tier: None,
        speed: None,
        inference_geo: None,
        response: None,
    })
}

/// Stores the sessions, their tasks, what the agents did and the tokens it took.
fn store_activity(writer: &mut Writer, nodes: &Nodes) -> TestResult {
    let (first, second) = (first_task(), second_task());
    let mut transaction = writer.transaction()?;
    transaction.record_session(&session("s1", 100)?)?;
    transaction.record_session(&session("s2", 2_900)?)?;
    for (task, id, at_ms) in [(first, "s1", 1_000), (second, "s2", 3_000)] {
        transaction.start_task(&TaskRecord {
            task_id: task,
            session_id: Identifier::new(id)?,
            started_at_ms: at_ms,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: Some(false),
        })?;
    }
    let s1 = ("s1", first);
    let read = |node, path| (EventKind::Read, node, path);
    for row in [
        event(s1, 2_000, read(nodes.app, "src/app.rs"), None)?,
        event(s1, 2_100, read(nodes.app, "src/app.rs"), None)?,
        event(s1, 2_200, read(nodes.orders, "src/orders.rs"), None)?,
        event(
            s1,
            2_300,
            (EventKind::Edit, nodes.orders, "src/orders.rs"),
            None,
        )?,
        event(
            s1,
            2_400,
            (EventKind::Edit, nodes.util, "src/util.rs"),
            Some(EventSource::Watcher),
        )?,
        event(
            ("s2", second),
            3_000,
            read(nodes.guide, "docs/guide.md"),
            None,
        )?,
    ] {
        transaction.append_event(&row)?;
    }
    transaction.replace_usage(&[
        usage(("s1", 1, 2_050), [100, 0, 0, 50])?,
        usage(("s1", 2, 2_250), [10, 20, 30, 40])?,
        usage(("s1", 3, 2_350), [100, 0, 0, 100])?,
        usage(("s2", 1, 3_100), [300, 0, 0, 100])?,
    ])?;
    transaction.commit()?;
    Ok(())
}

/// An edge to `dst` that goes one way only, as the miner writes them.
fn one_way(dst: NodeId, weight: f64, count: u32) -> EdgeRecord {
    EdgeRecord {
        dst,
        weight,
        count,
        last_seq: 0,
        is_directed: true,
    }
}

/// Stores interest, commit counts, co-change edges and links of use.
fn store_learning(writer: &mut Writer, nodes: &Nodes) -> TestResult {
    let mut transaction = writer.transaction()?;
    let interest = |node, score, last_seq| InterestRow {
        node,
        score,
        last_seq,
        tier: InterestTier::Interesting,
    };
    transaction.replace_interest(&[
        interest(nodes.app, 2.0, 2),
        interest(nodes.orders, 1.0, 1),
        interest(nodes.util, 0.001, 2),
    ])?;
    let commits = |file, count| FileCommitsRow {
        file,
        count,
        weighted: f64::from(count),
    };
    let [_, app, orders, orders_test, _] = nodes.rows;
    transaction.add_cochange(
        &[commits(app, 9), commits(orders, 8), commits(orders_test, 4)],
        &[],
    )?;
    let co_change = EdgeType::CoChange;
    transaction.replace_edges(nodes.app, co_change, &[one_way(nodes.orders, 0.8, 6)])?;
    transaction.replace_edges(
        nodes.orders,
        co_change,
        &[
            one_way(nodes.app, 0.5, 6),
            one_way(nodes.orders_test, 0.6, 4),
        ],
    )?;
    transaction.replace_edges(
        nodes.orders_test,
        co_change,
        &[one_way(nodes.orders, 0.9, 4)],
    )?;
    let used = |dst| EdgeRecord {
        dst,
        weight: 2.5,
        count: 1,
        last_seq: 1,
        is_directed: false,
    };
    transaction.replace_edges(nodes.app, EdgeType::CoAccess, &[used(nodes.util)])?;
    transaction.replace_edges(nodes.util, EdgeType::CoAccess, &[used(nodes.app)])?;
    transaction.commit()?;
    Ok(())
}

/// A fact saying `text` from `source` made at `at_ms`, about `node`.
fn fact(text: &str, source: FactSource, at_ms: i64, node: NodeId) -> FactRecord {
    FactRecord {
        text: text.to_owned(),
        kind: FactKind::Convention,
        source,
        status: source.first_status(),
        created_at_ms: at_ms,
        task: (source == FactSource::Agent).then(first_task),
        anchors: vec![FactAnchor {
            node,
            hash: ShortHash::of(&node.get().to_le_bytes()),
        }],
    }
}

/// Stores five facts: three about `src/orders.rs` (active, a candidate anchored to its symbol, and
/// one made suspect) and two about `src/app.rs` (active, and one retired).
fn store_facts(writer: &mut Writer, nodes: &Nodes) -> TestResult {
    let mut transaction = writer.transaction()?;
    let user = FactSource::User;
    transaction.add_fact(&fact(
        "Orders are charged in integer cents.",
        user,
        1,
        nodes.orders,
    ))?;
    transaction.add_fact(&fact(
        "Orders may be cancelled once.",
        FactSource::Agent,
        2,
        nodes.book,
    ))?;
    let suspect = transaction.add_fact(&fact("Totals never round.", user, 3, nodes.orders))?;
    transaction.set_fact_status(suspect, FactStatus::Suspect, 4)?;
    transaction.add_fact(&fact("The app starts in one thread.", user, 5, nodes.app))?;
    let retired = transaction.add_fact(&fact("The app is single user.", user, 6, nodes.app))?;
    transaction.set_fact_status(retired, FactStatus::Retired, 7)?;
    transaction.commit()?;
    Ok(())
}

/// A prompt's work order of `task`, of session `session_id`, at `at_ms` that named `files` and
/// reached the agent.
pub(super) fn work_order(
    (task, session_id): (Ulid, &str),
    at_ms: i64,
    files: &[NodeId],
) -> TestResult<InjectionRecord> {
    Ok(InjectionRecord {
        task_id: Some(task),
        session_id: Identifier::new(session_id)?,
        hook: HookKind::Prompt,
        at_ms,
        files: files
            .iter()
            .map(|node| InjectedFile {
                node: *node,
                spans: vec![(1, 2)],
                reason: Some(ReasonKind::Named),
                shown: Vec::new(),
                content: None,
            })
            .collect(),
        tokens: 100,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"a work order")),
        features: Some(GateFeatures {
            top_lexical: 1.0,
            top_mass: 0.5,
            agreeing_kinds: 1,
            supporting_kinds: 1,
            similar_task: 0.0,
            recent_precision: None,
            is_continuing: false,
        }),
    })
}

/// Stores three work orders and two warnings: the first two work orders, of the first task, were
/// labelled at its end (used); the third, of the second task, was not; a warning to
/// `src/orders.rs` has no resolution yet and one to `src/orders_test.rs` was resolved as unused.
fn store_told(writer: &mut Writer, nodes: &Nodes) -> TestResult {
    let (first, second) = (first_task(), second_task());
    let mut transaction = writer.transaction()?;
    let (s1, s2) = ((first, "s1"), (second, "s2"));
    let one = transaction.record_injection(&work_order(s1, 1_500, &[nodes.app, nodes.orders])?)?;
    let two = transaction.record_injection(&work_order(
        s1,
        1_600,
        &[nodes.app, nodes.orders, nodes.orders_test],
    )?)?;
    transaction.record_injection(&work_order(s2, 3_050, &[nodes.orders_test])?)?;
    transaction.label_injection(one, 2, true)?;
    transaction.label_injection(two, 3, true)?;
    let warning = |kind, at_ms, source, target| WarningRecord {
        task_id: first,
        at_ms,
        kind,
        source,
        target,
        strength: 0.5,
    };
    transaction.record_warning(&warning(
        WarningKind::CoChange,
        2_310,
        nodes.app,
        nodes.orders,
    ))?;
    let unused = transaction.record_warning(&warning(
        WarningKind::Test,
        2_320,
        nodes.orders,
        nodes.orders_test,
    ))?;
    transaction.resolve_warning(unused, false)?;
    transaction.commit()?;
    Ok(())
}

/// A card of the commit `oid`, committed at `committed_at_s`, from pull request `pull_request`
/// naming `issues`.
pub(super) fn card(
    oid: &str,
    committed_at_s: i64,
    pull_request: Option<u64>,
    issues: &[&str],
) -> ChangeCardRecord {
    ChangeCardRecord {
        oid: oid.to_owned(),
        first_parent: None,
        committed_at_s,
        kind: CardKind::Normal,
        pull_request,
        issues: issues.iter().map(|issue| (*issue).to_owned()).collect(),
        has_reason: false,
        files: Vec::new(),
        symbols: Vec::new(),
        tests: Vec::new(),
        carded_at_ms: 1,
    }
}

/// Stores a blame of `src/orders.rs` naming three commits, and the cards of those and of one more.
fn store_history(writer: &mut Writer, texts: &Jobs) -> TestResult {
    let run = |start, end, oid: &str| BlameRunRecord {
        start,
        end,
        source_start: 1,
        oid: oid.to_owned(),
    };
    let mut transaction = writer.transaction()?;
    transaction.put_blame(&BlameRecord {
        path: RepoPath::new("src/orders.rs")?,
        file_hash: ContentHash::of(b"src/orders.rs"),
        first_line: 1,
        last_line: 80,
        runs: vec![
            run(1, 30, CHARGE),
            run(31, 60, ROUND),
            run(61, 80, FORGOTTEN),
        ],
        computed_at_ms: 1,
    })?;
    for card in [
        card(CHARGE, 1_762_300_000, Some(482), &["#12"]),
        card(ROUND, 1_762_000_000, None, &[]),
        card(FORGOTTEN, 1_762_500_000, None, &["#9"]),
        card(
            "3333333333333333333333333333333333333333",
            1_762_100_000,
            None,
            &[],
        ),
    ] {
        transaction.put_change_card(&card, &[])?;
    }
    transaction.commit()?;
    texts.put_subject(CHARGE, "Charge in integer cents");
    texts.put_subject(ROUND, "Round tax once");
    Ok(())
}

/// Writes the scripted repository's store under `root` and returns the cache of commit subjects
/// that goes with it.
pub(super) fn script(root: &Path) -> TestResult<Arc<Jobs>> {
    let mut writer = writer(root)?;
    let nodes = store_files(&mut writer)?;
    store_activity(&mut writer, &nodes)?;
    store_learning(&mut writer, &nodes)?;
    store_facts(&mut writer, &nodes)?;
    store_told(&mut writer, &nodes)?;
    let texts = Arc::new(Jobs::default());
    store_history(&mut writer, &texts)?;
    Ok(texts)
}

/// Writes a cold store under `root`: two files and nothing learned.
pub(super) fn cold(root: &Path) -> TestResult {
    let mut writer = writer(root)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&file("README.md", 3, Vec::new())?)?;
    transaction.replace_file(&file("src/main.rs", 9, Vec::new())?)?;
    transaction.commit()?;
    Ok(())
}
