//! A commit read as a card for the store: its record from `git show -s`, the paths it changed
//! from the history read in-process (as the miner reads a commit), whether it only moved white
//! space (asked of git only when the card would otherwise be a normal one), the file nodes of the
//! paths it changed and those of them the index classes as tests, and its terms. The subject and
//! the reason line go to the text cache and nowhere else; the store is given numbers, ids and the
//! words the terms are.

use std::collections::BTreeSet;

use cortex_core::{RepoPath, SymbolKind, identifier_tokens};
use cortex_git::card::{CardContext, ChangeCard, ChangeKind, ChangedPaths, read_card};
use cortex_git::{HistorySource, PathChange, Window};
use cortex_index::is_test_path;
use cortex_store::{CardKind, ChangeCardRecord, MAX_CARD_NODES, NodeId, Queries};

use super::repository::Git;
use super::texts::CardText;

/// What carding reads besides the commit: the settings that decide its kind.
#[derive(Debug, Clone)]
pub(super) struct CardSettings {
    /// `git.max_files_per_commit`: a commit that changed more paths is a mass change.
    pub(super) max_files: usize,
    /// `history.bot_patterns`: the names of accounts that are bots.
    pub(super) bot_patterns: Vec<String>,
}

/// What a card is made with besides git and the store.
pub(super) struct CardRun<'a> {
    /// The settings that decide its kind.
    pub(super) settings: &'a CardSettings,
    /// When it is made, in UTC milliseconds.
    pub(super) now_ms: i64,
    /// Whether the next git process may start: false once the worker is told to stop.
    pub(super) may_go_on: &'a dyn Fn() -> bool,
}

/// Why a commit was not carded.
#[derive(Debug)]
pub(super) enum NotCarded {
    /// The worker was told to stop between two git processes.
    Stopped,
    /// Git or the store failed, in words that name the commit and never its text.
    Failed(String),
}

/// A commit carded: what the store keeps, the words to match it by, and its text for the cache.
pub(super) struct NewCard {
    /// The card as the store keeps it.
    pub(super) record: ChangeCardRecord,
    /// Its terms: the subject's and the reason line's words, split as identifiers are.
    pub(super) tokens: Vec<String>,
    /// Its subject and reason line, for the text cache only.
    pub(super) text: CardText,
}

impl std::fmt::Debug for NewCard {
    /// The card, without its terms or text: they are a person's words, which a debug line could
    /// carry into a log.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NewCard")
            .field("record", &self.record)
            .finish_non_exhaustive()
    }
}

/// The card of the commit `oid`, read with `git` and the index `queries` reads, made at `now_ms`;
/// when `is_boundary`, only when the commit is a root (blame stopped at any other boundary at its
/// date limit, and the commit's runs are not kept), none otherwise.
///
/// The card's symbols are empty: the symbols a commit changed would come from its diff against
/// the symbol table, which nothing reads yet.
///
/// # Errors
///
/// A stop before one of its git processes; git cannot read the commit or the paths it changed,
/// its record is not one a card reads, or the store cannot be read.
pub(super) fn card(
    git: &Git<'_>,
    queries: Queries<'_>,
    (oid, is_boundary): (&str, bool),
    run: &CardRun<'_>,
) -> Result<Option<NewCard>, NotCarded> {
    let settings = run.settings;
    if !(run.may_go_on)() {
        return Err(NotCarded::Stopped);
    }
    let show = git
        .reader
        .record(oid)
        .map_err(|error| NotCarded::Failed(format!("reading commit {oid}: {error}")))?;
    if is_boundary && has_parent(&show) {
        return Ok(None);
    }
    let changes = changed_paths(git, oid, settings.max_files).map_err(NotCarded::Failed)?;
    let mut context = CardContext {
        changed_paths: changes
            .as_ref()
            .map_or(ChangedPaths::OverMinerLimit, |paths| {
                ChangedPaths::Count(paths.len())
            }),
        max_files: settings.max_files,
        is_cosmetic: false,
        bot_patterns: &settings.bot_patterns,
    };
    let mut card = read(&show, &context, oid).map_err(NotCarded::Failed)?;
    // Whether the commit only moved white space is a diff of the whole commit: asked only when it
    // decides the kind.
    if card.kind == ChangeKind::Normal {
        if !(run.may_go_on)() {
            return Err(NotCarded::Stopped);
        }
        context.is_cosmetic = git
            .reader
            .is_cosmetic(oid)
            .map_err(|error| NotCarded::Failed(format!("comparing commit {oid}: {error}")))?;
        if context.is_cosmetic {
            card = read(&show, &context, oid).map_err(NotCarded::Failed)?;
        }
    }
    let kind = card_kind(card.kind);
    let (files, tests) = match (kind, changes) {
        (CardKind::Mass, _) | (_, None) => (Vec::new(), Vec::new()),
        (_, Some(paths)) => file_nodes(queries, &paths).map_err(|error| {
            NotCarded::Failed(format!("reading the files of commit {oid}: {error}"))
        })?,
    };
    Ok(Some(new_card(card, kind, (files, tests), run.now_ms)))
}

/// Whether the record `show`, as [`cortex_git::card::SHOW_FORMAT`] prints it, names a parent: its
/// second line lists the parents.
fn has_parent(show: &str) -> bool {
    show.lines()
        .nth(1)
        .is_some_and(|parents| !parents.trim().is_empty())
}

/// The subject and reason line of the commit `oid`, read again for the text cache: its kind is
/// not asked, so no path is read and no comparison run.
///
/// # Errors
///
/// Git cannot read the commit, or its record is not one a card reads.
pub(super) fn text(git: &Git<'_>, oid: &str, settings: &CardSettings) -> Result<CardText, String> {
    let show = git
        .reader
        .record(oid)
        .map_err(|error| format!("reading commit {oid}: {error}"))?;
    let context = CardContext {
        changed_paths: ChangedPaths::Count(0),
        max_files: settings.max_files,
        is_cosmetic: false,
        bot_patterns: &settings.bot_patterns,
    };
    let card = read(&show, &context, oid)?;
    Ok(CardText {
        subject: card.subject,
        reason: card.reason,
    })
}

/// `show` read as a card under `context`, for the commit `oid`.
fn read(show: &str, context: &CardContext<'_>, oid: &str) -> Result<ChangeCard, String> {
    read_card(show, context).map_err(|error| format!("reading commit {oid}'s card: {error}"))
}

/// The paths the commit `oid` changed against its first parent, as the miner reads a commit;
/// none when it changed more than `max_files`.
fn changed_paths(
    git: &Git<'_>,
    oid: &str,
    max_files: usize,
) -> Result<Option<Vec<PathChange>>, String> {
    let window = Window {
        max_commits: 1,
        oldest_s: i64::MIN,
        max_files,
        stop_at: None,
        from: Some(oid.to_owned()),
    };
    let mined = git
        .history
        .read(&window)
        .map_err(|error| format!("reading the paths commit {oid} changed: {error}"))?;
    let commit = mined
        .commits
        .into_iter()
        .find(|commit| commit.id == oid)
        .ok_or_else(|| format!("commit {oid} was not read"))?;
    Ok(commit.changes)
}

/// The nodes of the files at `paths` that the index holds, by node, and those of them that are
/// tests, each list at most [`MAX_CARD_NODES`] long. A path no file has now (deleted or moved
/// since) has no node.
fn file_nodes(
    queries: Queries<'_>,
    paths: &[PathChange],
) -> Result<(Vec<NodeId>, Vec<NodeId>), cortex_store::StoreError> {
    let mut files = BTreeSet::new();
    let mut tests = BTreeSet::new();
    for change in paths {
        let Some((node, is_test)) = file_node(queries, &change.path)? else {
            continue;
        };
        files.insert(node);
        if is_test {
            tests.insert(node);
        }
    }
    let capped = |nodes: BTreeSet<NodeId>| nodes.into_iter().take(MAX_CARD_NODES).collect();
    Ok((capped(files), capped(tests)))
}

/// The node of the file at `path`, and whether it is a test: the index's rules for test files
/// say so of its path and language (a test folder, a test file's name), or the index found a
/// test in it.
fn file_node(
    queries: Queries<'_>,
    path: &RepoPath,
) -> Result<Option<(NodeId, bool)>, cortex_store::StoreError> {
    let Some(row) = queries.file(path)? else {
        return Ok(None);
    };
    let Some(node) = queries.file_node(row.id)? else {
        return Ok(None);
    };
    let is_test = is_test_path(path, row.file.language)
        || queries
            .symbols_in_file(row.id)?
            .iter()
            .any(|symbol| symbol.kind == SymbolKind::Test);
    Ok(Some((node, is_test)))
}

/// The store's name for `kind`.
fn card_kind(kind: ChangeKind) -> CardKind {
    match kind {
        ChangeKind::Normal => CardKind::Normal,
        ChangeKind::Trivial => CardKind::Trivial,
        ChangeKind::Mass => CardKind::Mass,
        ChangeKind::Bot => CardKind::Bot,
    }
}

/// `card` as the store keeps it, of `kind`, with its file and test nodes, made at `now_ms`, with
/// its terms and its text.
fn new_card(
    card: ChangeCard,
    kind: CardKind,
    (files, tests): (Vec<NodeId>, Vec<NodeId>),
    now_ms: i64,
) -> NewCard {
    let mut tokens = identifier_tokens(&card.subject);
    if let Some(reason) = &card.reason {
        tokens.extend(identifier_tokens(reason));
    }
    // INVARIANT: no-content-stored. The record has no field for the subject or the reason line:
    // they leave here in the text, which only the in-memory cache holds.
    let record = ChangeCardRecord {
        first_parent: card.parents.first().cloned(),
        committed_at_s: card.time_s,
        kind,
        pull_request: card.pull_request,
        issues: card.issues,
        has_reason: card.reason.is_some(),
        files,
        symbols: Vec::new(),
        tests,
        carded_at_ms: now_ms,
        oid: card.oid,
    };
    NewCard {
        record,
        tokens,
        text: CardText {
            subject: card.subject,
            reason: card.reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record's second line lists the parents: one or two ids is a parent, an empty line a root.
    #[test]
    fn a_record_says_whether_the_commit_has_a_parent() {
        let id = "a".repeat(40);
        let root = format!("{id}\n\n1\nA\nA\nsubject\n");
        let child = format!("{id}\n{id}\n1\nA\nA\nsubject\n");
        let merge = format!("{id}\n{id} {id}\n1\nA\nA\nsubject\n");
        assert!(!has_parent(&root));
        assert!(has_parent(&child));
        assert!(has_parent(&merge));
    }
}
