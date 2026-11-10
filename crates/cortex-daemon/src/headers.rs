//! The signatures of definitions as the index read them, kept in memory by the hash the index
//! stores for each declaration header, so that a notice of a changed signature can say what it was
//! before the change. The store keeps only that hash: the text was in the file, and once the file
//! is edited the old text is in no file the index reads. So every file the index parses leaves its
//! definitions' signatures here, and a signature is taken back only by a hash the store held for
//! the definition before the change: the hash vouches that the text is the one the index read.
//!
//! Nothing here is written anywhere: the signatures are code, which never reaches the store or the
//! logs. The memory is bounded; a header not read since the daemon started, or pushed out by newer
//! ones, is not here, and the version committed at HEAD is read instead when it has the same hash.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use cortex_coord::Signature;
use cortex_core::ShortHash;
use cortex_index::Extraction;

/// The most signatures kept: those noted first go first. Each holds a hash of 16 bytes and at most
/// 120 characters of text, so by estimate (not measured) under 300 bytes, and this bounds the
/// memory at under 20 MB.
const MAX_KEPT: usize = 65_536;

/// The longest declaration header read for its signature, in bytes, before its whitespace is
/// collapsed: a definition with no body is all header, and a whole class or table read for a
/// signature of at most 120 characters would be work and memory for nothing.
pub(crate) const MAX_HEADER_BYTES: usize = 1_024;

/// What is kept, behind the lock.
#[derive(Debug)]
struct Kept {
    /// Each signature by the hash of its header.
    by_hash: BTreeMap<ShortHash, Signature>,
    /// The same hashes, in the order they were first noted.
    order: VecDeque<ShortHash>,
}

/// The signatures the index read lately.
#[derive(Debug)]
pub(crate) struct Headers {
    /// What is kept.
    kept: Mutex<Kept>,
}

impl Headers {
    /// Nothing kept yet; usable where a value must be made at compile time, such as a static.
    pub(crate) const fn new() -> Self {
        Self {
            kept: Mutex::new(Kept {
                by_hash: BTreeMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    /// Keeps the signatures of the definitions `extraction` read from `source`, worked out before
    /// the memory is locked.
    pub(crate) fn note(&self, extraction: &Extraction, source: &[u8]) {
        let read = signatures_in(extraction, source);
        if read.is_empty() {
            return;
        }
        let mut kept = self.lock();
        for (sig_hash, signature) in read {
            kept.keep(sig_hash, signature);
        }
    }

    /// The signature read from the header whose hash is `sig_hash`, if it is kept.
    pub(crate) fn said(&self, sig_hash: ShortHash) -> Option<Signature> {
        self.lock().by_hash.get(&sig_hash).cloned()
    }

    /// What is kept, locked. Each change to it is made whole before the lock is let go, so a
    /// poisoned lock is taken as it is.
    fn lock(&self) -> MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Kept {
    /// Keeps `signature` under `sig_hash`, unless it is kept already, pushing out the one noted
    /// first past [`MAX_KEPT`].
    fn keep(&mut self, sig_hash: ShortHash, signature: Signature) {
        if self.by_hash.contains_key(&sig_hash) {
            return;
        }
        self.by_hash.insert(sig_hash, signature);
        self.order.push_back(sig_hash);
        while self.order.len() > MAX_KEPT {
            if let Some(oldest) = self.order.pop_front() {
                self.by_hash.remove(&oldest);
            }
        }
    }
}

/// The signatures the definitions `extraction` read from `source` can be said in, each with the
/// hash of its header, in the order of the definitions, those [`signature_at`] says.
pub(crate) fn signatures_in(extraction: &Extraction, source: &[u8]) -> Vec<(ShortHash, Signature)> {
    (0..extraction.symbols.len())
        .filter_map(|position| {
            let symbol = extraction.symbols.get(position)?;
            Some((symbol.sig_hash, signature_at(extraction, position, source)?))
        })
        .collect()
}

/// The signature of the definition at `position` in what `extraction` read from `source`: none
/// for a header longer than [`MAX_HEADER_BYTES`], not UTF-8, or whose signature cannot be said.
pub(crate) fn signature_at(
    extraction: &Extraction,
    position: usize,
    source: &[u8],
) -> Option<Signature> {
    let symbol = extraction.symbols.get(position)?;
    let header = extraction.header(position, source)?;
    // INVARIANT: budgeted (a header past the cap is not collapsed, nor kept)
    if header.len() > MAX_HEADER_BYTES {
        return None;
    }
    Signature::read(std::str::from_utf8(header).ok()?, symbol.name.as_str())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use cortex_core::{Language, RepoPath};
    use cortex_index::{Extractor, Grammars};

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// `source`, a Python file, read as the index reads it.
    fn read(source: &str) -> TestResult<Extraction> {
        let mut extractor = Extractor::new(Arc::new(Grammars::load()?), Duration::from_secs(60));
        Ok(extractor.extract(
            &RepoPath::new("ops.py")?,
            Language::Python,
            source.as_bytes(),
        )?)
    }

    /// What is said of the header hashed as `add`'s in `source`, as `headers` keeps it.
    fn said_of_add(headers: &Headers, source: &str) -> TestResult<Option<String>> {
        let hash = read(source)?
            .symbols
            .iter()
            .find(|symbol| symbol.name.as_str() == "add")
            .map(|symbol| symbol.sig_hash)
            .ok_or("no add")?;
        Ok(headers.said(hash).map(|said| said.as_str().to_owned()))
    }

    /// Each header read is kept by its hash: `add(a, b)` and then `add(a, b, c=0)` are both told
    /// by their own hashes; a header never read is not.
    #[test]
    fn each_signature_is_told_by_the_hash_of_its_header() -> TestResult {
        let (old, new) = (
            "def add(a, b):\n    pass\n",
            "def add(a, b, c=0):\n    pass\n",
        );
        let headers = Headers::new();
        headers.note(&read(old)?, old.as_bytes());
        assert_eq!(said_of_add(&headers, old)?.as_deref(), Some("(a, b)"));
        assert_eq!(said_of_add(&headers, new)?, None);
        headers.note(&read(new)?, new.as_bytes());
        assert_eq!(said_of_add(&headers, old)?.as_deref(), Some("(a, b)"));
        assert_eq!(said_of_add(&headers, new)?.as_deref(), Some("(a, b, c=0)"));
        Ok(())
    }

    /// A header of 1,024 bytes is read; one of 1,025 is not, though its signature, once the
    /// spaces in it are collapsed, would be the same and short.
    #[test]
    fn a_header_longer_than_the_cap_is_not_read() -> TestResult {
        // `def add(a,` is 10 bytes and `b):` 3: spaces fill the rest of the header.
        let header = |bytes: usize| format!("def add(a,{}b):\n    pass\n", " ".repeat(bytes - 13));
        // Both collapse to `def add(a, b):`, one hash: each is read into a memory of its own.
        let noted = |source: &str| -> TestResult<Option<String>> {
            let headers = Headers::new();
            headers.note(&read(source)?, source.as_bytes());
            said_of_add(&headers, source)
        };
        let fits = noted(&header(MAX_HEADER_BYTES))?;
        assert_eq!(fits.as_deref(), Some("(a, b)"));
        assert_eq!(noted(&header(MAX_HEADER_BYTES + 1))?, None);
        Ok(())
    }

    /// Past the bound, the signature noted first goes first; one noted again stays where it was.
    #[test]
    fn the_signature_noted_first_goes_first() -> TestResult {
        let signature = Signature::read("def f(x):", "f").ok_or("not said")?;
        let headers = Headers::new();
        let mut kept = headers.lock();
        for at in 0..=MAX_KEPT {
            kept.keep(ShortHash::of(format!("{at}").as_bytes()), signature.clone());
        }
        kept.keep(ShortHash::of(b"1"), signature);
        assert_eq!(kept.by_hash.len(), MAX_KEPT);
        assert_eq!(kept.order.len(), MAX_KEPT);
        assert!(!kept.by_hash.contains_key(&ShortHash::of(b"0")));
        assert!(kept.by_hash.contains_key(&ShortHash::of(b"1")));
        Ok(())
    }
}
