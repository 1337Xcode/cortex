//! The symbols whose signature the writer's indexes changed lately, kept a short while for an
//! edit's warnings. An edit's hook usually comes before the watcher's index of the file, and the
//! store then still shows the old signature; when the index came first, the store already shows the
//! new one and only this memory still knows that it changed, and what the header's hash was before
//! the change, which vouches for the signature a notice says it changed from. Beside them are the
//! signatures themselves as the indexes read them, for a notice to say what a changed one was.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use cortex_core::ShortHash;
use cortex_store::SymbolId;

use crate::headers::Headers;

/// How long a signature change is kept, in milliseconds: long enough for an edit's hook that came
/// after the watcher's index, short enough that an old change is not taken for the edit's.
const KEPT_FOR_MS: i64 = 5 * 60 * 1_000;

/// The most signature changes kept: a large index that changes many signatures keeps its latest.
const MAX_KEPT: usize = 4_096;

/// One signature change an index found.
#[derive(Debug, Clone, Copy)]
struct Change {
    /// The symbol.
    symbol: SymbolId,
    /// When the change was found, in UTC milliseconds.
    found_ms: i64,
    /// The hash of the symbol's header as stored before the change.
    sig_hash_before: ShortHash,
}

/// The signature changes of the latest indexes, oldest first, each with when it was found.
#[derive(Debug)]
pub(crate) struct Signatures {
    /// The changes, oldest first.
    changes: Mutex<VecDeque<Change>>,
    /// The signatures the indexes read, kept while `coordination.contracts` is on.
    pub(crate) headers: Headers,
}

impl Default for Signatures {
    /// No change kept yet.
    fn default() -> Self {
        Self::new()
    }
}

impl Signatures {
    /// No change kept yet; usable where a value must be made at compile time, such as a static.
    pub(crate) const fn new() -> Self {
        Self {
            changes: Mutex::new(VecDeque::new()),
            headers: Headers::new(),
        }
    }

    /// Keeps `changed`, the symbols whose signature an index changed, each with the hash its
    /// header had before, as found at `at_ms`, dropping what is older than [`KEPT_FOR_MS`] by then
    /// and the oldest past [`MAX_KEPT`].
    pub(crate) fn note(
        &self,
        changed: impl IntoIterator<Item = (SymbolId, ShortHash)>,
        at_ms: i64,
    ) {
        let mut changes = self.lock();
        changes.extend(changed.into_iter().map(|(symbol, sig_hash_before)| Change {
            symbol,
            found_ms: at_ms,
            sig_hash_before,
        }));
        while changes
            .front()
            .is_some_and(|change| at_ms - change.found_ms > KEPT_FOR_MS)
        {
            changes.pop_front();
        }
        while changes.len() > MAX_KEPT {
            changes.pop_front();
        }
    }

    /// Those of `symbols` whose signature an index changed at or after `since_ms` and no longer
    /// than [`KEPT_FOR_MS`] before `now_ms`, each once, in the order of `symbols`.
    pub(crate) fn changed_among(&self, symbols: &[SymbolId], window: (i64, i64)) -> Vec<SymbolId> {
        symbols
            .iter()
            .copied()
            .filter(|symbol| self.before_of(*symbol, window).is_some())
            .collect()
    }

    /// The hash `symbol`'s header had before the latest change an index found of it at or after
    /// `since_ms` and no longer than [`KEPT_FOR_MS`] before `now_ms`; none without such a change.
    pub(crate) fn before_of(
        &self,
        symbol: SymbolId,
        (since_ms, now_ms): (i64, i64),
    ) -> Option<ShortHash> {
        let changes = self.lock();
        changes
            .iter()
            .rev()
            .find(|change| {
                change.symbol == symbol
                    && change.found_ms >= since_ms
                    && now_ms - change.found_ms <= KEPT_FOR_MS
            })
            .map(|change| change.sig_hash_before)
    }

    /// The changes, locked. Each change is one push or pop, so a panic while they were held leaves
    /// them whole, and a poisoned lock is taken as it is.
    fn lock(&self) -> MutexGuard<'_, VecDeque<Change>> {
        self.changes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::{Identifier, RepoPath, ShortHash, SymbolKind};
    use cortex_store::{FileIndex, FileRecord, SymbolRecord};

    use super::*;
    use crate::test_store::{TestResult, writer};

    /// Three symbols' ids, from a store of one file defining `a`, `b` and `c`.
    fn three_symbols() -> TestResult<(tempfile::TempDir, [SymbolId; 3])> {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let definition = |name: &str| -> TestResult<SymbolRecord> {
            Ok(SymbolRecord {
                parent: None,
                kind: SymbolKind::Function,
                name: Identifier::new(name)?,
                qual_name: Identifier::new(name)?,
                start_byte: 0,
                end_byte: 1,
                start_line: 1,
                end_line: 1,
                sig_hash: ShortHash::of(name.as_bytes()),
                body_hash: ShortHash::of(name.as_bytes()),
            })
        };
        let mut transaction = writer.transaction()?;
        let change = transaction.replace_file(&FileIndex {
            file: FileRecord {
                path: RepoPath::new("a.py")?,
                language: None,
                size_bytes: 1,
                line_count: Some(1),
                content_hash: None,
                mtime_ns: 1,
                flags: cortex_core::FileFlags::empty(),
                indexed_at_ms: 1,
            },
            symbols: vec![definition("a")?, definition("b")?, definition("c")?],
            refs: Vec::new(),
            passes_on: Vec::new(),
        })?;
        transaction.commit()?;
        let [a, b, c] = change.symbols[..] else {
            return Err("three symbols".into());
        };
        Ok((root, [a, b, c]))
    }

    /// Changes are told for the symbols asked about, found at or after the time asked from and no
    /// more than five minutes (300,000 ms) before now, each once, in the order asked, each with
    /// the hash its header had before its latest change.
    #[test]
    fn changes_are_told_within_their_time() -> TestResult {
        let (_root, [a, b, c]) = three_symbols()?;
        let signatures = Signatures::default();
        let (first, second) = (ShortHash::of(b"first"), ShortHash::of(b"second"));
        signatures.note([(a, first)], 1_000);
        signatures.note([(b, first), (a, second)], 2_000);
        assert_eq!(signatures.changed_among(&[c, b, a], (0, 2_000)), [b, a]);
        assert_eq!(signatures.changed_among(&[a, b], (1_500, 2_000)), [a, b]);
        assert_eq!(signatures.changed_among(&[a, b], (2_001, 2_001)), []);
        // At 302,000 the change of 2,000 is exactly 300,000 old: kept; at 302,001 gone.
        assert_eq!(signatures.changed_among(&[b], (0, 302_000)), [b]);
        assert_eq!(signatures.changed_among(&[b], (0, 302_001)), []);
        assert_eq!(signatures.before_of(a, (0, 2_000)), Some(second));
        assert_eq!(signatures.before_of(a, (2_001, 2_001)), None);
        assert_eq!(signatures.before_of(c, (0, 2_000)), None);
        Ok(())
    }

    /// Noting drops what is older than five minutes by then, and keeps at most 4,096 changes, the
    /// latest.
    #[test]
    fn old_and_surplus_changes_are_dropped() -> TestResult {
        let (_root, [a, b, _]) = three_symbols()?;
        let signatures = Signatures::default();
        let hash = ShortHash::of(b"x");
        signatures.note([(a, hash)], 0);
        signatures.note([(b, hash)], 300_001);
        assert_eq!(signatures.lock().len(), 1);
        signatures.note(std::iter::repeat_n((a, hash), MAX_KEPT + 3), 300_002);
        assert_eq!(signatures.lock().len(), MAX_KEPT);
        assert_eq!(signatures.changed_among(&[b], (0, 300_002)), []);
        Ok(())
    }
}
