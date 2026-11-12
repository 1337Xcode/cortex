//! What Cortex puts in front of an agent, at its fullest on fixed fixtures and at the default
//! settings, measured against a record. A change that makes any of it longer or shorter fails here
//! until the same commit updates [`RECORD`], so what every agent session pays is seen in review,
//! below the caps as well as at them. The caps themselves are asserted separately: the record is a
//! measurement to watch, not a statement of what the sizes should be.

mod common;

use common::{Repo, TestResult, function, task};
use cortex_core::{
    ContentHash, FileFlags, Identifier, InterestTier, LineRange, RepoPath, RetrievalConfig,
    ScaleMode, ToolKind, WarningKind, WarningsConfig, estimate_tokens,
};
use cortex_retrieve::{
    Basis, CallSite, Drift, EditFacts, GraphSettings, HintQuery, Item, ItemSpan, Limits,
    MAX_HINT_TOKENS, MAX_PRIMER_LANDMARKS, MAX_PRIMER_TOKENS, MAX_RECAP_TOKENS,
    MAX_STOP_REASON_TOKENS, MAX_WARNING_TOKENS, MAX_WORK_ORDER_CHARS, NamedFile, Partner, Reason,
    TaskSoFar, TestFile, WorkOrder, primer, read_graph, read_hint, recap, render, stop_reason,
    warnings_after_edit,
};

/// Each text's size in bytes and estimated tokens (bytes / 4, rounded up), as last measured.
/// Update a row in the commit that changes it, and say why in the commit.
///
/// Counted by hand when recorded: the primer is its header (71 bytes, "about 8 source files, 320
/// lines", each `·` two bytes), the landmark line (11 + 8 paths of 32 + 7 separators of 2, and its
/// line break: 282) and its closing line (91): 444 bytes, 111 tokens. The work order, at the default
/// caps (900 tokens, 3,600 bytes; two code spans), is cut until it fits: two whole 60-line spans,
/// then the second cut to 30, 15 and 8 lines, are too long; with the second taken out and the first
/// cut to 30 lines it fits. It is its header (71), "Start here:" (12) and the first item's line (5 +
/// 69 + 1: 75), one "Also likely" line (13 + 4 items of 68 + 3 separators of 2, and its line break:
/// 292), "Current code:" (14), the code's heading (40), 30 numbered lines (2 + 2 + 3 + 60 + 1: 68
/// each, 2,040) and the line saying lines 31 to 60 are not shown (28): 2,572 bytes, 643 tokens.
/// (Before code was cut rather than taken out whole, no code fitted: 433 bytes, 109 tokens.)
///
/// The paths below are `src/services/orders/handler_NN.py` (33 bytes), the edited
/// `src/services/orders/service.py` (30), its tests `tests/services/orders/test_service.py` (37)
/// and its partner `src/services/orders/models.py` (29). An edit's warnings are three lines, each
/// `[cortex] ` and a line break around its text: the drift of `create_order`, called from seven
/// files of which five are listed with their line (`path:NNN`) and then `, ...` (262 bytes); the
/// tests (89); the partner, in 8 of 11 changes (133): 484 bytes, 121 tokens. Strict mode's reason
/// is its 116-byte opening, then the files of thirty call-site warnings, the latest first, as many
/// as fit in 250 tokens: 24 (24 paths and 23 separators of 2) and ` and 6 more.` and a line break:
/// 967 bytes, 242 tokens. The recap is its header (54), the edited line naming twelve of thirteen
/// edited files and `and 1 more` (439), six call-site warnings' lines (119 each, 714) and the
/// prompt's first 200 characters in quotes (227): 1,434 bytes, 359 tokens. A read hint of a
/// 700-line file names the three functions the work order gave (161): 161 bytes, 41 tokens.
const RECORD: &[(&str, usize, usize)] = &[
    (PRIMER, 444, 111),
    (WORK_ORDER, 2_572, 643),
    (WARNINGS, 484, 121),
    (STOP_REASON, 967, 242),
    (RECAP, 1_434, 359),
    (READ_HINT, 161, 41),
];

/// The record's name for the primer.
const PRIMER: &str = "primer with eight landmarks";

/// The record's name for the work order.
const WORK_ORDER: &str = "work order at every default cap";

/// The record's name for an edit's warnings.
const WARNINGS: &str = "an edit's three warnings";

/// The record's name for strict mode's reason.
const STOP_REASON: &str = "strict mode's reason naming thirty files";

/// The record's name for the recap.
const RECAP: &str = "recap of thirteen edits, six warnings and a prompt";

/// The record's name for a read hint.
const READ_HINT: &str = "read hint naming three parts";

/// The file the warnings, the recap and the hint are about.
const EDITED: &str = "src/services/orders/service.py";

/// The prompt the recapped task began with, longer than the 200 characters a task keeps of it.
const PROMPT: &str = "Rework the order service so that every handler validates its input before it \
                      touches the database, logs the outcome, and returns a typed result; keep the \
                      public functions' names and add tests for each handler";

/// A line of code of the length real code often has: 60 characters and its line break.
const LINE: &str = "    total = compute_total(order.items, order.discounts, tax)\n";

/// The size of each text, in the record's order.
fn measured() -> TestResult<Vec<(String, usize, usize)>> {
    let primer = fullest_primer()?;
    assert!(estimate_tokens(&primer) <= MAX_PRIMER_TOKENS, "{primer}");
    let order = fullest_work_order()?;
    let tokens = usize::try_from(order.tokens)?;
    let cap = usize::try_from(RetrievalConfig::default().max_tokens)?;
    assert!(tokens <= cap, "{} tokens over {cap}", order.tokens);
    assert!(order.text.len() <= MAX_WORK_ORDER_CHARS);
    let warnings = fullest_warnings()?;
    assert!(estimate_tokens(&warnings) <= MAX_WARNING_TOKENS);
    let reason = fullest_stop_reason()?;
    assert!(estimate_tokens(&reason) <= MAX_STOP_REASON_TOKENS);
    let recap = fullest_recap()?;
    assert!(estimate_tokens(&recap) <= MAX_RECAP_TOKENS);
    let hint = fullest_hint()?;
    assert!(estimate_tokens(&hint) <= MAX_HINT_TOKENS);
    let size = |name: &str, text: &str| (name.to_owned(), text.len(), estimate_tokens(text));
    Ok(vec![
        size(PRIMER, &primer),
        (WORK_ORDER.to_owned(), order.text.len(), tokens),
        size(WARNINGS, &warnings),
        size(STOP_REASON, &reason),
        size(RECAP, &recap),
        size(READ_HINT, &hint),
    ])
}

/// The path of handler `number`, as deep as a typical service's file.
fn handler(number: u32) -> String {
    format!("src/services/orders/handler_{number:02}.py")
}

/// The warnings of an edit of [`EDITED`] that every rule has something to say about: the
/// signature of `create_order` changed and seven handlers call it, at lines 120 to 180; its tests
/// are tied to it at 0.8; `models.py` changed with it in 8 of its 11 commits.
fn fullest_warnings() -> TestResult<String> {
    let mut repo = Repo::new()?;
    let mut named = |path: &str| -> TestResult<NamedFile> {
        let nodes = repo.stored(path, "", &[], FileFlags::empty())?;
        Ok(NamedFile {
            node: *nodes.first().ok_or("a file node")?,
            path: RepoPath::new(path)?,
        })
    };
    let edited = named(EDITED)?;
    let tests = named("tests/services/orders/test_service.py")?;
    let partner = named("src/services/orders/models.py")?;
    let mut call_sites = Vec::new();
    for number in 1..=7 {
        call_sites.push(CallSite {
            file: named(&handler(number))?,
            line: 110 + 10 * number,
            confidence: 1.0,
        });
    }
    let facts = EditFacts {
        edited: edited.clone(),
        commits: 11,
        partners: vec![Partner {
            file: partner,
            co_changes: 8,
            confidence: 8.0 / 11.0,
            reverse_confidence: 0.8,
        }],
        linked: Vec::new(),
        drifts: vec![Drift {
            symbol: edited.node,
            name: "create_order".to_owned(),
            call_sites,
            unchecked: 0,
        }],
        tests: vec![TestFile {
            file: tests,
            confidence: 0.8,
        }],
        origin: None,
    };
    let warned = warnings_after_edit(
        &facts,
        &TaskSoFar::default(),
        &WarningsConfig::default(),
        ScaleMode::Medium,
    );
    warned.text.ok_or_else(|| "no warnings".into())
}

/// Strict mode's reason for a task warned, one after another, of thirty handlers that call a
/// signature it changed, none of them read or edited since.
fn fullest_stop_reason() -> TestResult<String> {
    let mut repo = Repo::new()?;
    let task = task::task(&mut repo, None)?;
    let (_, symbol) = task::file(&mut repo, EDITED)?;
    for number in 1..=30 {
        let (caller, _) = task::file(&mut repo, &handler(number))?;
        let at_ms = 10 + i64::from(number);
        task::warned(
            &mut repo,
            task,
            (WarningKind::CallSite, symbol, caller),
            at_ms,
        )?;
    }
    let reader = repo.reader()?;
    stop_reason(reader.queries(), task)?.ok_or_else(|| "no reason".into())
}

/// The recap of a task that began with [`PROMPT`], kept, edited thirteen handlers one after
/// another, and was then warned of six more that call a signature it changed.
fn fullest_recap() -> TestResult<String> {
    let mut repo = Repo::new()?;
    let task = task::task(&mut repo, Some(PROMPT))?;
    let (_, symbol) = task::file(&mut repo, EDITED)?;
    for number in 1..=13 {
        let path = handler(number);
        let (node, _) = task::file(&mut repo, &path)?;
        let at_ms = 10 + i64::from(number);
        task::acted(
            &mut repo,
            task,
            (ToolKind::Edit, &path, node),
            (at_ms, true),
        )?;
    }
    for number in 21..=26 {
        let (caller, _) = task::file(&mut repo, &handler(number))?;
        let at_ms = 100 + i64::from(number);
        task::warned(
            &mut repo,
            task,
            (WarningKind::CallSite, symbol, caller),
            at_ms,
        )?;
    }
    let reader = repo.reader()?;
    let queries = reader.queries();
    let open = queries
        .open_task(&Identifier::new(task::SESSION)?)?
        .ok_or("an open task")?;
    recap(queries, &open, (repo.root.path(), 1_048_576))?.ok_or_else(|| "no recap".into())
}

/// The hint a read of the whole of [`EDITED`], 700 lines, gets when the task's work order gave its
/// three functions: `create_order` (120 to 150), `cancel_order` (300 to 330) and `refund_order`
/// (500 to 530).
fn fullest_hint() -> TestResult<String> {
    let mut repo = Repo::new()?;
    let nodes = repo.file(
        EDITED,
        &LINE.repeat(700),
        &[
            function("create_order", 120, 150),
            function("cancel_order", 300, 330),
            function("refund_order", 500, 530),
        ],
        FileFlags::empty(),
    )?;
    let node = *nodes.first().ok_or("the file's node")?;
    let task = task::task(&mut repo, None)?;
    task::given(
        &mut repo,
        (task, node),
        &[(120, 150), (300, 330), (500, 530)],
        false,
    )?;
    let reader = repo.reader()?;
    let path = RepoPath::new(EDITED)?;
    let session = Identifier::new(task::SESSION)?;
    let query = HintQuery {
        root: repo.root.path(),
        path: &path,
        asked: None,
        session: &session,
        task: Some(task),
        max_file_bytes: 1_048_576,
    };
    read_hint(reader.queries(), &query)?.ok_or_else(|| "no hint".into())
}

/// The primer of a medium repository with as many landmarks as it names, each at a path as deep
/// as a typical service's.
fn fullest_primer() -> TestResult<String> {
    let mut repo = Repo::new()?;
    let mut landmarks = Vec::new();
    for number in 0..MAX_PRIMER_LANDMARKS {
        let path = format!("src/services/orders/handler_{number}.py");
        let nodes = repo.file(&path, &LINE.repeat(40), &[], FileFlags::empty())?;
        let file = nodes.first().copied().ok_or("the file's node")?;
        landmarks.push((file, (10.0, 0), InterestTier::Landmark));
    }
    repo.interest(&landmarks)?;
    repo.mode(ScaleMode::Medium)?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    primer(reader.queries(), &graph)?.ok_or_else(|| "no primer".into())
}

/// A work order of as many files as the defaults list, each with two spans of as many lines as a
/// span may hold, rendered at the default caps of a medium repository.
fn fullest_work_order() -> TestResult<cortex_retrieve::Rendered> {
    let defaults = RetrievalConfig::default();
    let span_lines = defaults.span_max_lines;
    let text = LINE.repeat(usize::try_from(span_lines * 2)?);
    let mut repo = Repo::new()?;
    let mut items = Vec::new();
    for number in 0..defaults.max_files {
        let path = format!("src/services/orders/handler_{number}.py");
        repo.file(&path, &text, &[], FileFlags::empty())?;
        items.push(Item {
            path: RepoPath::new(&path)?,
            content: Some(ContentHash::of(text.as_bytes())),
            spans: vec![
                ItemSpan {
                    lines: LineRange::new(1, span_lines)?,
                    name: None,
                    scope: None,
                },
                ItemSpan {
                    lines: LineRange::new(span_lines + 1, span_lines * 2)?,
                    name: None,
                    scope: None,
                },
            ],
            reason: Reason::Matches(vec!["orders".to_owned(), "total".to_owned()]),
        });
    }
    let order = WorkOrder {
        facts: Vec::new(),
        hint: None,
        basis: Basis::Agreement(vec!["the prompt's words", "the commit history"]),
        items,
    };
    let limits = Limits {
        max_tokens: defaults.max_tokens,
        max_files: usize::try_from(defaults.max_files)?,
        max_code_spans: usize::try_from(defaults.max_code_spans)?,
        span_max_lines: defaults.span_max_lines,
        max_file_bytes: 1_048_576,
    };
    render(&order, repo.root.path(), &limits).ok_or_else(|| "no work order".into())
}

/// Every text is the size the record says.
#[test]
fn what_agents_are_given_is_the_size_recorded() -> TestResult {
    let measured = measured()?;
    let recorded: Vec<(String, usize, usize)> = RECORD
        .iter()
        .map(|(name, bytes, tokens)| ((*name).to_owned(), *bytes, *tokens))
        .collect();
    assert_eq!(
        measured, recorded,
        "a text Cortex gives agents changed size; if that is intended, update RECORD in \
         crates/cortex-retrieve/tests/context_sizes.rs in the same commit"
    );
    Ok(())
}
