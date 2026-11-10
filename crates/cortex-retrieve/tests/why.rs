//! Why code is where it is, from a store built here: its file's strongest links of each kind, the
//! facts about it true now (those on the symbol asked of first), the sessions working on its file
//! now, and the warnings about its file whose tasks have not ended; and what a target names, or
//! why it names nothing. Every expected line is typed out by hand from the fixture.

mod common;

use common::task::{acted, task, warned};
use common::{Def, Repo, TestResult};
use cortex_coord::Timing;
use cortex_core::{
    Agent, EdgeType, EventKind, EventMeta, FactKind, FactSource, FactStatus, FileFlags, Identifier,
    PositionMode, RepoPath, SessionStatus, SymbolKind, ToolKind, WarningKind,
};
use cortex_retrieve::{MAX_WHY_TOKENS, Why, WhyAt, why};
use cortex_store::{EventRecord, FactRecord, NodeId, PositionRecord, SessionRecord};

/// A file name holding a right-to-left override, which no name shown to an agent may hold.
const CRAFTED: &str = "lib/x\u{202E}yp.py";

/// A minute in milliseconds.
const MINUTE_MS: i64 = 60_000;

/// When every answer here is given.
const NOW_MS: i64 = 600 * MINUTE_MS;

/// `orders.py` and `cart.py` hold this text: a class with a method on lines 1 to 3, a blank line,
/// and a function on lines 5 and 6; six lines in all.
const CLASS_AND_FUNCTION: &str =
    "class Order:\n    def create(self):\n        return 1\n\ndef total():\n    return 2\n";

/// A definition of `kind` called `name` on lines `start` to `end`, inside the one at `parent`.
fn def(name: &str, kind: SymbolKind, (start, end): (u32, u32), parent: Option<usize>) -> Def<'_> {
    Def {
        name,
        kind,
        start,
        end,
        parent,
    }
}

/// How long positions hold here: a read 30 minutes, an edit or a claim an hour; a session is live
/// for 15 minutes after its last event.
fn timing() -> Timing {
    Timing {
        read_ms: 30 * MINUTE_MS,
        edit_ms: 60 * MINUTE_MS,
        edit_after_task_ms: 10 * MINUTE_MS,
        claim_ms: 60 * MINUTE_MS,
        live_ms: 15 * MINUTE_MS,
        same_file_ms: 5 * MINUTE_MS,
        handoff_idle_ms: 10 * MINUTE_MS,
    }
}

/// The answer for `target`, or the refusal with `Refused: ` before it.
fn asked(repo: &Repo, target: &str) -> TestResult<String> {
    let reader = repo.reader()?;
    let timing = timing();
    let at = WhyAt {
        now_ms: NOW_MS,
        timing: &timing,
        root: repo.root.path(),
        max_file_bytes: 1_048_576,
        link_decay: 0.5,
    };
    Ok(match why(reader.queries(), target, &at)? {
        Why::Answer(text) => text,
        Why::Refused(why) => format!("Refused: {why}"),
    })
}

/// The nodes of the fixture's files and definitions.
struct Nodes {
    /// `orders.py`, `Order`, `Order.create`, `total`.
    orders: [NodeId; 4],
    /// `cart.py`, `Cart`, `Cart.add`, `total`.
    cart: [NodeId; 4],
    /// `views.py`.
    views: NodeId,
    /// `tests/test_orders.py`.
    tests: NodeId,
    /// `jobs.py`.
    jobs: NodeId,
    /// [`CRAFTED`].
    crafted: NodeId,
}

/// The fixture's files: `orders.py` and `cart.py` as [`CLASS_AND_FUNCTION`] says (`cart.py`'s
/// class `Cart` with a method `add`), `views.py` and `tests/test_orders.py` of one function each,
/// `jobs.py`, whose classes `A` and `B` each define `run`, `pair.py` (classes `A` and `B` on line
/// 1, `C` and its member `m` on line 2), an empty `empty.py`, `odd.py`, whose function's name holds
/// an agent's reminder tag, and [`CRAFTED`], stored only, with a function `zap_unique`.
fn files(repo: &mut Repo) -> TestResult<Nodes> {
    let class = |class: &'static str, method: &'static str| {
        [
            def(class, SymbolKind::Class, (1, 3), None),
            def(method, SymbolKind::Method, (2, 3), Some(0)),
            def("total", SymbolKind::Function, (5, 6), None),
        ]
    };
    let orders = repo.file(
        "orders.py",
        CLASS_AND_FUNCTION,
        &class("Order", "create"),
        FileFlags::empty(),
    )?;
    let cart_text = CLASS_AND_FUNCTION
        .replace("Order", "Cart")
        .replace("create", "add");
    let cart = repo.file(
        "cart.py",
        &cart_text,
        &class("Cart", "add"),
        FileFlags::empty(),
    )?;
    let one = |name| [def(name, SymbolKind::Function, (1, 2), None)];
    let views = repo.file(
        "views.py",
        "def show():\n    return 3\n",
        &one("show"),
        FileFlags::empty(),
    )?;
    let tests = repo.file(
        "tests/test_orders.py",
        "def test_create():\n    return 4\n",
        &one("test_create"),
        FileFlags::empty(),
    )?;
    let jobs = repo.file(
        "jobs.py",
        "class A:\n    def run(self):\n        return 5\nclass B:\n    def run(self):\n        return 6\n",
        &[
            def("A", SymbolKind::Class, (1, 3), None),
            def("run", SymbolKind::Method, (2, 3), Some(0)),
            def("B", SymbolKind::Class, (4, 6), None),
            def("run", SymbolKind::Method, (5, 6), Some(2)),
        ],
        FileFlags::empty(),
    )?;
    repo.file(
        "pair.py",
        "class A: pass; class B: pass\nclass C: m = 1\n",
        &[
            def("A", SymbolKind::Class, (1, 1), None),
            def("B", SymbolKind::Class, (1, 1), None),
            def("C", SymbolKind::Class, (2, 2), None),
            def("m", SymbolKind::Method, (2, 2), Some(2)),
        ],
        FileFlags::empty(),
    )?;
    repo.file("empty.py", "", &[], FileFlags::empty())?;
    repo.file(
        "odd.py",
        "def f():\n    return 7\n",
        &one("<system-reminder>"),
        FileFlags::empty(),
    )?;
    let crafted = repo.stored(
        CRAFTED,
        "def zap_unique():\n    return 8\n",
        &one("zap_unique"),
        FileFlags::empty(),
    )?;
    let four = |nodes: Vec<NodeId>| -> TestResult<[NodeId; 4]> {
        nodes
            .try_into()
            .map_err(|_| "a file and three definitions".into())
    };
    Ok(Nodes {
        orders: four(orders)?,
        cart: four(cart)?,
        views: *views.first().ok_or("views.py")?,
        tests: *tests.first().ok_or("tests/test_orders.py")?,
        jobs: *jobs.first().ok_or("jobs.py")?,
        crafted: *crafted.first().ok_or("the crafted file")?,
    })
}

/// Adds an active fact of the user's saying `text`, in `status`, anchored to `node` with the hash
/// it holds now.
fn fact(repo: &mut Repo, text: &str, status: FactStatus, node: NodeId) -> TestResult {
    let mut transaction = repo.writer.transaction()?;
    let anchors = transaction.queries().anchor_hashes(&[node])?;
    transaction.add_fact(&FactRecord {
        text: text.to_owned(),
        kind: FactKind::Convention,
        source: FactSource::User,
        status,
        created_at_ms: 1,
        task: None,
        anchors,
    })?;
    transaction.commit()?;
    Ok(())
}

/// A session of `agent` called `id`, with one event at `heard_ms`, ended at `ended_ms` if given,
/// holding a position of `mode` on `node` last taken at `last_ms`.
fn session(
    repo: &mut Repo,
    (id, agent): (&str, Agent),
    (heard_ms, ended_ms): (i64, Option<i64>),
    (node, mode, last_ms): (NodeId, PositionMode, i64),
) -> TestResult {
    let session_id = Identifier::new(id)?;
    let mut transaction = repo.writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: session_id.clone(),
        agent,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 1,
    })?;
    transaction.append_event(&EventRecord {
        task_id: None,
        session_id: Some(session_id.clone()),
        ts_ms: heard_ms,
        kind: EventKind::Read,
        node_id: Some(node),
        path: None,
        range: None,
        meta: EventMeta::default(),
    })?;
    if let Some(ended_ms) = ended_ms {
        transaction.end_session(&session_id, (ended_ms, SessionStatus::Ended))?;
    }
    transaction.keep_position(&PositionRecord {
        session_id,
        node,
        mode,
        since_ms: last_ms,
        last_ms,
        worktree: None,
        branch: None,
    })?;
    transaction.commit()?;
    Ok(())
}

/// Builds the whole fixture: the files, their links, facts, sessions and warnings.
///
/// Links of `orders.py`, counting its definitions: by code, `Order.create` to `Cart.add` 0.9, the
/// file to `views.py` 0.85, to `jobs.py` 0.6, below the 0.8 a reference needs to count, and to
/// [`CRAFTED`] 0.99, whose name is never shown; in history, to `tests/test_orders.py` 0.75 and to
/// `cart.py` 0.4; in past tasks, from `views.py` 1.5 at task 1, the latest, read the other way, to
/// `tests/test_orders.py` 2.0 at task 0 from the file and 1.75 at task 1 from `total`, and to
/// `jobs.py` 0.08 at task 0. A co-access link keeps half its weight for each task since its last
/// here: `views.py` keeps 1.5, 0.30 of the most a link holds (5); `tests/test_orders.py` 1.0 of
/// its stronger stored link, 0.20, and 1.75 of the other, 0.35, the stronger now; `jobs.py` 0.04,
/// below the 0.05 that counts.
/// `cart.py` to `views.py` by code, 0.95, is not `orders.py`'s.
///
/// Facts, in id order: 1 on `Order.create`, 2 on `orders.py`, 3 on `total`, all active; 4 on
/// `cart.py`, active, about another file; 5 on `orders.py`, a candidate.
///
/// Sessions, now being minute 600: `s1` (Claude Code, the task's) edited `orders.py` at 596 and was
/// last heard from then; Codex `z7f3a9c2-1b4d` read it at 599; Codex `quiet` read it at 580, its
/// read holding but itself not heard from for 20 minutes; Claude Code `gone` read it at 598 and
/// ended at 599; Codex `earlier` read it at 565, a read that held 30 minutes, though it was heard
/// from at 598; `s1` also edited `cart.py`.
///
/// Warnings in `s1`'s open task, latest first: at 595, `tests/test_orders.py` changes with
/// `orders.py`; at 594, it holds `orders.py`'s tests; at 593, `orders.py` calls `cart.py`'s
/// `Cart.add`, whose signature changed; at 592, the first again; at 591, `views.py` changes with
/// `cart.py`, naming `orders.py` nowhere.
fn fixture() -> TestResult<(Repo, Nodes)> {
    let mut repo = Repo::new()?;
    let nodes = files(&mut repo)?;
    let [orders, _, create, total] = nodes.orders;
    let [cart, _, add, _] = nodes.cart;
    repo.link((create, add), EdgeType::Structural, (0.9, 1, 1))?;
    repo.link((orders, nodes.views), EdgeType::Structural, (0.85, 1, 1))?;
    repo.link((orders, nodes.jobs), EdgeType::Structural, (0.6, 1, 1))?;
    repo.link((orders, nodes.crafted), EdgeType::Structural, (0.99, 1, 1))?;
    repo.link((orders, nodes.tests), EdgeType::CoChange, (0.75, 3, 1))?;
    repo.link((orders, cart), EdgeType::CoChange, (0.4, 2, 1))?;
    repo.link((nodes.views, orders), EdgeType::CoAccess, (1.5, 1, 1))?;
    repo.link((orders, nodes.tests), EdgeType::CoAccess, (2.0, 1, 0))?;
    repo.link((total, nodes.tests), EdgeType::CoAccess, (1.75, 1, 1))?;
    repo.link((orders, nodes.jobs), EdgeType::CoAccess, (0.08, 1, 0))?;
    repo.link((cart, nodes.views), EdgeType::Structural, (0.95, 1, 1))?;
    fact(
        &mut repo,
        "Orders are created from a whole cart.",
        FactStatus::Active,
        create,
    )?;
    fact(
        &mut repo,
        "Totals are in cents.",
        FactStatus::Active,
        orders,
    )?;
    fact(
        &mut repo,
        "Totals round half up.",
        FactStatus::Active,
        total,
    )?;
    fact(
        &mut repo,
        "Carts are never empty.",
        FactStatus::Active,
        cart,
    )?;
    fact(
        &mut repo,
        "Orders are drafts first.",
        FactStatus::Candidate,
        orders,
    )?;
    let task = task(&mut repo, None)?;
    acted(
        &mut repo,
        task,
        (ToolKind::Edit, "orders.py", orders),
        (596 * MINUTE_MS, true),
    )?;
    let held = |node, mode, minute: i64| (node, mode, minute * MINUTE_MS);
    let s1 = Identifier::new("s1")?;
    let mut transaction = repo.writer.transaction()?;
    for (node, minute) in [(orders, 596), (cart, 590)] {
        transaction.keep_position(&PositionRecord {
            session_id: s1.clone(),
            node,
            mode: PositionMode::Edit,
            since_ms: minute * MINUTE_MS,
            last_ms: minute * MINUTE_MS,
            worktree: None,
            branch: None,
        })?;
    }
    transaction.commit()?;
    let read = PositionMode::Read;
    session(
        &mut repo,
        ("z7f3a9c2-1b4d", Agent::Codex),
        (599 * MINUTE_MS, None),
        held(orders, read, 599),
    )?;
    session(
        &mut repo,
        ("quiet", Agent::Codex),
        (580 * MINUTE_MS, None),
        held(orders, read, 580),
    )?;
    session(
        &mut repo,
        ("gone", Agent::Claude),
        (598 * MINUTE_MS, Some(599 * MINUTE_MS)),
        held(orders, read, 598),
    )?;
    session(
        &mut repo,
        ("earlier", Agent::Codex),
        (598 * MINUTE_MS, None),
        held(orders, read, 565),
    )?;
    let warn = |kind, source, target, minute: i64| (kind, source, target, minute * MINUTE_MS);
    for (kind, source, target, at_ms) in [
        warn(WarningKind::CoChange, orders, nodes.tests, 595),
        warn(WarningKind::Test, orders, nodes.tests, 594),
        warn(WarningKind::CallSite, add, orders, 593),
        warn(WarningKind::CoChange, orders, nodes.tests, 592),
        warn(WarningKind::CoChange, cart, nodes.views, 591),
    ] {
        warned(&mut repo, task, (kind, source, target), at_ms)?;
    }
    Ok((repo, nodes))
}

/// Asked of `orders.py`, worked by hand from [`fixture`]: its links by code, `cart.py` 0.90 (by
/// its method) and `views.py` 0.85; in history, `tests/test_orders.py` 0.75 then `cart.py` 0.40;
/// in past tasks, `tests/test_orders.py` 0.35 then `views.py` 0.30. Its facts, none of their
/// symbols asked of: 1 and 3, on its
/// definitions, by id, then 2, on the file; 4 is another file's, 5 a candidate. Its sessions, the
/// last to act first, not in id order: Codex `z7f3a9c2` read it a minute ago, `s1` edited it 4 minutes ago; `quiet`
/// is not heard from, `gone` ended and the read of `earlier` no longer holds. Its warnings, latest first, the repeated one once:
/// changes-with at 595 (5 minutes ago), tests at 594, the call at 593; `views.py`'s is not about
/// it.
#[test]
fn a_file_is_explained_by_its_links_facts_sessions_and_warnings() -> TestResult {
    let (repo, _) = fixture()?;
    assert_eq!(
        asked(&repo, "orders.py")?,
        "[cortex] Why · orders.py\n\
         Linked by code:\n\
         \x20 - cart.py (0.90)\n\
         \x20 - views.py (0.85)\n\
         Changed together in history:\n\
         \x20 - tests/test_orders.py (0.75)\n\
         \x20 - cart.py (0.40)\n\
         Used together in past tasks:\n\
         \x20 - tests/test_orders.py (0.35)\n\
         \x20 - views.py (0.30)\n\
         Project facts:\n\
         \x20 - Orders are created from a whole cart. (anchored to orders.py:Order.create)\n\
         \x20 - Totals round half up. (anchored to orders.py:total)\n\
         \x20 - Totals are in cents. (anchored to orders.py)\n\
         Sessions working on it now:\n\
         \x20 - Codex session z7f3a9c2 read it 1 minute ago\n\
         \x20 - Claude Code session s1 edited it 4 minutes ago\n\
         Warnings whose tasks have not ended:\n\
         \x20 - tests/test_orders.py usually changes with orders.py, which was edited 5 minutes ago\n\
         \x20 - tests/test_orders.py holds the tests of orders.py, which was edited 6 minutes ago\n\
         \x20 - orders.py calls cart.py:Cart.add, whose signature changed 7 minutes ago\n"
    );
    Ok(())
}

/// Asked of a symbol, the answer names it and says the rest is its file's, and the facts on it
/// come first: of `orders.py:total`, fact 3 (on it), then 1 (another definition), then 2 (the
/// file). `orders.py:3` is the innermost definition spanning line 3, `Order.create` (lines 2 and
/// 3, inside `Order`, lines 1 to 3), so fact 1 leads. `views.py`, linked and used but with no fact,
/// session or warning of its own, has the links alone: `orders.py` by code 0.85 and `cart.py`
/// 0.95, strongest first, and `orders.py` in past tasks; the warning naming it is one whose task
/// is open, `views.py` changing with `cart.py`.
#[test]
fn a_symbol_is_explained_with_its_own_facts_first() -> TestResult {
    let (repo, _) = fixture()?;
    let total = asked(&repo, "orders.py:total")?;
    assert!(
        total.starts_with(
            "[cortex] Why · orders.py:total · what is known of its file, its own facts first\n"
        ),
        "{total}"
    );
    assert!(
        total.contains(
            "Project facts:\n\
             \x20 - Totals round half up. (anchored to orders.py:total)\n\
             \x20 - Orders are created from a whole cart. (anchored to orders.py:Order.create)\n\
             \x20 - Totals are in cents. (anchored to orders.py)\n"
        ),
        "{total}"
    );
    let line = asked(&repo, "orders.py:3")?;
    assert!(
        line.starts_with(
            "[cortex] Why · orders.py:Order.create · what is known of its file, its own facts \
             first\n"
        ),
        "{line}"
    );
    assert!(
        line.contains("Project facts:\n  - Orders are created from a whole cart."),
        "{line}"
    );
    assert_eq!(
        asked(&repo, "views.py")?,
        "[cortex] Why · views.py\n\
         Linked by code:\n\
         \x20 - cart.py (0.95)\n\
         \x20 - orders.py (0.85)\n\
         Used together in past tasks:\n\
         \x20 - orders.py (0.30)\n\
         Warnings whose tasks have not ended:\n\
         \x20 - views.py usually changes with cart.py, which was edited 9 minutes ago\n"
    );
    Ok(())
}

/// Worked by hand from [`fixture`]. Each way of naming code, and each refusal: a path; `path:line`
/// for line 1 (`Order`, lines 1 to 3), line 3 (`Order.create`, the shorter of the two spanning it)
/// and line 4 (no definition: the file); lines 0 and 7 of six; `path:symbol` by a qualified or a
/// plain name; a symbol the file does not define, or defines twice (`jobs.py`'s two `run`); a
/// symbol by name alone, one (`Cart.add`, `create` naming `Order.create` alone) or two (`total`,
/// in `orders.py` first, stored first); a name of nothing; text that is empty or not plain; a
/// line two definitions share (`pair.py`'s line 1, `A` and `B`) and one where a class and its one
/// member nest (line 2: the member, `C.m`); an empty file; a definition whose name holds an agent's
/// reminder tag (`odd.py`), and one in a file whose name holds a right-to-left override, named
/// alone (`zap_unique`); and a target whose echo would be over the cap.
#[test]
fn a_target_names_one_thing_or_is_refused() -> TestResult {
    let (repo, _) = fixture()?;
    let first = |target: &str| -> TestResult<String> {
        let text = asked(&repo, target)?;
        Ok(text.lines().next().unwrap_or_default().to_owned())
    };
    let symbol = |name: &str| {
        format!("[cortex] Why · {name} · what is known of its file, its own facts first")
    };
    assert_eq!(first("orders.py")?, "[cortex] Why · orders.py");
    assert_eq!(first("orders.py:1")?, symbol("orders.py:Order"));
    assert_eq!(first("orders.py:3")?, symbol("orders.py:Order.create"));
    assert_eq!(
        first("orders.py:4")?,
        "[cortex] Why · orders.py · line 4 is in no definition, so this is the file"
    );
    assert_eq!(first("orders.py:0")?, "Refused: orders.py has lines 1 to 6");
    assert_eq!(first("orders.py:7")?, "Refused: orders.py has lines 1 to 6");
    assert_eq!(
        first("orders.py:Order.create")?,
        symbol("orders.py:Order.create")
    );
    assert_eq!(first("orders.py:create")?, symbol("orders.py:Order.create"));
    assert_eq!(
        first("orders.py:pay")?,
        "Refused: orders.py defines no symbol pay"
    );
    assert_eq!(
        first("jobs.py:run")?,
        "Refused: jobs.py defines 2 symbols named run: A.run, B.run; name one by its qualified name"
    );
    assert_eq!(first("jobs.py:B.run")?, symbol("jobs.py:B.run"));
    assert_eq!(first("Cart.add")?, symbol("cart.py:Cart.add"));
    assert_eq!(first("create")?, symbol("orders.py:Order.create"));
    assert_eq!(
        first("total")?,
        "Refused: 2 symbols are named total: orders.py:total, cart.py:total; name one as \
         `path:symbol`"
    );
    assert_eq!(
        first("refund")?,
        "Refused: refund names no file or symbol Cortex indexes"
    );
    let plain = "Refused: name a file, `path:line`, `path:symbol` or a symbol, in plain text";
    assert_eq!(first("  ")?, plain);
    assert_eq!(first("orders.py\u{7}")?, plain);
    assert_eq!(
        first("pair.py:1")?,
        "Refused: line 1 of pair.py holds 2 definitions: A, B; name one as `path:symbol`"
    );
    assert_eq!(first("pair.py:2")?, symbol("pair.py:C.m"));
    assert_eq!(first("empty.py:1")?, "Refused: empty.py is empty");
    let shown = "Refused: the file or definition it names has a name Cortex does not show as it is";
    assert_eq!(first("odd.py:1")?, shown);
    assert_eq!(first("zap_unique")?, shown);
    assert_eq!(
        asked(&repo, &"x".repeat(5_000))?,
        "Refused: the answer would be longer than Cortex gives: name the target by a shorter \
         path or name"
    );
    Ok(())
}

/// A line is looked up only in the copy the index read: once `orders.py` changes on disk, line 3
/// is refused, while the file and its symbols by name are still answered, without the facts about
/// them, which can no longer be shown true. A file stored but gone from disk is refused the same
/// way.
#[test]
fn a_line_of_a_changed_file_is_refused() -> TestResult {
    let (repo, _) = fixture()?;
    std::fs::write(
        RepoPath::new("orders.py")?.to_native(repo.root.path()),
        "# moved\n",
    )?;
    let refused = "Refused: orders.py on disk is not the copy Cortex last read, so its lines are \
                   not known yet: ask again in a moment";
    assert_eq!(asked(&repo, "orders.py:3")?, refused);
    let create = asked(&repo, "orders.py:Order.create")?;
    assert!(
        create.starts_with("[cortex] Why · orders.py:Order.create"),
        "{create}"
    );
    assert!(!create.contains("Project facts:"), "{create}");
    assert!(
        create.contains("Linked by code:\n  - cart.py (0.90)\n"),
        "{create}"
    );
    std::fs::remove_file(RepoPath::new("views.py")?.to_native(repo.root.path()))?;
    assert_eq!(
        asked(&repo, "views.py:1")?,
        "Refused: views.py on disk is not the copy Cortex last read, so its lines are not known \
         yet: ask again in a moment"
    );
    Ok(())
}

/// A file nothing is known of yet says so, and no answer is over its cap: `jobs.py` has no fact,
/// session or warning, and its links, by code at 0.6 and in past tasks faded to 0.04, are too weak
/// to count; the busiest answer here is far below the cap.
#[test]
fn an_answer_with_nothing_known_says_so() -> TestResult {
    let (repo, _) = fixture()?;
    assert_eq!(
        asked(&repo, "jobs.py")?,
        "[cortex] Why · jobs.py\nNothing is known yet of its links, facts, sessions or warnings.\n"
    );
    assert!(cortex_core::estimate_tokens(&asked(&repo, "orders.py")?) <= MAX_WHY_TOKENS);
    Ok(())
}

/// A hub file with more of each than an answer names, worked by hand: `hub.py` linked by code to
/// `spoke0.py` to `spoke5.py` at 0.81 to 0.86; Codex sessions `a0` to `a5` reading it at minutes
/// 590 to 595, each heard from then; and warnings, in an open task, that `spoke0.py` to
/// `spoke5.py` change with it, at minutes 580 to 585. The answer names the five strongest links,
/// the five sessions that read last and the five latest warnings; `spoke0.py` and `a0` are left
/// out.
#[test]
fn an_answer_names_at_most_five_of_each() -> TestResult {
    let mut repo = Repo::new()?;
    let one = [def("hub", SymbolKind::Function, (1, 2), None)];
    let hub = *repo
        .file(
            "hub.py",
            "def hub():\n    return 0\n",
            &one,
            FileFlags::empty(),
        )?
        .first()
        .ok_or("hub.py")?;
    let task = task(&mut repo, None)?;
    let weights = [0.81, 0.82, 0.83, 0.84, 0.85, 0.86];
    for (at, weight) in (0_i64..).zip(weights) {
        let path = format!("spoke{at}.py");
        let spoke = *repo
            .file(&path, "x = 1\n", &[], FileFlags::empty())?
            .first()
            .ok_or("a spoke")?;
        repo.link((hub, spoke), EdgeType::Structural, (weight, 1, 1))?;
        let minute = 590 + at;
        session(
            &mut repo,
            (&format!("a{at}"), Agent::Codex),
            (minute * MINUTE_MS, None),
            (hub, PositionMode::Read, minute * MINUTE_MS),
        )?;
        let warning = (WarningKind::CoChange, hub, spoke);
        warned(&mut repo, task, warning, (580 + at) * MINUTE_MS)?;
    }
    assert_eq!(
        asked(&repo, "hub.py")?,
        "[cortex] Why · hub.py\n\
         Linked by code:\n\
         \x20 - spoke5.py (0.86)\n\
         \x20 - spoke4.py (0.85)\n\
         \x20 - spoke3.py (0.84)\n\
         \x20 - spoke2.py (0.83)\n\
         \x20 - spoke1.py (0.82)\n\
         Sessions working on it now:\n\
         \x20 - Codex session a5 read it 5 minutes ago\n\
         \x20 - Codex session a4 read it 6 minutes ago\n\
         \x20 - Codex session a3 read it 7 minutes ago\n\
         \x20 - Codex session a2 read it 8 minutes ago\n\
         \x20 - Codex session a1 read it 9 minutes ago\n\
         Warnings whose tasks have not ended:\n\
         \x20 - spoke5.py usually changes with hub.py, which was edited 15 minutes ago\n\
         \x20 - spoke4.py usually changes with hub.py, which was edited 16 minutes ago\n\
         \x20 - spoke3.py usually changes with hub.py, which was edited 17 minutes ago\n\
         \x20 - spoke2.py usually changes with hub.py, which was edited 18 minutes ago\n\
         \x20 - spoke1.py usually changes with hub.py, which was edited 19 minutes ago\n"
    );
    Ok(())
}
