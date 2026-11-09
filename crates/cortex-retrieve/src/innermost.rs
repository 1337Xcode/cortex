//! The innermost definitions holding a line of a file: the one rule `path:line` is read by, in a
//! prompt and in a `why` target. Of the definitions whose lines hold the line, those spanning the
//! fewest lines; of those, any that is the parent of another is not innermost (a class and its one
//! method written on one line). Two left over do not nest: the line holds both.

use cortex_store::{FileId, Queries, StoreError, SymbolSpan};

/// The innermost definitions of `file` whose lines hold `line`, by node; none when no definition
/// holds it, several when definitions that do not nest share it.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn innermost_at(
    queries: Queries<'_>,
    file: FileId,
    line: u32,
) -> Result<Vec<SymbolSpan>, StoreError> {
    let holding: Vec<SymbolSpan> = queries
        .symbol_spans_in_file(file)?
        .into_iter()
        .filter(|span| span.start_line <= line && line <= span.end_line)
        .collect();
    let Some(fewest) = holding
        .iter()
        .map(|span| span.end_line - span.start_line)
        .min()
    else {
        return Ok(Vec::new());
    };
    let shortest: Vec<&SymbolSpan> = holding
        .iter()
        .filter(|span| span.end_line - span.start_line == fewest)
        .collect();
    let mut innermost: Vec<SymbolSpan> = shortest
        .iter()
        .filter(|span| {
            !shortest
                .iter()
                .any(|other| other.parent == Some(span.symbol))
        })
        .map(|span| (*span).clone())
        .collect();
    innermost.sort_by_key(|span| span.node);
    Ok(innermost)
}
