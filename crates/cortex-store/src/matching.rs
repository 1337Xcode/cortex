//! Which stored symbol a file's new definition is: the one with the same kind and qualified name,
//! several such matched in the order they were stored. The writer keeps a matched symbol's id, its
//! node and the references tied to it; anyone else may ask, without writing, which stored symbols a
//! new reading of the file would change, and how.

use std::collections::{BTreeMap, VecDeque};

use cortex_core::{Identifier, ShortHash, SymbolKind};
use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::ids::{FileId, SymbolId};
use crate::queries::Queries;
use crate::records::{ChangeKind, SymbolRecord};
use crate::{schema, values};

/// A stored symbol, as far as matching needs it.
pub(crate) struct StoredSymbol {
    /// Its row.
    pub(crate) id: SymbolId,
    /// What it is.
    pub(crate) kind: SymbolKind,
    /// Its name.
    pub(crate) name: Identifier,
    /// Its qualified name.
    pub(crate) qual_name: Identifier,
    /// The hash of its declaration header.
    pub(crate) sig_hash: ShortHash,
    /// The hash of the whole definition.
    pub(crate) body_hash: ShortHash,
}

/// How a file's new definitions pair with its stored symbols.
pub(crate) struct Pairing {
    /// For each new definition, in its order, the stored symbol it is, if any.
    pub(crate) matched: Vec<Option<StoredSymbol>>,
    /// The stored symbols no new definition is, by id.
    pub(crate) gone: Vec<StoredSymbol>,
}

/// Pairs each of `symbols`, a file's new definitions, with the first of `stored` of the same kind
/// and qualified name not taken by an earlier one.
pub(crate) fn pair(stored: Vec<StoredSymbol>, symbols: &[SymbolRecord]) -> Pairing {
    let mut waiting: BTreeMap<(SymbolKind, Identifier), VecDeque<StoredSymbol>> = BTreeMap::new();
    for symbol in stored {
        waiting
            .entry((symbol.kind, symbol.qual_name.clone()))
            .or_default()
            .push_back(symbol);
    }
    let matched = symbols
        .iter()
        .map(|symbol| {
            waiting
                .get_mut(&(symbol.kind, symbol.qual_name.clone()))
                .and_then(VecDeque::pop_front)
        })
        .collect();
    let mut gone: Vec<StoredSymbol> = waiting.into_values().flatten().collect();
    gone.sort_by_key(|old| old.id);
    Pairing { matched, gone }
}

/// What changed between a stored symbol and the definition that matched it.
pub(crate) fn change_between(old: &StoredSymbol, new: &SymbolRecord) -> Option<ChangeKind> {
    if old.sig_hash != new.sig_hash {
        Some(ChangeKind::Signature)
    } else if old.body_hash != new.body_hash {
        Some(ChangeKind::Body)
    } else {
        None
    }
}

/// The stored symbols of `file`, in the order matching takes them.
pub(crate) fn stored_symbols(
    connection: &Connection,
    file: FileId,
) -> Result<Vec<StoredSymbol>, StoreError> {
    type Raw = (i64, String, String, String, [u8; 16], [u8; 16]);
    let raw: Vec<Raw> = connection
        .prepare_cached(schema::SYMBOLS_TO_MATCH)
        .and_then(|mut statement| {
            statement
                .query_map([file.get()], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?
                .collect()
        })
        .map_err(sqlite("reading symbols"))?;
    raw.into_iter()
        .map(|(id, kind, name, qual_name, sig_hash, body_hash)| {
            Ok(StoredSymbol {
                id: SymbolId(id),
                kind: values::kind(&kind, SymbolKind::from_name, "symbols.kind", id)?,
                name: values::identifier(&name, "symbols.name", id)?,
                qual_name: values::identifier(&qual_name, "symbols.qual_name", id)?,
                sig_hash: ShortHash::from_bytes(sig_hash),
                body_hash: ShortHash::from_bytes(body_hash),
            })
        })
        .collect()
}

impl Queries<'_> {
    /// The stored symbols of `file` whose signature `symbols`, a new reading of the file, would
    /// change if it were written, matched as the writer matches them, in the order of `symbols`.
    /// Nothing is written.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn signature_changes(
        self,
        file: FileId,
        symbols: &[SymbolRecord],
    ) -> Result<Vec<SymbolId>, StoreError> {
        let pairing = pair(stored_symbols(self.connection(), file)?, symbols);
        Ok(pairing
            .matched
            .iter()
            .zip(symbols)
            .filter_map(|(old, new)| {
                let old = old.as_ref()?;
                (change_between(old, new) == Some(ChangeKind::Signature)).then_some(old.id)
            })
            .collect())
    }
}
