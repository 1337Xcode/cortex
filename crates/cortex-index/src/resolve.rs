//! Tying references to definitions, when what the file says settles which one. A bare name an
//! import in its scope binds is tied to the one definition, made directly in the files the import
//! names, of the name the import takes, when every import binding it there finds that same one; a
//! file that only imports the name itself, as a package's `__init__.py` or a TypeScript index file
//! does, is followed, at most four files deep. An import's own reference, and a name an import
//! binds, are tied that way or not at all: the name is the import's, so what the import cannot
//! settle stays unresolved. Any other reference is tied to the one definition with its name in its
//! own file, or else the one among the files of its language family. Anything less certain stays
//! unresolved.
//!
//! A name written after a qualifier that imports give a module is that module's: it is tied to
//! the one definition of the name the module makes or passes on, when every such import finds the
//! same one, and stays unresolved otherwise, as it does when an import gives the qualifier
//! something outside the repository.
//!
//! A family is a group of languages whose code calls each other's definitions directly:
//! TypeScript, TSX and JavaScript; C and C++; Java, Kotlin and Scala. Every other language is a
//! family of its own, so a Python call is never tied to a function of another language that
//! happens to share its name.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Identifier, Language, RefKind, RepoPath, Resolution, SymbolKind};
use cortex_store::{FileId, RefName, Resolved, StoreError, SymbolId, SymbolName, Target, Writer};

use crate::bindings::{Bindings, Lookup, Reach};

/// How many references are written per transaction.
const RESOLUTIONS_PER_TRANSACTION: usize = 4096;

/// How many files that only pass a name on, by importing it themselves, are followed to find its
/// definition. Package `__init__.py` files and TypeScript index files rarely chain more than two;
/// the cap bounds the work a crafted chain can cause.
const MAX_FILES_PASSED_THROUGH: usize = 4;

/// How many times one search for a name may look into a file, looks into one file at several
/// depths included. An index file with a few hundred `export *` lines is searched whole; past the
/// budget the name is left unresolved, which bounds the work crafted re-exports can cause.
const MAX_FILES_SEARCHED: usize = 256;

/// What a name finds among some definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    /// Exactly one definition.
    One(SymbolId),
    /// More than one.
    Several,
}

/// What two lookups find together: one definition when they find the same one, or when only one
/// of them finds anything.
fn merge(first: Option<Found>, second: Option<Found>) -> Option<Found> {
    match (first, second) {
        (None, found) | (found, None) => found,
        (Some(Found::One(a)), Some(Found::One(b))) if a == b => Some(Found::One(a)),
        _ => Some(Found::Several),
    }
}

/// What the name pass knows of the files: each stored file's row by path, and the bindings of
/// each file this index wrote, by row.
struct Written<'a> {
    /// Every stored file's row.
    files: BTreeMap<&'a RepoPath, FileId>,
    /// The bindings of each written file.
    bindings: BTreeMap<FileId, &'a Bindings>,
}

impl<'a> Written<'a> {
    /// What `bindings`, each written file's by path, say, with `files`, every stored file and its
    /// row.
    fn new(files: &'a [(RepoPath, FileId)], bindings: &'a BTreeMap<RepoPath, Bindings>) -> Self {
        let files: BTreeMap<&RepoPath, FileId> =
            files.iter().map(|(path, id)| (path, *id)).collect();
        let bindings = bindings
            .iter()
            .filter_map(|(path, bindings)| Some((*files.get(path)?, bindings)))
            .collect();
        Self { files, bindings }
    }
}

/// One search for a name: where it looked, with the fewest files passed through to get there,
/// and how many more looks it may take.
struct Search<'a> {
    /// Each file and name looked into, with the fewest files passed through to get there.
    searched: BTreeMap<(FileId, &'a Identifier), usize>,
    /// The looks left.
    budget: usize,
}

/// What earlier lookups found, so that each is made once per index.
#[derive(Default)]
struct Memo<'a> {
    /// What each file's import at a position, among those that take a name, finds.
    imports: BTreeMap<(FileId, usize), Option<Found>>,
    /// What each name finds in each module a file reaches, by the module's position.
    members: BTreeMap<(FileId, usize, &'a Identifier), Option<Found>>,
}

/// The definitions of each name, per file and per language family.
struct Definitions<'a> {
    /// By file and name.
    in_file: BTreeMap<(FileId, &'a str), Found>,
    /// By file and name, only those made directly in the file: what an import can take.
    top_level: BTreeMap<(FileId, &'a str), Found>,
    /// By language family and name.
    in_family: BTreeMap<(Language, &'a str), Found>,
    /// The definitions of modules.
    modules: BTreeSet<SymbolId>,
}

impl<'a> Definitions<'a> {
    /// The definitions among `symbols`.
    fn of(symbols: &'a [SymbolName]) -> Self {
        let mut definitions = Self {
            in_file: BTreeMap::new(),
            top_level: BTreeMap::new(),
            in_family: BTreeMap::new(),
            modules: BTreeSet::new(),
        };
        for symbol in symbols {
            let name = symbol.name.as_str();
            if symbol.kind == SymbolKind::Module {
                definitions.modules.insert(symbol.id);
            }
            count(&mut definitions.in_file, (symbol.file, name), symbol.id);
            if symbol.is_top_level {
                count(&mut definitions.top_level, (symbol.file, name), symbol.id);
            }
            if let Some(language) = symbol.language {
                count(
                    &mut definitions.in_family,
                    (family(language), name),
                    symbol.id,
                );
            }
        }
        definitions
    }

    /// What `reference`, at `position` among its file's references, is tied to. In a file this
    /// index wrote, as its bindings say: an import's own reference through its import, a bound
    /// name through the imports that bind it, a name after a qualifier through the modules the
    /// qualifier's imports reach, each only that way; any other reference by its name. In a file
    /// it did not write, whose imports were not read: a tie through an import stays as it was,
    /// and any other reference goes by its name.
    fn target(
        &self,
        reference: &'a RefName,
        position: usize,
        written: &Written<'a>,
        memo: &mut Memo<'a>,
    ) -> Option<Target> {
        let Some(bindings) = written.bindings.get(&reference.file) else {
            let is_through_import = reference.kind == RefKind::Import
                || reference
                    .target
                    .is_some_and(|target| target.resolution == Resolution::ImportPath);
            return if is_through_import {
                reference.target
            } else {
                self.by_name(reference)
            };
        };
        let found = match bindings.lookup(position) {
            Some(Lookup::Import(Some(taken))) => {
                self.through_import(reference.file, *taken, written, memo)
            }
            Some(Lookup::Bound(takens)) => {
                let finds: Vec<Option<Found>> = takens
                    .iter()
                    .map(|taken| self.through_import(reference.file, *taken, written, memo))
                    .collect();
                agreed(&finds)
            }
            Some(Lookup::Member(reaches)) => {
                if self.names_a_definition(reference.file, reaches, written, memo) {
                    return self.by_name(reference);
                }
                let finds: Vec<Option<Found>> = reaches
                    .iter()
                    .map(|reach| match reach {
                        Reach::Module(module) | Reach::Submodule { module, .. } => {
                            self.in_module(reference.file, *module, &reference.name, written, memo)
                        }
                        Reach::Outside => None,
                    })
                    .collect();
                agreed(&finds)
            }
            // A reference past the file's lookups means the two lists came apart; such a one goes
            // by its name, as it would without imports.
            Some(Lookup::ByName) | None if reference.kind != RefKind::Import => {
                return self.by_name(reference);
            }
            Some(Lookup::Import(None) | Lookup::ByName) | None => None,
        };
        // A name an import binds is that import's: what the imports cannot settle stays unresolved.
        found.and_then(|found| tie(found, Resolution::ImportPath))
    }

    /// What the import at `taken`, among the imports of the written file `file` that take a
    /// name, finds: its name made directly in the files it names, following files that pass it
    /// on.
    fn through_import(
        &self,
        file: FileId,
        taken: usize,
        written: &Written<'a>,
        memo: &mut Memo<'a>,
    ) -> Option<Found> {
        if let Some(found) = memo.imports.get(&(file, taken)) {
            return *found;
        }
        let found = written
            .bindings
            .get(&file)
            .and_then(|bindings| bindings.taken(taken))
            .and_then(|import| self.defined_in(&import.name, &import.files, written));
        memo.imports.insert((file, taken), found);
        found
    }

    /// Whether one of `reaches`, from the written file `file`, is a submodule whose name the
    /// import's own module makes as something other than a module: then the qualifier names that
    /// definition, a class or a value, not the submodule.
    fn names_a_definition(
        &self,
        file: FileId,
        reaches: &[Reach],
        written: &Written<'a>,
        memo: &mut Memo<'a>,
    ) -> bool {
        reaches.iter().any(|reach| {
            let Reach::Submodule { taken, .. } = reach else {
                return false;
            };
            match self.through_import(file, *taken, written, memo) {
                Some(Found::One(symbol)) => !self.modules.contains(&symbol),
                Some(Found::Several) => true,
                None => false,
            }
        })
    }

    /// What `name` finds in the module at `module` among those the written file `file` reaches:
    /// made directly in the module's files, or passed on by them.
    fn in_module(
        &self,
        file: FileId,
        module: usize,
        name: &'a Identifier,
        written: &Written<'a>,
        memo: &mut Memo<'a>,
    ) -> Option<Found> {
        if let Some(found) = memo.members.get(&(file, module, name)) {
            return *found;
        }
        let found = written
            .bindings
            .get(&file)
            .and_then(|bindings| self.defined_in(name, bindings.module(module), written));
        memo.members.insert((file, module, name), found);
        found
    }

    /// What `reference` is tied to by its name alone: the one definition with its name in its
    /// file, else the one with its name in its language family, else nothing.
    fn by_name(&self, reference: &RefName) -> Option<Target> {
        let name = reference.name.as_str();
        // A name defined more than once in the file is defined more than once in its family too.
        if let Some(found) = self.in_file.get(&(reference.file, name)) {
            return tie(*found, Resolution::SameFile);
        }
        let family = family(reference.language?);
        tie(*self.in_family.get(&(family, name))?, Resolution::NameOnly)
    }

    /// What `name` finds made directly in `files`, following the files that only pass it on.
    fn defined_in<'f>(
        &self,
        name: &'a Identifier,
        files: impl IntoIterator<Item = &'f RepoPath>,
        written: &Written<'a>,
    ) -> Option<Found> {
        let mut search = Search {
            searched: BTreeMap::new(),
            budget: MAX_FILES_SEARCHED,
        };
        self.search(name, files, 0, written, &mut search)
    }

    /// What `name` finds made directly in `files`, having passed through `passed` files. A file
    /// that does not make it is followed through its top-level imports that bind it and its
    /// imports of every name, while the cap allows. A file is not searched again for a name it
    /// was searched for after passing through as few files or fewer; reached by a shorter way, it
    /// is, since the cap may have cut the longer search short. The search stops as soon as it
    /// finds two definitions, and finds two when it runs out of looks.
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
            let Some(budget) = search.budget.checked_sub(1) else {
                return Some(Found::Several);
            };
            search.budget = budget;
            let here = match self.top_level.get(&(file, name.as_str())) {
                Some(made) => Some(*made),
                None if passed < MAX_FILES_PASSED_THROUGH => {
                    self.passed_on(name, file, passed + 1, written, search)
                }
                None => None,
            };
            found = merge(found, here);
            if found == Some(Found::Several) {
                return found;
            }
        }
        found
    }

    /// What `name` finds through the top-level imports of the written file `file` that bring it
    /// in, having passed through `passed` files.
    fn passed_on(
        &self,
        name: &'a Identifier,
        file: FileId,
        passed: usize,
        written: &Written<'a>,
        search: &mut Search<'a>,
    ) -> Option<Found> {
        let bindings = written.bindings.get(&file)?;
        let mut found = None;
        for taken in bindings.exports(name) {
            let Some(import) = bindings.taken(*taken) else {
                continue;
            };
            let through = self.search(&import.name, &import.files, passed, written, search);
            found = merge(found, through);
            if found == Some(Found::Several) {
                return found;
            }
        }
        let everything = self.search(name, bindings.everything(), passed, written, search);
        merge(found, everything)
    }
}

/// What several imports binding one name find together: the one definition when every one of
/// them finds that same one, else nothing, since which import holds is not known.
fn agreed(finds: &[Option<Found>]) -> Option<Found> {
    let mut agreed = None;
    for found in finds {
        match (*found, agreed) {
            (Some(Found::One(symbol)), None) => agreed = Some(symbol),
            (Some(Found::One(symbol)), Some(earlier)) if symbol == earlier => {}
            _ => return None,
        }
    }
    agreed.map(Found::One)
}

/// Adds the definition `symbol` under `key`.
fn count<K: Ord>(found: &mut BTreeMap<K, Found>, key: K, symbol: SymbolId) {
    found
        .entry(key)
        .and_modify(|found| *found = Found::Several)
        .or_insert(Found::One(symbol));
}

/// The target `found` gives with `resolution`: one, or none when the name is ambiguous.
fn tie(found: Found, resolution: Resolution) -> Option<Target> {
    match found {
        Found::One(symbol) => Some(Target { symbol, resolution }),
        Found::Several => None,
    }
}

/// The language family `language` belongs to, named by its first member.
fn family(language: Language) -> Language {
    match language {
        Language::Tsx | Language::JavaScript => Language::TypeScript,
        Language::Cpp => Language::C,
        Language::Kotlin | Language::Scala => Language::Java,
        other => other,
    }
}

/// Ties every stored reference to the definition its file's imports or its name settle, or marks
/// it unresolved, and writes the references whose target changed, in transactions of
/// [`RESOLUTIONS_PER_TRANSACTION`]. `bindings` holds what the imports of each file this index
/// wrote bring in, by path, with a lookup for each of its references in the order they were
/// written.
///
/// # Errors
///
/// Store errors; the transactions committed before stay written.
pub(crate) fn resolve_names(
    writer: &mut Writer,
    bindings: &BTreeMap<RepoPath, Bindings>,
) -> Result<(), StoreError> {
    let queries = writer.queries();
    let files = queries.file_ids()?;
    let symbols = queries.symbol_names()?;
    let refs = queries.ref_names()?;
    let written = Written::new(&files, bindings);
    let definitions = Definitions::of(&symbols);
    let mut memo = Memo::default();
    // A file's references come back in the order they were written, which is its lookups' order.
    let mut positions: BTreeMap<FileId, usize> = BTreeMap::new();
    let mut changed: Vec<Resolved> = Vec::new();
    for reference in &refs {
        let next = positions.entry(reference.file).or_insert(0);
        let position = *next;
        *next += 1;
        let target = definitions.target(reference, position, &written, &mut memo);
        if target != reference.target {
            changed.push(Resolved {
                reference: reference.id,
                target,
            });
        }
    }
    for batch in changed.chunks(RESOLUTIONS_PER_TRANSACTION) {
        let mut transaction = writer.transaction()?;
        transaction.set_resolutions(batch)?;
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Languages that call each other's definitions directly share a family; the others stand
    /// alone.
    #[test]
    fn families_join_languages_that_call_each_other() {
        let cases = [
            (Language::TypeScript, Language::TypeScript),
            (Language::Tsx, Language::TypeScript),
            (Language::JavaScript, Language::TypeScript),
            (Language::C, Language::C),
            (Language::Cpp, Language::C),
            (Language::Java, Language::Java),
            (Language::Kotlin, Language::Java),
            (Language::Scala, Language::Java),
            (Language::Python, Language::Python),
            (Language::Go, Language::Go),
            (Language::Sql, Language::Sql),
        ];
        for (language, expected) in cases {
            assert_eq!(family(language), expected, "{language:?}");
        }
    }
}
