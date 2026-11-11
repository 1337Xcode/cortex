//! The text cache: the two lines of a commit's message worth showing, its subject and its reason
//! line, held in memory for at most [`MAX_TEXTS`] commits and never written anywhere. When it is
//! full, the commit read or put longest ago gives way. Uses are counted, not timed, so the same
//! reads and puts always leave the same commits.

use std::collections::BTreeMap;
use std::fmt;

/// The most commits whose text is held: a work order shows a reason or two for each of a few
/// spans, so 2,000 commits cover the spans of many tasks, at a few hundred bytes each (a subject
/// is at most 72 characters and a reason line at most 100).
pub(crate) const MAX_TEXTS: usize = 2_000;

/// The two lines of a commit's message worth showing, as its card trimmed them.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CardText {
    /// The subject, at most 72 characters.
    pub(crate) subject: String,
    /// The first line of the body that gives a reason, at most 100 characters, if one does.
    pub(crate) reason: Option<String>,
}

impl fmt::Debug for CardText {
    /// Says nothing of the text, which a person wrote: a debug line that reached a log would
    /// otherwise copy it there.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("CardText").finish_non_exhaustive()
    }
}

/// The texts held, by commit id, each with the count of the use that last touched it.
#[derive(Debug)]
pub(crate) struct Texts {
    /// The text of each commit held, by id, with the use that last touched it.
    by_commit: BTreeMap<String, (u64, CardText)>,
    /// The commit each use last touched, by use: the first is the one to give way.
    by_use: BTreeMap<u64, String>,
    /// The count of the last use.
    last_use: u64,
    /// The most commits held.
    capacity: usize,
}

impl Default for Texts {
    /// An empty cache of [`MAX_TEXTS`] commits.
    fn default() -> Self {
        Self::with_capacity(MAX_TEXTS)
    }
}

impl Texts {
    /// An empty cache of at most `capacity` commits.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            by_commit: BTreeMap::new(),
            by_use: BTreeMap::new(),
            last_use: 0,
            capacity,
        }
    }

    /// The text of the commit `oid`, which counts as a use of it; none when it is not held.
    pub(crate) fn get(&mut self, oid: &str) -> Option<CardText> {
        let use_count = self.next_use();
        let (used, text) = self.by_commit.get_mut(oid)?;
        self.by_use.remove(used);
        *used = use_count;
        self.by_use.insert(use_count, oid.to_owned());
        Some(text.clone())
    }

    /// Holds `text` as the commit `oid`'s, replacing what was held for it; when the cache is
    /// full, the commit used longest ago gives way.
    pub(crate) fn put(&mut self, oid: &str, text: CardText) {
        let use_count = self.next_use();
        if let Some((used, _)) = self.by_commit.remove(oid) {
            self.by_use.remove(&used);
        }
        while self.by_commit.len() >= self.capacity {
            let Some((_, oldest)) = self.by_use.pop_first() else {
                break;
            };
            self.by_commit.remove(&oldest);
        }
        if self.capacity > 0 {
            self.by_commit.insert(oid.to_owned(), (use_count, text));
            self.by_use.insert(use_count, oid.to_owned());
        }
    }

    /// How many commits' texts are held.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.by_commit.len()
    }

    /// The count of a new use.
    fn next_use(&mut self) -> u64 {
        self.last_use = self.last_use.saturating_add(1);
        self.last_use
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id of the commit numbered `number`.
    fn oid(number: u32) -> String {
        format!("{number:040x}")
    }

    /// A text whose subject names `number`.
    fn text(number: u32) -> CardText {
        CardText {
            subject: format!("change {number}"),
            reason: None,
        }
    }

    /// Worked by hand, in a cache of 3: commits 1, 2 and 3 are put, then 1 is read, so 2 is the
    /// one used longest ago; putting 4 drops 2. Putting 3 again uses it, so 1 is now oldest, and
    /// putting 5 drops 1. Left: 3, 4 and 5.
    #[test]
    fn the_commit_used_longest_ago_gives_way() {
        let mut texts = Texts::with_capacity(3);
        for number in [1, 2, 3] {
            texts.put(&oid(number), text(number));
        }
        assert_eq!(texts.get(&oid(1)), Some(text(1)));
        texts.put(&oid(4), text(4));
        assert_eq!(texts.get(&oid(2)), None);
        texts.put(&oid(3), text(3));
        texts.put(&oid(5), text(5));
        assert_eq!(texts.len(), 3);
        assert_eq!(texts.get(&oid(1)), None);
        for number in [3, 4, 5] {
            assert_eq!(texts.get(&oid(number)), Some(text(number)), "{number}");
        }
    }

    /// The cache never holds more than 2,000 commits: 2,500 put leave the last 2,000, the first
    /// 500 gone.
    #[test]
    fn at_most_2000_commits_are_held() {
        let mut texts = Texts::default();
        for number in 0..2_500 {
            texts.put(&oid(number), text(number));
        }
        assert_eq!(texts.len(), 2_000);
        assert_eq!(texts.get(&oid(499)), None);
        assert_eq!(texts.get(&oid(500)), Some(text(500)));
        assert_eq!(texts.get(&oid(2_499)), Some(text(2_499)));
    }

    /// A text's debug form holds none of its words.
    #[test]
    fn a_text_is_not_shown_in_debug_output() {
        let shown = format!(
            "{:?}",
            CardText {
                subject: "Charge in integer cents".to_owned(),
                reason: Some("stop rounding drift".to_owned()),
            }
        );
        assert_eq!(shown, "CardText { .. }");
    }
}
