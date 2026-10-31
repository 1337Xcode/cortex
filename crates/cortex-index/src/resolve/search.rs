//! The search for a name among the files an import or a qualifier names, following the files that
//! pass the name on, as a package's `__init__.py` or a TypeScript index file does.
//!
//! A file speaks of a name through its own definition of it and through its top-level imports
//! that take it: these must all lead to the same definition, or the name is ambiguous, since which
//! of them holds depends on order or on what runs (a `try` import with a fallback, a later
//! definition). The file's imports of every name (`*`) count only beside them: they make the name
//! ambiguous when they find another definition, as a later `*` import wins in Python, and they
//! cannot stand in for imports of the name that find nothing, as an explicit re-export wins in
//! TypeScript. When nothing else speaks of the name, the one definition the `*` imports find is
//! the file's, unless one of them names a module outside the repository, which could pass on the
//! name too.

use std::collections::BTreeMap;

use cortex_core::{Identifier, RepoPath};
use cortex_store::FileId;

use super::{Definitions, Found, Written, agreed};

/// How many files that only pass a name on, by importing it themselves, are followed to find its
/// definition. Package `__init__.py` files and TypeScript index files rarely chain more than two;
/// the cap bounds the work a crafted chain can cause.
pub(super) const MAX_FILES_PASSED_THROUGH: usize = 4;

/// How many times one search for a name may look into a file, looks into one file at several
/// depths included. An index file with a few hundred `export *` lines is searched whole; past the
/// budget the name is left unresolved, which bounds the work crafted re-exports can cause.
const MAX_LOOKS: usize = 256;

/// One search for a name: where it looked, with the fewest files passed through to get there,
/// and how many more looks it may take.
struct Search<'a> {
    /// Each file and name looked into, with the fewest files passed through to get there.
    searched: BTreeMap<(FileId, &'a Identifier), usize>,
    /// The looks left.
    budget: usize,
}

impl<'a> Definitions<'a> {
    /// What `name` finds made directly in `files`, or passed on by them.
    pub(super) fn defined_in<'f>(
        &self,
        name: &'a Identifier,
        files: impl IntoIterator<Item = &'f RepoPath>,
        written: &Written<'a>,
    ) -> Option<Found> {
        let mut search = Search {
            searched: BTreeMap::new(),
            budget: MAX_LOOKS,
        };
        self.search(name, files, 0, written, &mut search)
    }

    /// What `name` finds in `files`, reached having passed through `passed` files. Several files
    /// may hold one module, as a Go package's do: one of them makes the name. A file is not
    /// searched again for a name it was searched for after passing through as few files or fewer;
    /// reached by a shorter way, it is, since the cap may have cut the longer search short. The
    /// search stops as soon as it finds two definitions, and finds two when it runs out of looks.
    fn search<'f>(
        &self,
        name: &'a Identifier,
        files: impl IntoIterator<Item = &'f RepoPath>,
        passed: usize,
        written: &Written<'a>,
        search: &mut Search<'a>,
    ) -> Option<Found> {
        let mut found = None;
        for path in files {
            let Some(&file) = written.files.get(path) else {
                continue;
            };
            let fewest = search.searched.entry((file, name)).or_insert(usize::MAX);
            if *fewest <= passed {
                continue;
            }
            *fewest = passed;
            // INVARIANT: budgeted
            let Some(budget) = search.budget.checked_sub(1) else {
                return Some(Found::Several);
            };
            search.budget = budget;
            found = merge(found, self.in_file(name, file, passed, written, search));
            if found == Some(Found::Several) {
                return found;
            }
        }
        found
    }

    /// What `name` finds in the file `file`, reached having passed through `passed` files: by the
    /// file's own definition and its imports of the name, which must agree, and by its imports of
    /// every name, as the module documentation says. What a file passes on comes from its bindings
    /// when the pass read it, else as stored; past the cap, a file's imports are not followed.
    fn in_file(
        &self,
        name: &'a Identifier,
        file: FileId,
        passed: usize,
        written: &Written<'a>,
        search: &mut Search<'a>,
    ) -> Option<Found> {
        let made = self.top_level.get(&(file, name.as_str())).copied();
        let Some(passing) = written.passes(file) else {
            return made;
        };
        let can_follow = passed < MAX_FILES_PASSED_THROUGH;
        let mut finds: Vec<Option<Found>> = made.into_iter().map(Some).collect();
        for (taken, files) in passing.taken_for(name) {
            finds.push(if can_follow {
                self.search(taken, files, passed + 1, written, search)
            } else {
                None
            });
        }
        // INVARIANT: silent-when-unsure
        let named = consensus(&finds);
        if named.is_none() && !finds.is_empty() {
            return None;
        }
        let everything = if can_follow {
            self.search(name, passing.everything(), passed + 1, written, search)
        } else {
            None
        };
        match (named, everything) {
            (None, Some(_)) if passing.passes_on_outside() => Some(Found::Several),
            (None, found) | (found, None) => found,
            (Some(Found::One(a)), Some(Found::One(b))) if a == b => Some(Found::One(a)),
            (Some(_), Some(_)) => Some(Found::Several),
        }
    }
}

/// What two files of one module find together: one definition when they find the same one, or
/// when only one of them finds anything.
fn merge(first: Option<Found>, second: Option<Found>) -> Option<Found> {
    match (first, second) {
        (None, found) | (found, None) => found,
        (Some(Found::One(a)), Some(Found::One(b))) if a == b => Some(Found::One(a)),
        _ => Some(Found::Several),
    }
}

/// What the routes a file gives to one name find together: nothing when none finds anything, the
/// one definition when every route finds that same one, else several, since which route holds is
/// not known.
fn consensus(finds: &[Option<Found>]) -> Option<Found> {
    if finds.iter().all(Option::is_none) {
        return None;
    }
    Some(agreed(finds).unwrap_or(Found::Several))
}
