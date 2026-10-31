//! The order huge mode parses files in, a few at a time: first the files an agent reads or edits,
//! then the other waiting files of their folders, then the rest, the most recently changed first.
//! A file is handed out once. How fast the crawl goes is the caller's
//! (`index.crawl_files_per_second`); this only decides the order.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use cortex_core::RepoPath;
use cortex_store::{Queries, StoreError};

use crate::pipeline::waits_to_be_parsed;

/// The files a huge index left unparsed, in the order a crawler parses them.
#[derive(Debug, Default)]
pub struct Crawl {
    /// Files asked for, in the order asked.
    requested: VecDeque<RepoPath>,
    /// Waiting files in the folders of those asked for.
    neighbours: VecDeque<RepoPath>,
    /// Every other waiting file, the most recently changed last, so it is taken first.
    rest: Vec<RepoPath>,
    /// The files still waiting.
    waiting: BTreeSet<RepoPath>,
    /// The waiting files of each folder, in path order.
    by_folder: BTreeMap<RepoPath, Vec<RepoPath>>,
    /// The files asked for so far, each queued once.
    asked: BTreeSet<RepoPath>,
    /// The folders whose waiting files are queued as neighbours, each once.
    asked_folders: BTreeSet<RepoPath>,
}

impl Crawl {
    /// A crawl of the stored files `queries` reads that wait to be parsed: those huge mode took
    /// inventory of, in a language Cortex parses and parsed for their kind.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub fn of_store(queries: Queries<'_>) -> Result<Self, StoreError> {
        let waiting = queries
            .file_rows()?
            .into_iter()
            .filter(|row| waits_to_be_parsed(&row.file))
            .map(|row| (row.file.path, row.file.mtime_ns))
            .collect();
        Ok(Self::new(waiting))
    }

    /// A crawl of `waiting`, each file with its modification time in nanoseconds.
    #[must_use]
    pub fn new(mut waiting: Vec<(RepoPath, i64)>) -> Self {
        // The oldest first and, at one time, later paths first, so that taking from the end
        // takes the newest file, and the first path among equally new ones.
        waiting.sort_by(|(left, left_ns), (right, right_ns)| {
            left_ns.cmp(right_ns).then_with(|| right.cmp(left))
        });
        let mut crawl = Self::default();
        for (path, _) in waiting {
            if let Some(folder) = path.parent() {
                crawl
                    .by_folder
                    .entry(folder)
                    .or_default()
                    .push(path.clone());
            }
            crawl.waiting.insert(path.clone());
            crawl.rest.push(path);
        }
        for paths in crawl.by_folder.values_mut() {
            paths.sort();
        }
        crawl
    }

    /// Puts the waiting files among `paths` first, in order, and the other waiting files of their
    /// folders after them, in path order. A file or a folder asked for again is not queued again,
    /// so the queues never hold more than twice the waiting files.
    pub fn request(&mut self, paths: &[RepoPath]) {
        for path in paths {
            if !self.waiting.contains(path) {
                continue;
            }
            if self.asked.insert(path.clone()) {
                self.requested.push_back(path.clone());
            }
            let Some(folder) = path.parent() else {
                continue;
            };
            if !self.asked_folders.insert(folder.clone()) {
                continue;
            }
            if let Some(beside) = self.by_folder.get(&folder) {
                let others = beside.iter().filter(|other| *other != path).cloned();
                self.neighbours.extend(others);
            }
        }
    }

    /// Up to `count` waiting files to parse next, each handed out once: those asked for, then
    /// their neighbours, then the rest.
    pub fn next(&mut self, count: usize) -> Vec<RepoPath> {
        let mut next = Vec::new();
        while next.len() < count {
            let candidate = self
                .requested
                .pop_front()
                .or_else(|| self.neighbours.pop_front())
                .or_else(|| self.rest.pop());
            let Some(path) = candidate else {
                break;
            };
            if self.waiting.remove(&path) {
                next.push(path);
            }
        }
        next
    }

    /// How many files still wait.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::PathError;

    use super::*;

    /// The waiting files of `paths`, each with its time.
    fn crawl(paths: &[(&str, i64)]) -> Result<Crawl, PathError> {
        let waiting = paths
            .iter()
            .map(|(path, time)| Ok((RepoPath::new(path)?, *time)))
            .collect::<Result<_, PathError>>()?;
        Ok(Crawl::new(waiting))
    }

    /// The names of `paths`.
    fn names(paths: &[RepoPath]) -> Vec<&str> {
        paths.iter().map(RepoPath::as_str).collect()
    }

    /// Unasked, files come the most recently changed first, equally new ones in path order.
    #[test]
    fn the_rest_come_newest_first() -> Result<(), PathError> {
        let mut crawl = crawl(&[
            ("a/1.py", 10),
            ("a/2.py", 20),
            ("b/3.py", 30),
            ("c/4.py", 5),
            ("c/5.py", 5),
        ])?;
        assert_eq!(crawl.waiting(), 5);
        assert_eq!(
            names(&crawl.next(10)),
            ["b/3.py", "a/2.py", "a/1.py", "c/4.py", "c/5.py"]
        );
        assert_eq!(crawl.waiting(), 0);
        Ok(())
    }

    /// Files asked for come first, then the other waiting files of their folders, then the rest;
    /// a file is handed out once, asked for again or not, and one not waiting is not handed out.
    #[test]
    fn requests_and_their_neighbours_come_first() -> Result<(), PathError> {
        let mut crawl = crawl(&[
            ("a/1.py", 10),
            ("a/2.py", 20),
            ("b/3.py", 30),
            ("c/4.py", 5),
        ])?;
        crawl.request(&[RepoPath::new("a/1.py")?, RepoPath::new("x/none.py")?]);
        assert_eq!(names(&crawl.next(2)), ["a/1.py", "a/2.py"]);
        crawl.request(&[RepoPath::new("a/1.py")?, RepoPath::new("c/4.py")?]);
        assert_eq!(names(&crawl.next(1)), ["c/4.py"]);
        assert_eq!(crawl.next(0), Vec::<RepoPath>::new());
        assert_eq!(names(&crawl.next(10)), ["b/3.py"]);
        assert_eq!(crawl.next(10), Vec::<RepoPath>::new());
        Ok(())
    }

    /// A file asked for a thousand times is queued once, and its folder's other files once.
    #[test]
    fn requests_asked_again_are_queued_once() -> Result<(), PathError> {
        let mut crawl = crawl(&[("a/1.py", 10), ("a/2.py", 20), ("a/3.py", 30)])?;
        let one = RepoPath::new("a/1.py")?;
        for _ in 0..1000 {
            crawl.request(std::slice::from_ref(&one));
        }
        crawl.request(&[RepoPath::new("a/2.py")?]);
        assert_eq!((crawl.requested.len(), crawl.neighbours.len()), (2, 2));
        assert_eq!(names(&crawl.next(10)), ["a/1.py", "a/2.py", "a/3.py"]);
        Ok(())
    }
}
