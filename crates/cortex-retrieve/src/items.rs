//! The files a work order lists: the best-scoring files, each with its spans and the strongest true
//! reason for listing it, taken down the ranking until the list is full.
//!
//! A file is passed over when every span it would show is already in the session's context, and
//! when nothing true can be said of why it is listed. At most four files are looked at for each
//! one the list may hold, so a ranking full of files with nothing to say costs the prompt little.

use cortex_core::EdgeType;
use cortex_store::{FileId, FileRow, NodeId, Queries, StoreError};

use crate::graph::Graph;
use crate::reasons::{ItemEvidence, Reason, reason_of};
use crate::render::{Item, ItemSpan};
use crate::score::ScoredFile;
use crate::seeds::Seeds;
use crate::spans::{Outlined, Span, is_in_context, outline_of, spans_of};
use crate::support::{STRUCTURAL_FLOOR, Support};

/// Most files looked at for each file the list may hold.
pub const LOOKED_AT_PER_LISTED: usize = 4;

/// What the listing reads from.
#[derive(Debug, Clone, Copy)]
pub struct Listing<'a> {
    /// The store.
    pub queries: Queries<'a>,
    /// The graph.
    pub graph: &'a Graph<NodeId>,
    /// What each source seeded.
    pub seeds: &'a Seeds,
    /// The evidence behind each file.
    pub support: &'a Support<'a, NodeId>,
}

/// One file listed: its node, its spans as nodes and lines, and the item the work order writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    /// The file, by node.
    pub file: u32,
    /// Its spans, the strongest first.
    pub spans: Vec<Span>,
    /// What the work order writes of it.
    pub item: Item,
    /// Its spans whose code the work order wrote, once it is written; none before.
    pub shown: Vec<Span>,
}

/// The files to list from `scored`, the best first, at most `max_files`, each span at most
/// `span_max_lines` lines.
///
/// # Errors
///
/// The store's errors.
pub fn listed(
    listing: &Listing<'_>,
    scored: &[ScoredFile],
    (max_files, span_max_lines): (usize, u32),
) -> Result<Vec<Listed>, StoreError> {
    let looked_at: Vec<&ScoredFile> = scored
        .iter()
        .take(max_files.saturating_mul(LOOKED_AT_PER_LISTED))
        .collect();
    let ids: Vec<NodeId> = looked_at
        .iter()
        .filter_map(|file| listing.graph.id(file.file))
        .collect();
    let rows = listing.queries.files_of_nodes(&ids)?;
    let mut found = Vec::new();
    for file in looked_at {
        if found.len() >= max_files {
            break;
        }
        let Some(id) = listing.graph.id(file.file) else {
            continue;
        };
        let Some((_, row)) = rows.iter().find(|(node, _)| *node == id) else {
            continue;
        };
        if let Some(entry) = listed_file(listing, (file, row), span_max_lines)? {
            found.push(entry);
        }
    }
    Ok(found)
}

/// `file`, stored as `row`, as the list holds it: its spans out of the session's context and its
/// reason; none when its every span is in context or nothing true can be said of it.
fn listed_file(
    listing: &Listing<'_>,
    (file, row): (&ScoredFile, &FileRow),
    span_max_lines: u32,
) -> Result<Option<Listed>, StoreError> {
    let outline = outline_of(listing.queries, listing.graph, (file, row))?;
    // A count past `u32` is past any cap on lines, which is all `spans_of` asks of it.
    let line_count = u32::try_from(row.file.line_count.unwrap_or(0)).unwrap_or(u32::MAX);
    let spans = spans_of(&outline, line_count, span_max_lines);
    let held = row.file.content_hash.map(|hash| hash.short());
    let path = row.file.path.as_str();
    let kept: Vec<Span> = spans
        .iter()
        .filter(|span| {
            // A span the agent read, or a work order of the session showed, unchanged since
            // the latest compaction is in its context: it is not shown again.
            !held.is_some_and(|content| {
                let session = &listing.seeds.session;
                is_in_context(span, (path, content), &session.in_context)
                    || is_in_context(span, (path, content), &session.shown)
            })
        })
        .copied()
        .collect();
    if !spans.is_empty() && kept.is_empty() {
        return Ok(None);
    }
    let Some(reason) = reason_for(listing, (file.file, row))? else {
        return Ok(None);
    };
    let item = Item {
        path: row.file.path.clone(),
        content: row.file.content_hash,
        spans: kept.iter().map(|span| item_span(span, &outline)).collect(),
        reason,
    };
    Ok(Some(Listed {
        file: file.file,
        spans: kept,
        item,
        shown: Vec::new(),
    }))
}

/// `span` as the work order writes it: its lines, the name of the symbol it is for, and the first
/// line of the definition around that symbol when it starts before the span.
fn item_span(span: &Span, outline: &[Outlined]) -> ItemSpan {
    let symbol = span
        .symbol
        .and_then(|node| outline.iter().find(|symbol| symbol.node == node));
    ItemSpan {
        lines: span.lines,
        name: symbol.map(|symbol| symbol.name.clone()),
        scope: symbol
            .and_then(|symbol| symbol.parent)
            .and_then(|parent| outline.get(parent))
            .map(|parent| parent.lines.start_line())
            .filter(|line| *line < span.lines.start_line()),
    }
}

/// The strongest true reason to list `file`, stored as `row`: what the seeds say first, and only
/// when none of that is strong enough, what the links and the store's counts say.
fn reason_for(
    listing: &Listing<'_>,
    (file, row): (u32, &FileRow),
) -> Result<Option<Reason>, StoreError> {
    let seeds = listing.seeds;
    let matched = seeds.lexical.files.get(&file);
    let mut evidence = ItemEvidence {
        is_named: matched.is_some_and(|matched| matched.is_named),
        is_edited_here: seeds.session.edited_files.contains(&file),
        words: matched
            .map(|matched| matched.words.clone())
            .unwrap_or_default(),
        is_uncommitted: seeds.uncommitted.files.contains(&file),
        is_like_earlier_work: seeds.similar.seeds.contains_key(&file),
        is_read_here: seeds
            .session
            .in_context
            .iter()
            .any(|read| read.path == row.file.path),
        ..ItemEvidence::default()
    };
    if let Some(
        reason @ (Reason::Named | Reason::EditedHere | Reason::Matches(_) | Reason::Uncommitted),
    ) = reason_of(&evidence)
    {
        return Ok(Some(reason));
    }
    evidence.changes_with = changes_with(listing, (file, row))?;
    (evidence.refers_to, evidence.referred_to_by) = referred(listing, file)?;
    evidence.worked_on_with = match listing.support.partners(file, EdgeType::CoAccess).first() {
        Some((other, _)) => name_of(listing, *other)?,
        None => None,
    };
    Ok(reason_of(&evidence))
}

/// The seed file `file` changes with most strongly, by name, with the commits that changed both
/// and those that changed `file`, as the store counts them; none without the counts.
fn changes_with(
    listing: &Listing<'_>,
    (file, row): (u32, &FileRow),
) -> Result<Option<(String, u32, u32)>, StoreError> {
    let Some((other, _)) = listing
        .support
        .partners(file, EdgeType::CoChange)
        .first()
        .copied()
    else {
        return Ok(None);
    };
    let Some((other_id, other_name)) = row_of(listing, other)? else {
        return Ok(None);
    };
    let together = listing
        .queries
        .cochange_pairs_of(&[row.id])?
        .into_iter()
        .find(|pair| pair_of(pair, row.id) == Some(other_id))
        .map(|pair| pair.count);
    let of = listing
        .queries
        .file_commits_of(&[row.id])?
        .first()
        .map(|counts| counts.count);
    Ok(together
        .zip(of)
        .map(|(together, of)| (other_name, together, of)))
}

/// The other file of `pair`, when `file` is one of its two.
fn pair_of(pair: &cortex_store::PairRow, file: FileId) -> Option<FileId> {
    if pair.file_a == file {
        Some(pair.file_b)
    } else if pair.file_b == file {
        Some(pair.file_a)
    } else {
        None
    }
}

/// The seed file a reference ties `file` to most strongly, by name, as the file it refers to or the
/// file that refers to it: the stored edges between the two say which way the reference runs.
fn referred(
    listing: &Listing<'_>,
    file: u32,
) -> Result<(Option<String>, Option<String>), StoreError> {
    let Some((other, _)) = listing
        .support
        .partners(file, EdgeType::Structural)
        .first()
        .copied()
    else {
        return Ok((None, None));
    };
    let Some(name) = name_of(listing, other)? else {
        return Ok((None, None));
    };
    let ids_of = |node: u32| -> Vec<NodeId> {
        std::iter::once(node)
            .chain(listing.graph.symbols_of(node).iter().copied())
            .filter_map(|node| listing.graph.id(node))
            .collect()
    };
    let (ours, theirs) = (ids_of(file), ids_of(other));
    let both: Vec<NodeId> = ours.iter().chain(&theirs).copied().collect();
    let edges = listing.queries.edges_among(&both, EdgeType::Structural)?;
    let is_strong = |weight: f64| weight >= STRUCTURAL_FLOOR;
    let refers = edges.iter().any(|edge| {
        ours.contains(&edge.src) && theirs.contains(&edge.edge.dst) && is_strong(edge.edge.weight)
    });
    if refers {
        return Ok((Some(name), None));
    }
    let is_referred = edges.iter().any(|edge| {
        theirs.contains(&edge.src) && ours.contains(&edge.edge.dst) && is_strong(edge.edge.weight)
    });
    Ok((None, is_referred.then_some(name)))
}

/// The row and file name of file `node`.
fn row_of(listing: &Listing<'_>, node: u32) -> Result<Option<(FileId, String)>, StoreError> {
    let Some(id) = listing.graph.id(node) else {
        return Ok(None);
    };
    Ok(listing
        .queries
        .files_of_nodes(&[id])?
        .into_iter()
        .next()
        .map(|(_, row)| (row.id, row.file.path.file_name().to_owned())))
}

/// The file name of file `node`.
fn name_of(listing: &Listing<'_>, node: u32) -> Result<Option<String>, StoreError> {
    Ok(row_of(listing, node)?.map(|(_, name)| name))
}
