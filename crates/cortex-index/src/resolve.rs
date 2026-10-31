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

pub(crate) mod search;

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Identifier, Language, RefKind, RepoPath, Resolution, SymbolKind};
use cortex_store::{
    FileId, PassedOn, Queries, RefId, RefName, Resolved, StoreError, SymbolId, SymbolName, Target,
    Writer,
};

use crate::bindings::{Bindings, Lookup, Reach};
use crate::name_changes::{NameChanges, may_find_otherwise};

/// How many references are written per transaction.
const RESOLUTIONS_PER_TRANSACTION: usize = 4096;

/// What a name finds among some definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    /// Exactly one definition.
    One(SymbolId),
    /// More than one.
    Several,
}

/// What the name pass knows of the files: each stored file's row by path, the bindings of each
/// file this pass read, by row, and what other files pass on, as stored.
struct Written<'a> {
    /// Every stored file's row the pass may meet.
    files: BTreeMap<&'a RepoPath, FileId>,
    /// The bindings of each file this pass read.
    bindings: BTreeMap<FileId, &'a Bindings>,
    /// What the files this pass did not read pass on, as stored, by row.
    stored: &'a BTreeMap<FileId, Vec<PassedOn>>,
}

impl<'a> Written<'a> {
    /// What `bindings`, each read file's by path, say, with `files`, the stored files and their
    /// rows, and `stored`, what the other files pass on.
    fn new(
        files: &'a [(RepoPath, FileId)],
        bindings: &'a BTreeMap<RepoPath, Bindings>,
        stored: &'a BTreeMap<FileId, Vec<PassedOn>>,
    ) -> Self {
        let files: BTreeMap<&RepoPath, FileId> =
            files.iter().map(|(path, id)| (path, *id)).collect();
        let bindings = bindings
            .iter()
            .filter_map(|(path, bindings)| Some((*files.get(path)?, bindings)))
            .collect();
        Self {
            files,
            bindings,
            stored,
        }
    }

    /// What `file` passes on: from its bindings when this pass read it, else as stored.
    fn passes(&self, file: FileId) -> Option<Passes<'a>> {
        if let Some(bindings) = self.bindings.get(&file) {
            return Some(Passes::Read(bindings));
        }
        self.stored.get(&file).map(|passed| Passes::Stored(passed))
    }
}

/// What a file passes on of what its top-level imports bring in.
#[derive(Clone, Copy)]
enum Passes<'a> {
    /// From its bindings, read in this pass.
    Read(&'a Bindings),
    /// As stored.
    Stored(&'a [PassedOn]),
}

impl<'a> Passes<'a> {
    /// What the imports that pass `name` on take: each one's name and the files it names.
    fn taken_for(self, name: &Identifier) -> Vec<(&'a Identifier, &'a [RepoPath])> {
        match self {
            Self::Read(bindings) => bindings
                .exports(name)
                .iter()
                .filter_map(|position| bindings.taken(*position))
                .map(|taken| (&taken.name, taken.files.as_slice()))
                .collect(),
            Self::Stored(passed) => passed
                .iter()
                .filter_map(|passed| match passed {
                    PassedOn::Name {
                        name: passed_name,
                        taken,
                        files,
                        ..
                    } if passed_name == name => Some((taken, files.as_slice())),
                    _ => None,
                })
                .collect(),
        }
    }

    /// The files whose every name it passes on.
    fn everything(self) -> Vec<&'a RepoPath> {
        match self {
            Self::Read(bindings) => bindings.everything().iter().collect(),
            Self::Stored(passed) => passed
                .iter()
                .filter_map(|passed| match passed {
                    PassedOn::Every(path) => Some(path),
                    _ => None,
                })
                .collect(),
        }
    }

    /// Whether it passes on every name of a module outside the repository.
    fn passes_on_outside(self) -> bool {
        match self {
            Self::Read(bindings) => bindings.passes_on_outside(),
            Self::Stored(passed) => passed.contains(&PassedOn::Outside),
        }
    }
}

/// What earlier lookups found, so that each is made once per index whatever a file repeats.
#[derive(Default)]
struct Memo<'a> {
    /// What each written file's imports find, by the position of what they take.
    imports: BTreeMap<(FileId, usize), Option<Found>>,
    /// What each written file's groups of imports binding one bare name find together.
    bound: BTreeMap<(FileId, usize), Option<Found>>,
    /// Whether each written file's group of imports binding one qualifier gives it a definition.
    definitions: BTreeMap<(FileId, usize), bool>,
    /// What each name finds after the qualifier a written file's group of imports binds.
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

    /// What `reference`, at `position` among its file's references, is tied to, and whether it
    /// goes through an import. In a file this pass read, as its bindings say: an import's own
    /// reference through its import, a bound name through the imports that bind it, a name after a
    /// qualifier through the modules the qualifier's imports reach, each only that way; any other
    /// reference by its name. In a file it did not read, whose imports are not known: a reference
    /// that goes through an import stays as it was, tied or not, and any other goes by its name.
    fn target(
        &self,
        reference: &'a RefName,
        position: usize,
        written: &Written<'a>,
        memo: &mut Memo<'a>,
    ) -> (Option<Target>, bool) {
        let file = reference.file;
        let Some(bindings) = written.bindings.get(&file) else {
            if reference.goes_through_import || reference.kind == RefKind::Import {
                return (reference.target, true);
            }
            return (self.by_name(reference), false);
        };
        let found = match bindings.lookup(position) {
            Some(Lookup::Import(Some(taken))) => self.through_import(file, *taken, written, memo),
            Some(Lookup::Bound(group)) => self.through_bound(file, *group, written, memo),
            Some(Lookup::Member(group)) => {
                if self.gives_a_definition(file, *group, written, memo) {
                    return (self.by_name(reference), false);
                }
                self.through_member(file, (*group, &reference.name), written, memo)
            }
            // A reference past the file's lookups means the two lists came apart; such a one goes
            // by its name, as it would without imports.
            Some(Lookup::ByName) | None if reference.kind != RefKind::Import => {
                return (self.by_name(reference), false);
            }
            Some(Lookup::Import(None) | Lookup::ByName) | None => None,
        };
        // A name an import binds is that import's: what the imports cannot settle stays unresolved.
        // INVARIANT: silent-when-unsure
        (
            found.and_then(|found| tie(found, Resolution::ImportPath)),
            true,
        )
    }

    /// What the written file `file`'s imports find for what they take at `taken`: its name made
    /// directly in the files they name, or passed on by them.
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

    /// What the imports of the written file `file`'s group at `group`, binding one bare name,
    /// find together: the one definition when they all find it.
    fn through_bound(
        &self,
        file: FileId,
        group: usize,
        written: &Written<'a>,
        memo: &mut Memo<'a>,
    ) -> Option<Found> {
        if let Some(found) = memo.bound.get(&(file, group)) {
            return *found;
        }
        let takens = written
            .bindings
            .get(&file)
            .map_or(&[][..], |bindings| bindings.bound(group));
        let finds: Vec<Option<Found>> = takens
            .iter()
            .map(|taken| self.through_import(file, *taken, written, memo))
            .collect();
        let found = agreed(&finds);
        memo.bound.insert((file, group), found);
        found
    }

    /// Whether the written file `file`'s group at `group`, binding one qualifier, gives it a
    /// definition of the repository rather than modules: one of its imports takes the name of a
    /// submodule that the import's own module makes as something other than a module.
    fn gives_a_definition(
        &self,
        file: FileId,
        group: usize,
        written: &Written<'a>,
        memo: &mut Memo<'a>,
    ) -> bool {
        if let Some(gives) = memo.definitions.get(&(file, group)) {
            return *gives;
        }
        let reaches = written
            .bindings
            .get(&file)
            .map_or(&[][..], |bindings| bindings.reached(group));
        let gives = reaches.iter().any(|reach| {
            let Reach::Submodule { taken, .. } = reach else {
                return false;
            };
            match self.through_import(file, *taken, written, memo) {
                Some(Found::One(symbol)) => !self.modules.contains(&symbol),
                Some(Found::Several) => true,
                None => false,
            }
        });
        memo.definitions.insert((file, group), gives);
        gives
    }

    /// What `name`, written after the qualifier the written file `file`'s group at `group` binds,
    /// finds: the one definition every module the group reaches makes or passes on, and nothing
    /// when one of them is outside the repository.
    fn through_member(
        &self,
        file: FileId,
        (group, name): (usize, &'a Identifier),
        written: &Written<'a>,
        memo: &mut Memo<'a>,
    ) -> Option<Found> {
        if let Some(found) = memo.members.get(&(file, group, name)) {
            return *found;
        }
        let bindings = written.bindings.get(&file);
        let reaches = bindings.map_or(&[][..], |bindings| bindings.reached(group));
        let finds: Vec<Option<Found>> = reaches
            .iter()
            .map(|reach| match reach {
                Reach::Module(module) | Reach::Submodule { module, .. } => bindings
                    .and_then(|bindings| self.defined_in(name, bindings.module(*module), written)),
                Reach::Outside => None,
            })
            .collect();
        let found = agreed(&finds);
        memo.members.insert((file, group, name), found);
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
}

/// What several imports or routes to one name find together: the one definition when every one
/// of them finds that same one, else nothing, since which one holds is not known.
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

/// The languages of the family named by `family`, as [`family`] names it.
fn members(family: Language) -> Vec<Language> {
    match family {
        Language::TypeScript => vec![Language::TypeScript, Language::Tsx, Language::JavaScript],
        Language::C => vec![Language::C, Language::Cpp],
        Language::Java => vec![Language::Java, Language::Kotlin, Language::Scala],
        other => vec![other],
    }
}

/// The language family `language` belongs to, named by its first member.
pub(crate) fn family(language: Language) -> Language {
    match language {
        Language::Tsx | Language::JavaScript => Language::TypeScript,
        Language::Cpp => Language::C,
        Language::Kotlin | Language::Scala => Language::Java,
        other => other,
    }
}

/// What the name pass tied.
pub(crate) struct NamePass {
    /// Each tied reference it read that sits in a definition, as its file, that definition and
    /// its target.
    pub(crate) ties: Vec<(FileId, SymbolId, Target)>,
    /// The definitions holding a reference whose target changed.
    pub(crate) changed_sources: BTreeSet<SymbolId>,
    /// The files holding a reference whose target changed.
    pub(crate) changed_files: BTreeSet<FileId>,
}

/// The stored rows a name pass reads.
struct Scope {
    /// Each stored file the pass may meet, with its row.
    files: Vec<(RepoPath, FileId)>,
    /// The definitions the references, and the imports, may find: every definition of each name
    /// they look for.
    symbols: Vec<SymbolName>,
    /// The references to tie, by id: every reference of a file with bindings.
    refs: Vec<RefName>,
    /// What the files a name may be followed through, and that the pass does not read, pass on.
    stored: BTreeMap<FileId, Vec<PassedOn>>,
    /// Definitions a stored SCIP tie no longer holds for: those of files changed since the SCIP
    /// index was written, which a full index leaves out of what the SCIP index says.
    stale_scip: BTreeSet<SymbolId>,
}

/// Ties every stored reference to the definition its file's imports or its name settle, or marks
/// it unresolved, and writes the references whose target changed, in transactions of
/// [`RESOLUTIONS_PER_TRANSACTION`]. `bindings` holds what the imports of each file this index
/// wrote bring in, by path, with a lookup for each of its references in the order they were
/// written. A reference `scip` ties, from the repository's SCIP index, is tied as it says, before
/// any rule here. Returns what it tied: each tied reference that sits in a definition, and the
/// definitions holding a reference whose target changed.
///
/// # Errors
///
/// Store errors; the transactions committed before stay written.
pub(crate) fn resolve_names(
    writer: &mut Writer,
    bindings: &BTreeMap<RepoPath, Bindings>,
    scip: &BTreeMap<RefId, SymbolId>,
) -> Result<NamePass, StoreError> {
    let queries = writer.queries();
    let scope = Scope {
        files: queries.file_ids()?,
        symbols: queries.symbol_names()?,
        refs: queries.ref_names()?,
        stored: queries.every_passes_on()?,
        stale_scip: BTreeSet::new(),
    };
    resolve(writer, &scope, bindings, Some(scip))
}

/// Ties again the references a change can concern, as [`resolve_names`] would tie them: every
/// reference in the files `bindings` holds, read in this pass, and every reference named in
/// `names`, the names whose definitions changed. Only the definitions of the names those
/// references and the imports look for are read. Every other reference would keep its target,
/// since neither its file nor the definitions its name finds changed. The SCIP index is not read
/// again: a reference it tied keeps its tie, unless the tie is to one of `stale_scip`, the
/// definitions of the files written now, which a full index takes as changed since the SCIP index
/// was written; the files holding such a reference are among those `bindings` holds.
///
/// # Errors
///
/// Store errors; the transactions committed before stay written.
pub(crate) fn resolve_names_around(
    writer: &mut Writer,
    bindings: &BTreeMap<RepoPath, Bindings>,
    names: &NameChanges,
    stale_scip: &BTreeSet<SymbolId>,
) -> Result<NamePass, StoreError> {
    let queries = writer.queries();
    let mut files: BTreeMap<RepoPath, FileId> = BTreeMap::new();
    for path in bindings.keys() {
        if let Some(row) = queries.file(path)? {
            files.insert(path.clone(), row.id);
        }
    }
    let stored = reachable_surfaces(queries, bindings, &mut files)?;
    let mut refs: BTreeMap<RefId, RefName> = BTreeMap::new();
    for path in bindings.keys() {
        if let Some(file) = files.get(path) {
            refs.extend(
                queries
                    .ref_names_in_file(*file)?
                    .into_iter()
                    .map(|r| (r.id, r)),
            );
        }
    }
    refs.extend(
        by_name_elsewhere(queries, names)?
            .into_iter()
            .map(|r| (r.id, r)),
    );
    let mut wanted: BTreeSet<&Identifier> =
        refs.values().map(|reference| &reference.name).collect();
    wanted.extend(bindings.values().flat_map(Bindings::taken_names));
    wanted.extend(stored.values().flatten().filter_map(|passed| match passed {
        PassedOn::Name { taken, .. } => Some(taken),
        PassedOn::Every(_) | PassedOn::Outside => None,
    }));
    let symbols = definitions_for(queries, &wanted, &files, &refs)?;
    let bound: BTreeSet<FileId> = bindings
        .keys()
        .filter_map(|path| files.get(path).copied())
        .collect();
    let scope = Scope {
        files: files.into_iter().collect(),
        symbols,
        refs: refs.into_values().collect(),
        stored,
        stale_scip: stale_scip.clone(),
    };
    let mut pass = resolve(writer, &scope, bindings, None)?;
    // Only the files read in this pass had every reference tied: another file's ties are read back
    // whole once its changed targets are written, so that each of its definitions has them all.
    pass.ties.retain(|(file, _, _)| bound.contains(file));
    let queries = writer.queries();
    for file in pass.changed_files.difference(&bound) {
        for reference in queries.ref_names_in_file(*file)? {
            if let (Some(source), Some(target)) = (reference.symbol, reference.target) {
                pass.ties.push((*file, source, target));
            }
        }
    }
    Ok(pass)
}

/// The references elsewhere going by a name whose definitions changed, where what the name finds
/// may now differ: in a family whose count of the name's definitions reached one or none, before
/// or after (see [`may_find_otherwise`]), and in every family for a name whose definitions moved
/// between families. References that go through an import keep their ties in a file the pass
/// does not read, and are not read.
///
/// # Errors
///
/// Store errors.
fn by_name_elsewhere(
    queries: Queries<'_>,
    names: &NameChanges,
) -> Result<Vec<RefName>, StoreError> {
    let (counted, moved) = names.counted();
    let mut refs = Vec::new();
    for name in moved {
        refs.extend(queries.ref_names_by_name(name)?);
    }
    for ((name, named_family), delta) in counted {
        if moved.contains(name) {
            continue;
        }
        let found = queries.symbol_names_named_in(name, &members(*named_family), 3)?;
        let after = i64::try_from(found.len()).unwrap_or(i64::MAX);
        if !may_find_otherwise(after, *delta) {
            continue;
        }
        let in_family = queries
            .ref_names_by_name(name)?
            .into_iter()
            .filter(|reference| {
                reference
                    .language
                    .is_some_and(|language| family(language) == *named_family)
            });
        refs.extend(in_family);
    }
    Ok(refs)
}

/// The definitions the pass needs for the names it looks for, `wanted`: every definition in a file
/// it may look into, the `files` it reads or follows and the files of the references `refs` it
/// ties, and, for the name a reference goes by, the first two in the reference's language family,
/// which tell one definition there from several. A name finds nothing else: no other file is
/// looked into, and the family needs no more than two to be ambiguous.
///
/// # Errors
///
/// Store errors.
fn definitions_for(
    queries: Queries<'_>,
    wanted: &BTreeSet<&Identifier>,
    files: &BTreeMap<RepoPath, FileId>,
    refs: &BTreeMap<RefId, RefName>,
) -> Result<Vec<SymbolName>, StoreError> {
    let looked_into: BTreeSet<FileId> = files
        .values()
        .copied()
        .chain(refs.values().map(|reference| reference.file))
        .collect();
    let looked_into: Vec<FileId> = looked_into.into_iter().collect();
    let mut symbols: BTreeMap<SymbolId, SymbolName> = BTreeMap::new();
    for symbol in queries.symbol_names_in_files(&looked_into)? {
        if wanted.contains(&symbol.name) {
            symbols.insert(symbol.id, symbol);
        }
    }
    let mut families: BTreeSet<(&Identifier, Language)> = BTreeSet::new();
    for reference in refs.values() {
        if let Some(language) = reference.language {
            families.insert((&reference.name, family(language)));
        }
    }
    for (name, family) in families {
        for symbol in queries.symbol_names_named_in(name, &members(family), 2)? {
            symbols.insert(symbol.id, symbol);
        }
    }
    Ok(symbols.into_values().collect())
}

/// What the files a search from `bindings` can be followed through pass on, as stored, and the
/// rows of every file it can look into, added to `files`: the files the bindings name, and the
/// files those pass names on from, as far as a search follows. A file read in the pass is
/// followed through its bindings instead.
///
/// # Errors
///
/// Store errors.
fn reachable_surfaces(
    queries: Queries<'_>,
    bindings: &BTreeMap<RepoPath, Bindings>,
    files: &mut BTreeMap<RepoPath, FileId>,
) -> Result<BTreeMap<FileId, Vec<PassedOn>>, StoreError> {
    let mut stored = BTreeMap::new();
    let mut level: Vec<RepoPath> = bindings
        .values()
        .flat_map(Bindings::files)
        .cloned()
        .collect();
    for depth in 0..=search::MAX_FILES_PASSED_THROUGH {
        let mut next = Vec::new();
        for path in level {
            let file = if let Some(file) = files.get(&path) {
                *file
            } else {
                let Some(row) = queries.file(&path)? else {
                    continue;
                };
                files.insert(path.clone(), row.id);
                row.id
            };
            if bindings.contains_key(&path) || stored.contains_key(&file) {
                continue;
            }
            let passed = queries.passes_on(file)?;
            // A search looks into the last file it reaches, whose passing a name on makes the
            // name ambiguous there, but follows nothing further.
            if depth < search::MAX_FILES_PASSED_THROUGH {
                for passed in &passed {
                    match passed {
                        PassedOn::Name { files, .. } => next.extend(files.iter().cloned()),
                        PassedOn::Every(path) => next.push(path.clone()),
                        PassedOn::Outside => {}
                    }
                }
            }
            stored.insert(file, passed);
        }
        level = next;
    }
    Ok(stored)
}

/// Ties each reference of `scope` as the module documentation says and writes those whose
/// target changed. A reference `scip` ties is tied as it says; without `scip`, a stored SCIP tie
/// stands.
fn resolve(
    writer: &mut Writer,
    scope: &Scope,
    bindings: &BTreeMap<RepoPath, Bindings>,
    scip: Option<&BTreeMap<RefId, SymbolId>>,
) -> Result<NamePass, StoreError> {
    let written = Written::new(&scope.files, bindings, &scope.stored);
    let definitions = Definitions::of(&scope.symbols);
    let mut memo = Memo::default();
    // A file's references come back in the order they were written, which is its lookups' order.
    let mut positions: BTreeMap<FileId, usize> = BTreeMap::new();
    let mut changed: Vec<Resolved> = Vec::new();
    let mut ties = Vec::new();
    let mut changed_sources = BTreeSet::new();
    let mut changed_files = BTreeSet::new();
    for reference in &scope.refs {
        let next = positions.entry(reference.file).or_insert(0);
        let position = *next;
        *next += 1;
        let scip_tie = match scip {
            Some(scip) => scip.get(&reference.id).map(|symbol| Target {
                symbol: *symbol,
                resolution: Resolution::Scip,
            }),
            None => reference.target.filter(|target| {
                target.resolution == Resolution::Scip && !scope.stale_scip.contains(&target.symbol)
            }),
        };
        let (target, goes_through_import) = match scip_tie {
            Some(tie) => (Some(tie), false),
            None => definitions.target(reference, position, &written, &mut memo),
        };
        if let (Some(symbol), Some(target)) = (reference.symbol, target) {
            ties.push((reference.file, symbol, target));
        }
        if target != reference.target || goes_through_import != reference.goes_through_import {
            changed_files.insert(reference.file);
            changed_sources.extend(reference.symbol);
            changed.push(Resolved {
                reference: reference.id,
                target,
                goes_through_import,
            });
        }
    }
    for batch in changed.chunks(RESOLUTIONS_PER_TRANSACTION) {
        let mut transaction = writer.transaction()?;
        transaction.set_resolutions(batch)?;
        transaction.commit()?;
    }
    Ok(NamePass {
        ties,
        changed_sources,
        changed_files,
    })
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
