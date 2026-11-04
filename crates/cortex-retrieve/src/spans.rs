//! Which lines of a top file to point at: the file's best symbols, each taken out to the function
//! or class around it and cut to a window of lines centred on its strongest part.
//!
//! A file's symbols are ranked by their strength: their score, or, when none scored, how much
//! agents have worked on them. A symbol is taken out to the nearest definition around it, itself
//! included, that is a function, a method, a test, a class, an interface or a type: a constant in a
//! class points at the class. A span longer than the cap is cut to a window that long, centred on
//! the middle of the strongest symbol within it. Two spans that overlap or touch are one, so long as
//! the two together fit the cap; otherwise the weaker is passed over. A file gets at most two spans.
//! A file with symbols but none of any strength has none, and is listed by its path alone; a file
//! with no symbols at all and no more lines than the cap is shown whole.
//!
//! A span the session read since its context was last compacted, while the file still holds what
//! it read, is already in the agent's context, and is dropped.

use std::collections::BTreeMap;

use cortex_core::{Identifier, LineRange, ShortHash, SymbolKind};
use cortex_store::{FileRow, NodeId, Queries, StoreError, SymbolId, SymbolSpan};

use crate::graph::Graph;
use crate::score::ScoredFile;
use crate::session::ReadSpan;

/// Most spans one file gets.
pub const MAX_SPANS_PER_FILE: usize = 2;

/// A stretch of a file to point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Its lines.
    pub lines: LineRange,
    /// The strongest symbol within it, by node; none for a file shown whole.
    pub symbol: Option<u32>,
}

/// One symbol of a file as span selection reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Outlined {
    /// Its node.
    pub node: u32,
    /// Its name.
    pub name: Identifier,
    /// The position in the same list of the definition around it, if any.
    pub parent: Option<usize>,
    /// What it is.
    pub kind: SymbolKind,
    /// Its lines.
    pub lines: LineRange,
    /// How strongly it is ranked: its score, or its interest when no symbol of its file scored.
    pub strength: f64,
}

/// The spans to point at in a file of `line_count` lines whose symbols are `outline`, in the order
/// they start, each at most `cap` lines, the strongest first.
#[must_use]
pub fn spans_of(outline: &[Outlined], line_count: u32, cap: u32) -> Vec<Span> {
    if outline.is_empty() {
        return LineRange::new(1, line_count)
            .ok()
            .filter(|_| line_count <= cap)
            .map(|lines| Span {
                lines,
                symbol: None,
            })
            .into_iter()
            .collect();
    }
    let mut ranked: Vec<usize> = (0..outline.len())
        .filter(|at| outline.get(*at).is_some_and(|symbol| symbol.strength > 0.0))
        .collect();
    ranked.sort_by(|left, right| strength_order(outline, *left, *right));
    let mut spans: Vec<Span> = Vec::new();
    for at in ranked {
        if spans.len() >= MAX_SPANS_PER_FILE {
            break;
        }
        let Some(span) = span_around(outline, at, cap) else {
            continue;
        };
        match spans
            .iter()
            .position(|held| is_beside(held.lines, span.lines))
        {
            Some(held) => {
                if let Some(slot) = spans.get_mut(held)
                    && let Some(joined) = joined(slot.lines, span.lines, cap)
                {
                    slot.lines = joined;
                }
            }
            None => spans.push(span),
        }
    }
    spans
}

/// The outline of a file, stored as `row`, for span selection: each of its symbols as `graph`
/// numbers it, in the order they appear, with its strength: its score among `scored`'s symbols, or,
/// when none of the file's symbols scored, its interest. A symbol the graph does not hold is left
/// out, and so is a link to an enclosing symbol that is.
///
/// # Errors
///
/// The store's errors.
pub fn outline_of(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    (scored, row): (&ScoredFile, &FileRow),
) -> Result<Vec<Outlined>, StoreError> {
    let spans = queries.symbol_spans_in_file(row.id)?;
    let held: Vec<(u32, &SymbolSpan, LineRange)> = spans
        .iter()
        .filter_map(|span| {
            let node = graph.index_of(span.node)?;
            let lines = LineRange::new(span.start_line, span.end_line).ok()?;
            Some((node, span, lines))
        })
        .collect();
    let positions: BTreeMap<SymbolId, usize> = held
        .iter()
        .enumerate()
        .map(|(at, (_, span, _))| (span.symbol, at))
        .collect();
    let symbol_scores: BTreeMap<u32, f64> = scored.symbols.iter().copied().collect();
    let strength = |node: u32| {
        if symbol_scores.is_empty() {
            graph
                .standing(node)
                .map_or(0.0, |standing| standing.interest)
        } else {
            symbol_scores.get(&node).copied().unwrap_or(0.0)
        }
    };
    Ok(held
        .iter()
        .map(|(node, span, lines)| Outlined {
            node: *node,
            name: span.name.clone(),
            parent: span
                .parent
                .and_then(|parent| positions.get(&parent).copied()),
            kind: span.kind,
            lines: *lines,
            strength: strength(*node),
        })
        .collect())
}

/// Whether `span` of the file at `path`, which the index holds as `content`, is among the spans
/// `in_context` holds: read whole or over lines covering it, while the file still holds what was
/// read.
#[must_use]
pub fn is_in_context(
    span: &Span,
    (path, content): (&str, ShortHash),
    in_context: &[ReadSpan],
) -> bool {
    in_context.iter().any(|read| {
        read.path.as_str() == path
            && read.content == Some(content)
            && read.range.is_none_or(|range| {
                range.start_line() <= span.lines.start_line()
                    && span.lines.end_line() <= range.end_line()
            })
    })
}

/// The order of two symbols of `outline`: the stronger first, then the one that starts first.
fn strength_order(outline: &[Outlined], left: usize, right: usize) -> std::cmp::Ordering {
    let strength = |at: usize| outline.get(at).map_or(0.0, |symbol| symbol.strength);
    let start = |at: usize| {
        outline
            .get(at)
            .map_or(0, |symbol| symbol.lines.start_line())
    };
    strength(right)
        .total_cmp(&strength(left))
        .then(start(left).cmp(&start(right)))
        .then(left.cmp(&right))
}

/// The span for the symbol at `at` in `outline`: the body around it, cut to `cap` lines centred
/// on the strongest symbol within that body.
fn span_around(outline: &[Outlined], at: usize, cap: u32) -> Option<Span> {
    let body = body_of(outline, at)?;
    let strongest = (0..outline.len())
        .filter(|inner| {
            outline
                .get(*inner)
                .is_some_and(|symbol| contains(body.lines, symbol.lines))
        })
        .min_by(|left, right| strength_order(outline, *left, *right))
        .and_then(|inner| outline.get(inner))?;
    let centre = strongest.lines.start_line()
        + (strongest.lines.end_line() - strongest.lines.start_line()) / 2;
    Some(Span {
        lines: cut(body.lines, cap, centre)?,
        symbol: Some(strongest.node),
    })
}

/// The nearest definition around the symbol at `at`, itself included, that is a body; the symbol
/// itself when none is.
fn body_of(outline: &[Outlined], at: usize) -> Option<&Outlined> {
    let symbol = outline.get(at)?;
    let mut next = Some(at);
    while let Some(here) = next {
        let around = outline.get(here)?;
        if is_body(around.kind) {
            return Some(around);
        }
        // A parent comes before its children in the list, so the walk only goes up.
        next = around.parent.filter(|parent| *parent < here);
    }
    Some(symbol)
}

/// Whether a definition of `kind` is a body a span may be taken out to.
fn is_body(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function
            | SymbolKind::Method
            | SymbolKind::Test
            | SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::Type
    )
}

/// `lines` cut to at most `cap` lines, centred on line `centre` as far as `lines` allows.
fn cut(lines: LineRange, cap: u32, centre: u32) -> Option<LineRange> {
    let length = lines.end_line() - lines.start_line() + 1;
    if length <= cap {
        return Some(lines);
    }
    let cap = cap.max(1);
    let lowest = lines.start_line();
    let highest = lines.end_line() - (cap - 1);
    let start = centre.saturating_sub((cap - 1) / 2).clamp(lowest, highest);
    LineRange::new(start, start + cap - 1).ok()
}

/// Whether `outer` holds every line of `inner`.
fn contains(outer: LineRange, inner: LineRange) -> bool {
    outer.start_line() <= inner.start_line() && inner.end_line() <= outer.end_line()
}

/// Whether two stretches share a line or meet end to start.
fn is_beside(left: LineRange, right: LineRange) -> bool {
    left.start_line() <= right.end_line().saturating_add(1)
        && right.start_line() <= left.end_line().saturating_add(1)
}

/// The stretch covering both `left` and `right`, when it is no longer than `cap` lines.
fn joined(left: LineRange, right: LineRange, cap: u32) -> Option<LineRange> {
    let start = left.start_line().min(right.start_line());
    let end = left.end_line().max(right.end_line());
    (end - start < cap)
        .then(|| LineRange::new(start, end).ok())
        .flatten()
}
