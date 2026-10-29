//! The names a file uses as the index records them: each with its kind, its line and the
//! definition it sits in.

use cortex_core::Identifier;
use cortex_store::{RefRecord, SymbolRecord};

use crate::lines::line_number;
use crate::name_text::name_text;
use crate::tags::{Role, Tag};

/// The references among `tags`, which are in the order their names appear, as records, each tied
/// to the smallest of `symbols` around its name. `symbols` are in the order of their starts, each
/// after the one around it.
pub(crate) fn build_references(
    tags: &[Tag<'_>],
    source: &[u8],
    symbols: &[SymbolRecord],
) -> Vec<RefRecord> {
    let mut refs = Vec::new();
    // The positions of the symbols that start before the current name and may hold it, outer
    // before inner, and of the next symbol to consider.
    let mut around: Vec<usize> = Vec::new();
    let mut next = 0;
    for tag in tags {
        let Role::Reference(kind) = tag.role else {
            continue;
        };
        let (start, end) = (tag.name.start_byte() as u64, tag.name.end_byte() as u64);
        while let Some(symbol) = symbols.get(next)
            && symbol.start_byte <= start
        {
            around.push(next);
            next += 1;
        }
        leave(&mut around, symbols, start, end);
        let Some(dst_name) =
            name_text(tag.name, source).and_then(|text| Identifier::new(&text).ok())
        else {
            continue;
        };
        let Some(line) = line_number(tag.name.start_position().row) else {
            continue;
        };
        refs.push(RefRecord {
            symbol: around.last().copied(),
            dst_name,
            kind,
            line,
        });
    }
    refs
}

/// Drops from the end of `around` the symbols that do not hold the bytes from `start` to `end`,
/// leaving the smallest symbol that does on top. Every symbol in `around` starts before the name,
/// and names come in the order of their starts, so a symbol dropped here holds no later name.
fn leave(around: &mut Vec<usize>, symbols: &[SymbolRecord], start: u64, end: u64) {
    while let Some(&innermost) = around.last() {
        let holds = symbols
            .get(innermost)
            .is_some_and(|symbol| symbol.start_byte <= start && end <= symbol.end_byte);
        if holds {
            break;
        }
        around.pop();
    }
}
