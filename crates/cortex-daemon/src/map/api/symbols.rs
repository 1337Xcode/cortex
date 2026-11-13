//! A file's symbols for the map's file zoom, in the order they appear, each with how many of the
//! latest work orders that listed the file selected lines of it; and the spans those work orders
//! selected, so that the page can highlight them.
//!
//! The work orders read are the ones the panel's "told" list reads: the latest
//! [`told::MAX_TOLD`] that listed the file. A span counts for a symbol when the two share a line.
//! Line numbers are the index's, as of its last read of the file.

use std::collections::{BTreeMap, BTreeSet};

use cortex_store::{InjectionRow, Queries, StoreError};
use serde::Serialize;

use super::listing::ListedFile;
use super::told;

/// The most symbols the answer lists: the first in the file, enough for any file a person reads
/// whole, few enough that a generated file of thousands keeps the answer small.
pub(super) const MAX_SYMBOLS: usize = 500;

/// One symbol the file defines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Symbol {
    /// Its name.
    name: String,
    /// What it is: `function`, `method`, `class` and so on.
    kind: &'static str,
    /// The 1-based line where it starts.
    start_line: u32,
    /// The 1-based line where it ends.
    end_line: u32,
    /// Whether another symbol of the file holds it.
    is_nested: bool,
    /// How many of the work orders read selected a span sharing a line with it.
    selected: usize,
}

/// A span of the file that work orders selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Selected {
    /// The 1-based line where it starts.
    start_line: u32,
    /// The 1-based line where it ends.
    end_line: u32,
    /// How many of the work orders read selected it.
    orders: usize,
}

/// A file's symbols and selected spans.
#[derive(Debug, Default)]
pub(super) struct FileSymbols {
    /// The first [`MAX_SYMBOLS`] symbols, in the order they appear.
    pub(super) symbols: Vec<Symbol>,
    /// How many symbols the file defines.
    pub(super) total: usize,
    /// The spans the work orders read selected, by their lines.
    pub(super) selected: Vec<Selected>,
}

/// The symbols of `file` and the spans the latest work orders that listed it selected.
///
/// # Errors
///
/// SQLite errors and stored values that cannot be read back.
pub(super) fn of_file(queries: Queries<'_>, file: &ListedFile) -> Result<FileSymbols, StoreError> {
    let orders = queries.work_orders_naming(file.node, told::MAX_TOLD)?;
    let spans_by_order: Vec<BTreeSet<(u32, u32)>> =
        orders.iter().map(|order| spans_of(order, file)).collect();
    let mut selected: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for span in spans_by_order.iter().flatten() {
        *selected.entry(*span).or_insert(0) += 1;
    }
    let spans = queries.symbol_spans_in_file(file.file)?;
    let total = spans.len();
    let symbols = spans
        .iter()
        .take(MAX_SYMBOLS)
        .map(|span| Symbol {
            name: span.name.as_str().to_owned(),
            kind: span.kind.name(),
            start_line: span.start_line,
            end_line: span.end_line,
            is_nested: span.parent.is_some(),
            selected: spans_by_order
                .iter()
                .filter(|order| {
                    order
                        .iter()
                        .any(|&(start, end)| start <= span.end_line && span.start_line <= end)
                })
                .count(),
        })
        .collect();
    Ok(FileSymbols {
        symbols,
        total,
        selected: selected
            .into_iter()
            .map(|((start_line, end_line), orders)| Selected {
                start_line,
                end_line,
                orders,
            })
            .collect(),
    })
}

/// The spans of `file` that `order` selected, each once.
fn spans_of(order: &InjectionRow, file: &ListedFile) -> BTreeSet<(u32, u32)> {
    order
        .record
        .files
        .iter()
        .filter(|named| named.node == file.node)
        .flat_map(|named| named.spans.iter().copied())
        .collect()
}
