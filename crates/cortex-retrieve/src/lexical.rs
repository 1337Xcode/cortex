//! The prompt's words looked up in the search index: which nodes they match, how strongly, and
//! how well each file matches the prompt as a whole.
//!
//! The search index holds, for each node, the words of its name and path. The prompt's words are
//! looked up with BM25, the 50 best rows kept, each weighed by its score over the best's. A word
//! more than one row in twenty holds (and more than twenty rows) is too common to tell anything
//! apart and is not looked up, though it still counts in how well a file matches. A path or a name the prompt writes exactly,
//! and finds in the repository, is a seed of full weight: the file, the symbol at the line it gives,
//! the symbols of that name (when no more than five share it).
//!
//! How well a file matches is the share of the prompt's words it holds, each word weighed by how
//! rare it is in the index (its inverse document frequency, as BM25 weighs it): the words of the
//! file's own row and of its symbols that matched, over every prompt word some row holds. So that
//! one common word cannot make a perfect match, the whole is never taken below twice the weight of
//! a word one row in a hundred holds. A file the prompt names exactly matches fully. The top
//! lexical score is the best file's.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Identifier, RepoPath};
use cortex_store::{NodeId, Queries, StoreError};

use crate::graph::Graph;
use crate::prompt::PromptTerms;

/// Most rows a lookup keeps, the best by BM25.
pub const MAX_HITS: usize = 50;

/// The share of rows a word may be held by and still be looked up: past it, a word sorts nothing
/// from anything.
const COMMON_SHARE: f64 = 0.05;

/// The rows a word may always be held by and be looked up, however few rows there are: in a small
/// repository a word twenty rows hold still tells places apart.
const MIN_COMMON_ROWS: u64 = 20;

/// Most words one lookup asks for, the rarest first.
const MAX_LOOKED_UP: usize = 24;

/// The share of rows whose word sets the floor a file's match is taken over: twice the weight of a
/// word one row in a hundred holds.
const FLOOR_SHARE: f64 = 0.01;

/// Most symbols one name may name and still be a seed: past it, the name is not one place.
const MAX_NAMED_SYMBOLS: usize = 5;

/// How one file matches the prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct FileMatch {
    /// The share of the prompt's words it holds, weighed by rarity, from 0 to 1.
    pub coverage: f64,
    /// The prompt's words it holds, in the prompt's order.
    pub words: Vec<String>,
    /// Whether the prompt names it, or a symbol in it, exactly.
    pub is_named: bool,
}

/// What the prompt's words found.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LexicalMatch {
    /// Each node found, with its weight from 0 to 1, by node.
    pub seeds: BTreeMap<u32, f64>,
    /// Each file found, by its node, with how it matches.
    pub files: BTreeMap<u32, FileMatch>,
    /// The best file's coverage: 0 when nothing matched.
    pub top_lexical: f64,
}

/// What `terms` find in the store `queries` reads, as nodes of `graph`.
///
/// # Errors
///
/// The store's errors.
pub fn lexical(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    terms: &PromptTerms,
) -> Result<LexicalMatch, StoreError> {
    let rows = graph.len().max(1);
    let mut held: Vec<(String, u64)> = Vec::new();
    for word in &terms.words {
        let count = queries.term_rows(word)?;
        if count > 0 {
            held.push((word.clone(), count));
        }
    }
    let weight_of = |count: u64| idf(rows, count);
    let total: f64 = held.iter().map(|(_, count)| weight_of(*count)).sum();
    let floor = 2.0 * weight_of(share_of(rows, FLOOR_SHARE));
    let whole = total.max(floor);
    let mut found = LexicalMatch::default();
    let hits = looked_up(queries, &held, rows)?;
    let best = hits.iter().map(|(_, score)| *score).fold(0.0_f64, f64::max);
    let mut hit_nodes: BTreeSet<NodeId> = BTreeSet::new();
    for (node, score) in &hits {
        if let Some(index) = graph.index_of(*node) {
            if best > 0.0 {
                raise(&mut found.seeds, index, score / best);
            }
            hit_nodes.insert(*node);
        }
    }
    let named = named(queries, graph, terms)?;
    for (node, index) in &named.nodes {
        raise(&mut found.seeds, *index, 1.0);
        hit_nodes.insert(*node);
    }
    let listed: Vec<NodeId> = hit_nodes
        .iter()
        .copied()
        .chain(files_of(graph, &hit_nodes))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut tokens_of_file: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    for (node, tokens) in queries.search_tokens_of(&listed)? {
        let Some(file) = graph.index_of(node).and_then(|index| graph.file_of(index)) else {
            continue;
        };
        tokens_of_file
            .entry(file)
            .or_default()
            .extend(tokens.split(' ').map(str::to_owned));
    }
    for (file, tokens) in tokens_of_file {
        let words: Vec<String> = held
            .iter()
            .filter(|(word, _)| tokens.contains(word))
            .map(|(word, _)| word.clone())
            .collect();
        let matched: f64 = held
            .iter()
            .filter(|(word, _)| tokens.contains(word))
            .map(|(_, count)| weight_of(*count))
            .sum();
        let is_named = named.files.contains(&file);
        let coverage = if is_named {
            1.0
        } else if whole > 0.0 {
            (matched / whole).min(1.0)
        } else {
            0.0
        };
        found.files.insert(
            file,
            FileMatch {
                coverage,
                words,
                is_named,
            },
        );
    }
    for file in &named.files {
        found.files.entry(*file).or_insert_with(|| FileMatch {
            coverage: 1.0,
            words: Vec::new(),
            is_named: true,
        });
    }
    found.top_lexical = found
        .files
        .values()
        .map(|file| file.coverage)
        .fold(0.0, f64::max);
    Ok(found)
}

/// The rows `held`, each a word and how many rows hold it, find: the uncommon words, the rarest
/// first, at most [`MAX_LOOKED_UP`], looked up together, the best [`MAX_HITS`] rows with their BM25
/// scores, best first.
fn looked_up(
    queries: Queries<'_>,
    held: &[(String, u64)],
    rows: usize,
) -> Result<Vec<(NodeId, f64)>, StoreError> {
    let common = share_of(rows, COMMON_SHARE).max(MIN_COMMON_ROWS);
    let mut uncommon: Vec<&(String, u64)> =
        held.iter().filter(|(_, count)| *count <= common).collect();
    uncommon.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
    uncommon.truncate(MAX_LOOKED_UP);
    if uncommon.is_empty() {
        return Ok(Vec::new());
    }
    let words: Vec<&str> = uncommon.iter().map(|(word, _)| word.as_str()).collect();
    queries.search(&words, MAX_HITS)
}

/// The nodes the prompt's mentions name exactly, with their numbers in `graph`, and the files
/// those nodes are in.
#[derive(Debug, Default)]
struct Named {
    /// Each node named, with its number.
    nodes: BTreeMap<NodeId, u32>,
    /// The files named, or holding a symbol named, by number.
    files: BTreeSet<u32>,
}

/// What the mentions of `terms` name exactly in the store `queries` reads: a path's file and the
/// symbol at its line, or a name's symbols when there are few of them.
fn named(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    terms: &PromptTerms,
) -> Result<Named, StoreError> {
    let mut named = Named::default();
    for mention in &terms.mentions {
        if let Some(path) = &mention.path
            && let Some((file, node)) = file_named(queries, path)?
        {
            add(&mut named, graph, node);
            if let Some(line) = mention.line
                && let Some(symbol) = symbol_at(queries, file, line)?
            {
                add(&mut named, graph, symbol);
            }
            continue;
        }
        let Some(name) = mention
            .name
            .as_deref()
            .and_then(|name| Identifier::new(name).ok())
        else {
            continue;
        };
        let symbols = queries.symbol_nodes_named(&name, MAX_NAMED_SYMBOLS + 1)?;
        if symbols.len() <= MAX_NAMED_SYMBOLS {
            for symbol in symbols {
                add(&mut named, graph, symbol);
            }
        }
    }
    Ok(named)
}

/// Adds `node` to what is named, with its file.
fn add(named: &mut Named, graph: &Graph<NodeId>, node: NodeId) {
    let Some(index) = graph.index_of(node) else {
        return;
    };
    named.nodes.insert(node, index);
    if let Some(file) = graph.file_of(index) {
        named.files.insert(file);
    }
}

/// The one file `written`, a path as a prompt wrote it, names: the file at that path; else the one
/// file whose path ends with it after a `/`; else, for a path longer than any the repository holds
/// (an absolute one), the one file whose path it ends with after a `/`. `None` when no file or more
/// than one fits.
fn file_named(
    queries: Queries<'_>,
    written: &str,
) -> Result<Option<(cortex_store::FileId, NodeId)>, StoreError> {
    let trimmed = written.trim_start_matches('/');
    if let Ok(path) = RepoPath::new(trimmed)
        && let Some(row) = queries.file(&path)?
        && let Some(node) = queries.file_node_at(&path)?
    {
        return Ok(Some((row.id, node)));
    }
    let Some(base) = trimmed.rsplit('/').next().filter(|base| !base.is_empty()) else {
        return Ok(None);
    };
    let candidates = queries.files_named(base)?;
    let fits = |path: &str| {
        path == trimmed
            || path.ends_with(&format!("/{trimmed}"))
            || trimmed.ends_with(&format!("/{path}"))
    };
    let mut fitting = candidates
        .into_iter()
        .filter(|(path, _, _)| fits(path.as_str()));
    match (fitting.next(), fitting.next()) {
        (Some((_, file, node)), None) => Ok(Some((file, node))),
        _ => Ok(None),
    }
}

/// The innermost symbol of `file` whose lines hold `line`, if any.
fn symbol_at(
    queries: Queries<'_>,
    file: cortex_store::FileId,
    line: u32,
) -> Result<Option<NodeId>, StoreError> {
    Ok(queries
        .symbol_spans_in_file(file)?
        .into_iter()
        .filter(|span| span.start_line <= line && line <= span.end_line)
        .min_by_key(|span| (span.end_line - span.start_line, span.node))
        .map(|span| span.node))
}

/// The files of `nodes`, those that are files themselves included.
fn files_of<'a>(
    graph: &'a Graph<NodeId>,
    nodes: &'a BTreeSet<NodeId>,
) -> impl Iterator<Item = NodeId> + 'a {
    nodes.iter().filter_map(|node| {
        let index = graph.index_of(*node)?;
        graph.id(graph.file_of(index)?)
    })
}

/// Raises `node`'s weight in `seeds` to `weight` when that is more than it has.
fn raise(seeds: &mut BTreeMap<u32, f64>, node: u32, weight: f64) {
    let held = seeds.entry(node).or_insert(0.0);
    *held = held.max(weight);
}

/// How many of `rows` rows `share` of them is, at least one.
fn share_of(rows: usize, share: f64) -> u64 {
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a count of rows is far below where a float loses whole numbers, and the share is \
                  between 0 and 1"
    )]
    let count = (rows as f64 * share).ceil() as u64;
    count.max(1)
}

/// The inverse document frequency of a word `count` of `rows` rows hold, as BM25 weighs it:
/// ln(1 + (rows - count + 0.5) / (count + 0.5)).
fn idf(rows: usize, count: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "counts of rows are far below where a float loses whole numbers"
    )]
    let (rows, count) = (rows as f64, count as f64);
    (1.0 + (rows - count + 0.5) / (count + 0.5)).ln()
}
