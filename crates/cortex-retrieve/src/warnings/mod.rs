//! Warnings after an edit: the change the edit seems to leave unfinished, told to the agent while
//! it can still finish it. Each rule reads what the store and the task say about the edited file
//! and proposes warnings with a strength; this module keeps the strongest few, never a pair the
//! task was warned about already, and renders them within their token cap.
//!
//! The rules are a table, one file each: a co-change companion not touched yet, call sites of a
//! symbol whose signature changed, and the edited file's tests. Adding a rule is a file and a line
//! in [`RULES`]. In tiny mode only the call-site rule runs: with so little history the others guess.

mod call_site;
mod cochange;
mod facts;
mod test_companion;

use std::collections::BTreeSet;

use cortex_core::{RepoPath, ScaleMode, WarningKind, WarningsConfig, estimate_tokens};
use cortex_store::NodeId;

pub use call_site::MAX_LISTED_CALL_SITES;
pub use cochange::{MIN_LINK_TASKS, MIN_LINK_WEIGHT};
pub use facts::{EditQuery, MAX_CHECKED_CALLER_FILES, edit_facts};
pub use test_companion::MAX_LISTED_TESTS;

/// The most tokens the warnings of one edit take together, as estimated.
pub const MAX_WARNING_TOKENS: usize = 250;

/// A file a warning may name: its node and its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedFile {
    /// The file's node.
    pub node: NodeId,
    /// The file's path.
    pub path: RepoPath,
}

/// A file that changed in the same commits as the edited one.
#[derive(Debug, Clone, PartialEq)]
pub struct Partner {
    /// The file.
    pub file: NamedFile,
    /// How many commits changed both.
    pub co_changes: u32,
    /// The share of the edited file's commits, by weight, that also changed this one, from 0 to 1.
    pub confidence: f64,
}

/// A file agents worked on together with the edited one.
#[derive(Debug, Clone, PartialEq)]
pub struct Linked {
    /// The file.
    pub file: NamedFile,
    /// The link's weight after fading to the current task.
    pub weight: f64,
    /// How many tasks drew the link.
    pub tasks: u32,
}

/// Code in another file that calls a symbol, as the index tied it.
#[derive(Debug, Clone, PartialEq)]
pub struct CallSite {
    /// The file holding the call, as the index read it: its bytes on disk checked unchanged.
    pub file: NamedFile,
    /// The call's line.
    pub line: u32,
    /// How far the reference's tie to the symbol can be trusted, from 0 to 1.
    pub confidence: f64,
}

/// A symbol of the edited file whose signature the edit changed, and where code calls it.
#[derive(Debug, Clone, PartialEq)]
pub struct Drift {
    /// The symbol's node.
    pub symbol: NodeId,
    /// Its name.
    pub name: String,
    /// The calls to it, in path and line order.
    pub call_sites: Vec<CallSite>,
    /// How many more calls the index holds in files not checked against the disk, which the
    /// count can only say there are at least.
    pub unchecked: u32,
}

/// A test file of the edited file: one that imports it or is named for it.
#[derive(Debug, Clone, PartialEq)]
pub struct TestFile {
    /// The file.
    pub file: NamedFile,
    /// How far the tie between the test and the edited file can be trusted, from 0 to 1.
    pub confidence: f64,
}

/// What the store says about an edited file, as the rules read it.
#[derive(Debug, Clone, PartialEq)]
pub struct EditFacts {
    /// The edited file.
    pub edited: NamedFile,
    /// How many commits changed it.
    pub commits: u32,
    /// The files that changed with it, by co-change.
    pub partners: Vec<Partner>,
    /// The files agents worked on together with it.
    pub linked: Vec<Linked>,
    /// Its symbols whose signature the edit changed.
    pub drifts: Vec<Drift>,
    /// Its test files.
    pub tests: Vec<TestFile>,
}

/// What the task did so far, as the rules read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskSoFar {
    /// The files the task read or edited.
    pub touched: BTreeSet<NodeId>,
    /// The files the task edited.
    pub edited: BTreeSet<NodeId>,
    /// Each pair the task was warned about, with the kind of warning: what it was about, then the
    /// file it named.
    pub warned: BTreeSet<(WarningKind, NodeId, NodeId)>,
}

impl TaskSoFar {
    /// Whether a warning of `kind` about `source` may name `target`: the task was not warned of
    /// that pair yet.
    fn is_new(&self, kind: WarningKind, source: NodeId, target: NodeId) -> bool {
        !self.warned.contains(&(kind, source, target))
    }
}

/// One warning a rule proposes: what it is about, the files it names, how strong its evidence is
/// and its line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    /// What it points out.
    pub kind: WarningKind,
    /// What it is about: the edited file, or the symbol whose signature changed.
    pub source: NodeId,
    /// The files it names, each at most once.
    pub targets: Vec<NodeId>,
    /// How strong its evidence is, from 0 to 1.
    pub strength: f64,
    /// Its text, one line with no line break.
    pub line: String,
}

/// One rule: the kind of warning it gives, whether it runs in tiny mode, and how it finds them.
struct Rule {
    /// The kind of warning it gives.
    kind: WarningKind,
    /// Whether it runs on a tiny repository, where history and past work are too thin to go on.
    is_on_when_tiny: bool,
    /// Its warnings for an edit.
    find: fn(&EditFacts, &TaskSoFar, &WarningsConfig) -> Vec<Warning>,
}

/// Every rule.
const RULES: &[Rule] = &[
    Rule {
        kind: WarningKind::CoChange,
        is_on_when_tiny: false,
        find: cochange::warnings,
    },
    Rule {
        kind: WarningKind::CallSite,
        is_on_when_tiny: true,
        find: call_site::warnings,
    },
    Rule {
        kind: WarningKind::Test,
        is_on_when_tiny: false,
        find: test_companion::warnings,
    },
];

/// The warnings an edit gets and their text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Warned {
    /// The warnings given, the strongest first.
    pub given: Vec<Warning>,
    /// Their text, one line each; none when no warning is given.
    pub text: Option<String>,
}

/// The warnings the edit `facts` describes gets in the task `task`: every rule's that runs in
/// `mode`, the strongest first, at most `config.max_per_edit` of them and no more than fit in
/// [`MAX_WARNING_TOKENS`], each a line of the text.
#[must_use]
pub fn warnings_after_edit(
    facts: &EditFacts,
    task: &TaskSoFar,
    config: &WarningsConfig,
    mode: ScaleMode,
) -> Warned {
    let mut found: Vec<Warning> = RULES
        .iter()
        .filter(|rule| rule.is_on_when_tiny || mode != ScaleMode::Tiny)
        .flat_map(|rule| (rule.find)(facts, task, config))
        .collect();
    // INVARIANT: deterministic (a total order: strength, then kind, what the warning is about and
    // the files it names)
    found.sort_by(|left, right| {
        right
            .strength
            .total_cmp(&left.strength)
            .then_with(|| kind_rank(left.kind).cmp(&kind_rank(right.kind)))
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.targets.cmp(&right.targets))
    });
    found.truncate(usize::try_from(config.max_per_edit).unwrap_or(usize::MAX));
    // INVARIANT: budgeted (the weakest warnings go until the rest fit the cap)
    while !found.is_empty() && estimate_tokens(&text_of(&found)) > MAX_WARNING_TOKENS {
        found.pop();
    }
    let text = (!found.is_empty()).then(|| text_of(&found));
    Warned { given: found, text }
}

/// A kind's place among the rules, for ordering warnings of equal strength.
fn kind_rank(kind: WarningKind) -> usize {
    RULES
        .iter()
        .position(|rule| rule.kind == kind)
        .unwrap_or(RULES.len())
}

/// The text of `warnings`, one line each.
fn text_of(warnings: &[Warning]) -> String {
    let mut text = String::new();
    for warning in warnings {
        text.push_str("[cortex] ");
        text.push_str(&warning.line);
        text.push('\n');
    }
    text
}
