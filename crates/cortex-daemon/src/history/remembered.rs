//! What the worker remembers not to try again while the daemon runs, each at most
//! [`MAX_REMEMBERED`] entries, the oldest forgotten first: spans whose blame failed or that the
//! store refused, spans whose file at HEAD is not the file on disk, and commits that cannot be
//! carded. A span that fails once would fail on every prompt that shows it.

use std::collections::{BTreeMap, VecDeque};

use cortex_core::{ContentHash, RepoPath};

/// The most entries each memory keeps: a few hundred bytes each, and more failures than this in
/// one daemon's life are a repository where they keep coming, whose oldest may be tried once more.
pub(crate) const MAX_REMEMBERED: usize = 1_024;

/// A span of a file: its path, its content's hash, its first and last line.
pub(crate) type SpanKey = (RepoPath, ContentHash, u32, u32);

/// Keys remembered with a value each, oldest first, at most [`MAX_REMEMBERED`].
#[derive(Debug)]
pub(crate) struct Remembered<K: Ord + Clone, V> {
    /// The value of each key.
    values: BTreeMap<K, V>,
    /// The keys, oldest first, to forget the oldest.
    order: VecDeque<K>,
}

impl<K: Ord + Clone, V> Default for Remembered<K, V> {
    /// Nothing remembered.
    fn default() -> Self {
        Self {
            values: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }
}

impl<K: Ord + Clone, V> Remembered<K, V> {
    /// Remembers `value` for `key`, replacing an earlier one, and forgets the oldest key when full.
    pub(crate) fn insert(&mut self, key: K, value: V) {
        if self.values.insert(key.clone(), value).is_some() {
            return;
        }
        self.order.push_back(key);
        while self.order.len() > MAX_REMEMBERED {
            if let Some(oldest) = self.order.pop_front() {
                self.values.remove(&oldest);
            }
        }
    }

    /// The value remembered for `key`.
    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        self.values.get(key)
    }

    /// How many keys are remembered.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keys are remembered up to 1,024: the 1,025th forgets the first, and a key remembered again
    /// keeps its place and takes the new value.
    #[test]
    fn at_most_1024_are_remembered() {
        let mut remembered = Remembered::default();
        for number in 0..1_024_u32 {
            remembered.insert(number, "first");
        }
        remembered.insert(5, "again");
        assert_eq!(remembered.get(&5), Some(&"again"));
        assert_eq!(remembered.get(&0), Some(&"first"));
        remembered.insert(1_024, "first");
        assert_eq!(remembered.get(&0), None);
        assert_eq!(remembered.get(&1), Some(&"first"));
        assert_eq!(remembered.get(&1_024), Some(&"first"));
        assert_eq!(remembered.len(), 1_024);
    }
}
