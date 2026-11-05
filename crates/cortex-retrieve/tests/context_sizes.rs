//! What Cortex puts in front of an agent, at its fullest on fixed fixtures and at the default
//! settings, measured against a record. A change that makes any of it longer or shorter fails here
//! until the same commit updates [`RECORD`], so what every agent session pays is seen in review,
//! below the caps as well as at them. The caps themselves are asserted separately: the record is a
//! measurement to watch, not a statement of what the sizes should be.

mod common;

use common::{Repo, TestResult};
use cortex_core::{
    ContentHash, FileFlags, InterestTier, LineRange, RepoPath, RetrievalConfig, ScaleMode,
    estimate_tokens,
};
use cortex_retrieve::{
    Basis, GraphSettings, Item, ItemSpan, Limits, MAX_PRIMER_LANDMARKS, MAX_PRIMER_TOKENS,
    MAX_WORK_ORDER_CHARS, Reason, WorkOrder, primer, read_graph, render,
};

/// Each text's size in bytes and estimated tokens (bytes / 4, rounded up), as last measured.
/// Update a row in the commit that changes it, and say why in the commit.
///
/// Counted by hand when recorded: the primer is its header (71 bytes, "about 8 source files, 320
/// lines", each `·` two bytes), the landmark line (11 + 8 paths of 32 + 7 separators of 2, and its
/// line break: 282) and its closing line (91): 444 bytes, 111 tokens. The work order is its header
/// (71) and one "Also likely" line (13 + 5 items of 68 + 4 separators of 2, and its line break:
/// 362): 433 bytes, 109 tokens. At the default caps two 60-line spans of 60-byte lines leave no
/// room for code, so every file is listed by path and lines alone.
const RECORD: &[(&str, usize, usize)] = &[(PRIMER, 444, 111), (WORK_ORDER, 433, 109)];

/// The record's name for the primer.
const PRIMER: &str = "primer with eight landmarks";

/// The record's name for the work order.
const WORK_ORDER: &str = "work order at every default cap";

/// A line of code of the length real code often has: 60 bytes with its line break.
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
    Ok(vec![
        (PRIMER.to_owned(), primer.len(), estimate_tokens(&primer)),
        (WORK_ORDER.to_owned(), order.text.len(), tokens),
    ])
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
                },
                ItemSpan {
                    lines: LineRange::new(span_lines + 1, span_lines * 2)?,
                    name: None,
                },
            ],
            reason: Reason::Matches(vec!["orders".to_owned(), "total".to_owned()]),
        });
    }
    let order = WorkOrder {
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
