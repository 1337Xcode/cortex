//! What a file's imports bring into it, as the name pass reads them: for each reference of the
//! file, the imports that decide what it names, if any; and what the file passes on to files that
//! import from it.
//!
//! An import's own reference stands for that import alone. A bare name is looked up the way the
//! languages with imports scope names: from the definition the use sits in outwards, through the
//! functions, methods and modules around it, to the file. The first of these that makes the name
//! itself, or has an import taking it under that name, decides: a definition there leaves the
//! name to the definitions of the file, an import there binds it. A class body is no scope for the
//! methods in it: they reach its names through `self` or `this`. A name written after a
//! qualifier, `x.strip()` or `orders.Place()`, is not bound by an import of the bare name.
//!
//! A qualifier that is a plain name is looked up through the same scopes. When imports there
//! give it a module (a Go package, a namespace, a Python or Rust module), the name after it is
//! that module's, found among the module's files; when they give it something outside the
//! repository, the name is outside too and stays unresolved. When an import gives it a
//! definition of the repository, a class or a value, the name is one of that definition's
//! members, which only the name itself can find.
//!
//! An import of every name (`*`) binds no bare name: which names it brings in depends on each
//! language's rules of visibility and order. It does pass names on, as an import that takes a name
//! does: a file importing a name from this one finds it through them. A re-export, which only
//! passes names on, binds nothing in the file itself.
//!
//! Parameters and local variables are not known here, so a parameter named like an import does not
//! hide it.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Identifier, RefKind, RepoPath, SymbolKind};
use cortex_store::{RefRecord, SymbolRecord};

use crate::extract::Extraction;
use crate::imports::Targets;
use crate::references::Qualifier;
use crate::specifiers::{Import, Imported};

/// One name an import takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Taken {
    /// The name, as the module defines it.
    pub(crate) name: Identifier,
    /// The files the import names as its module, where the name should be defined.
    pub(crate) files: Vec<RepoPath>,
}

/// Where the definition of one reference is looked for first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// No import decides: the reference goes by its name.
    ByName,
    /// An import's own reference, standing for the import at this position of the file's
    /// imports that take a name; `None` when it could not be paired with one, and then it stays
    /// unresolved.
    Import(Option<usize>),
    /// A bare name bound by these imports, by position: the one definition they all find, or
    /// nothing.
    Bound(Vec<usize>),
    /// A name written after a qualifier that imports give these modules or outside things: the
    /// one definition the modules all make or pass on, or nothing.
    Member(Vec<Reach>),
}

/// What a qualifier reaches through one import that binds its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    /// A module of the repository, by position among the modules the file's imports reach.
    Module(usize),
    /// The module of the name an import takes, by position among the modules, unless the
    /// import's own module makes that name as something other than a module: Python's
    /// `from shop import models` reaches `shop/models.py` unless `shop/__init__.py` defines
    /// `models`. `taken` is the import's position among the imports that take a name.
    Submodule {
        /// The import's position among the imports that take a name.
        taken: usize,
        /// The submodule's position among the modules.
        module: usize,
    },
    /// Something outside the repository, whose members are outside too.
    Outside,
}

/// What one file's imports bring into it and pass on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Bindings {
    /// The imports that take a name, in the order they appear.
    taken: Vec<Taken>,
    /// The file's top-level imports that take a name, by the name each passes on: its alias,
    /// else the name. A file importing a name from this one finds it through these.
    exports: BTreeMap<Identifier, Vec<usize>>,
    /// The files whose every name the file's top-level `*` imports pass on.
    everything: BTreeSet<RepoPath>,
    /// The files of each module a qualifier reaches through the file's imports.
    modules: Vec<Vec<RepoPath>>,
    /// For each reference of the file, in the order they are written to the store, where its
    /// definition is looked for first.
    lookups: Vec<Lookup>,
}

/// The imports that bind a bare name in the file: the position of each, by the scope it is made
/// in (`None` at the top of the file) and the name it binds.
type Bound<'a> = BTreeMap<(Option<usize>, &'a Identifier), Vec<usize>>;

/// What the imports binding each name reach when it is written as a qualifier, by the scope they
/// are made in and the name: a module or something outside, or `None` for a definition of the
/// repository.
type Reaching<'a> = BTreeMap<(Option<usize>, &'a Identifier), Vec<Option<Reach>>>;

impl Bindings {
    /// The bindings of the file read as `extraction`, whose imports name `targets` and reach their
    /// modules by `module_names` when they take them whole with no alias, one of each for each
    /// import in order.
    pub(crate) fn of(
        extraction: &Extraction,
        targets: &[Targets],
        module_names: &[Option<Identifier>],
    ) -> Self {
        let pairs = pair_own_references(extraction);
        let scopes = import_scopes(extraction, &pairs);
        let mut read = Read::default();
        let made = extraction.imports.iter().zip(targets).zip(module_names);
        for (((import, files), module_name), scope) in made.zip(scopes) {
            let binds_here = !import.is_reexport;
            match (&import.imported, &import.alias) {
                (Imported::Name(name), _) => read.take(import, name, files, scope),
                (Imported::All, None) => {
                    if scope.is_none() {
                        read.bindings
                            .everything
                            .extend(files.module.iter().cloned());
                    }
                }
                (Imported::Module, alias) => {
                    if let Some(name) = alias.as_ref().or(module_name.as_ref()) {
                        let reach = read.bindings.reach(&files.module);
                        read.reach(scope, name, Some(reach));
                    }
                }
                (Imported::All, Some(alias)) if binds_here => {
                    let reach = read.bindings.reach(&files.module);
                    read.reach(scope, alias, Some(reach));
                }
                // A default import binds a definition of its module, or something outside.
                (Imported::Default, Some(alias)) if binds_here => {
                    let reach = files.module.is_empty().then_some(Reach::Outside);
                    read.reach(scope, alias, reach);
                }
                (Imported::All | Imported::Default, _) => {}
            }
        }
        let scopes = Scopes::of(&extraction.symbols);
        let Read {
            mut bindings,
            bound,
            reaching,
        } = read;
        bindings.lookups = extraction
            .refs
            .iter()
            .zip(&extraction.qualifiers)
            .zip(&pairs)
            .map(|((reference, qualifier), pair)| {
                if reference.kind == RefKind::Import {
                    return Lookup::Import(pair.map(|(_, taken)| taken));
                }
                match qualifier {
                    Qualifier::None => scopes.bare(reference, &bound),
                    Qualifier::Name(qualifier) => scopes.member(reference, qualifier, &reaching),
                    Qualifier::Other => Lookup::ByName,
                }
            })
            .collect();
        bindings
    }

    /// The import at `position` of the imports that take a name.
    pub(crate) fn taken(&self, position: usize) -> Option<&Taken> {
        self.taken.get(position)
    }

    /// The positions of the file's top-level imports that pass `name` on.
    pub(crate) fn exports(&self, name: &Identifier) -> &[usize] {
        self.exports.get(name).map_or(&[], Vec::as_slice)
    }

    /// The files whose every name the file's top-level `*` imports pass on.
    pub(crate) fn everything(&self) -> &BTreeSet<RepoPath> {
        &self.everything
    }

    /// The files of the module at `position` among those the file's imports reach.
    pub(crate) fn module(&self, position: usize) -> &[RepoPath] {
        self.modules.get(position).map_or(&[], Vec::as_slice)
    }

    /// Where the definition of the reference at `position` of the file's references is looked
    /// for first; `None` past the last.
    pub(crate) fn lookup(&self, position: usize) -> Option<&Lookup> {
        self.lookups.get(position)
    }

    /// What a qualifier reaches through an import of the whole module at `files`: that module, or
    /// something outside the repository when it has no file here.
    fn reach(&mut self, files: &[RepoPath]) -> Reach {
        if files.is_empty() {
            return Reach::Outside;
        }
        Reach::Module(self.add_module(files.to_vec()))
    }

    /// Adds the module at `files`, giving its position among the modules the file reaches.
    fn add_module(&mut self, files: Vec<RepoPath>) -> usize {
        self.modules.push(files);
        self.modules.len() - 1
    }
}

/// A file's bindings while its imports are read, with the names they bind there.
#[derive(Default)]
struct Read<'e> {
    /// The bindings so far.
    bindings: Bindings,
    /// The imports binding each bare name.
    bound: Bound<'e>,
    /// What the imports binding each name reach when it is written as a qualifier.
    reaching: Reaching<'e>,
}

impl<'e> Read<'e> {
    /// Reads `import`, which takes `name`, made in `scope` and naming `files`. Made at the top of
    /// the file, it passes the name on; unless it is a re-export, it binds the name, under its
    /// alias if it has one, and what it reaches as a qualifier: the submodule of that name, else
    /// the definition its module makes, or something outside when the module is.
    fn take(
        &mut self,
        import: &'e Import,
        name: &'e Identifier,
        files: &Targets,
        scope: Option<usize>,
    ) {
        let position = self.bindings.taken.len();
        self.bindings.taken.push(Taken {
            name: name.clone(),
            files: files.module.clone(),
        });
        let bound_as = import.alias.as_ref().unwrap_or(name);
        if scope.is_none() {
            let passed_on = self.bindings.exports.entry(bound_as.clone()).or_default();
            passed_on.push(position);
        }
        if import.is_reexport {
            return;
        }
        self.bound
            .entry((scope, bound_as))
            .or_default()
            .push(position);
        let reach = match &files.submodule {
            Some(submodule) => Some(Reach::Submodule {
                taken: position,
                module: self.bindings.add_module(vec![submodule.clone()]),
            }),
            None if files.module.is_empty() => Some(Reach::Outside),
            None => None,
        };
        self.reach(scope, bound_as, reach);
    }

    /// Notes that `name`, written as a qualifier in `scope`, reaches `reach` through one more
    /// import, `None` being a definition of the repository.
    fn reach(&mut self, scope: Option<usize>, name: &'e Identifier, reach: Option<Reach>) {
        self.reaching.entry((scope, name)).or_default().push(reach);
    }
}

/// For each reference of `extraction`, by position, the import it is the own reference of: the
/// imports that take a name and the references of kind `import` come in the same order, one for
/// one, and a pair whose name or line disagree is left unpaired.
fn pair_own_references(extraction: &Extraction) -> Vec<Option<(usize, usize)>> {
    let mut takers = extraction
        .imports
        .iter()
        .filter_map(|import| match &import.imported {
            Imported::Name(name) => Some((name, import.line)),
            Imported::Module | Imported::All | Imported::Default => None,
        })
        .enumerate();
    extraction
        .refs
        .iter()
        .enumerate()
        .map(|(position, reference)| {
            if reference.kind != RefKind::Import {
                return None;
            }
            let (taken, (name, line)) = takers.next()?;
            (*name == reference.dst_name && line == reference.line).then_some((position, taken))
        })
        .collect()
}

/// The scope each import of `extraction` is made in, in order: the definition its own reference
/// sits in when `pairs` pairs it with one, else the innermost definition whose lines hold its line
/// after their first; `None` at the top of the file.
fn import_scopes(extraction: &Extraction, pairs: &[Option<(usize, usize)>]) -> Vec<Option<usize>> {
    let own_reference: BTreeMap<usize, usize> = pairs
        .iter()
        .flatten()
        .map(|(reference, taken)| (*taken, *reference))
        .collect();
    let mut takers = 0;
    extraction
        .imports
        .iter()
        .map(|import| {
            let own = match import.imported {
                Imported::Name(_) => {
                    takers += 1;
                    own_reference.get(&(takers - 1))
                }
                Imported::Module | Imported::All | Imported::Default => None,
            };
            match own {
                Some(reference) => extraction.refs.get(*reference).and_then(|own| own.symbol),
                None => scope_at_line(&extraction.symbols, import.line),
            }
        })
        .collect()
}

/// The innermost of `symbols` whose lines hold `line` after their first: where an import that
/// has no reference of its own is made.
fn scope_at_line(symbols: &[SymbolRecord], line: u32) -> Option<usize> {
    symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.start_line < line && line <= symbol.end_line)
        .map(|(position, _)| position)
        .next_back()
}

/// A file's definitions, as a lookup walks the scopes around a use.
struct Scopes<'e> {
    /// The definitions.
    symbols: &'e [SymbolRecord],
    /// Each name made directly in a scope, with that scope: a definition with no qualifier, whose
    /// qualified name is its parent's joined to its name, or its name at the top.
    plain: BTreeSet<(Option<usize>, &'e str)>,
}

impl<'e> Scopes<'e> {
    /// The scopes of `symbols`.
    fn of(symbols: &'e [SymbolRecord]) -> Self {
        let plain = symbols
            .iter()
            .filter(
                |symbol| match symbol.parent.and_then(|position| symbols.get(position)) {
                    None => symbol.qual_name == symbol.name,
                    Some(parent) => {
                        symbol.qual_name.as_str()
                            == format!("{}.{}", parent.qual_name.as_str(), symbol.name.as_str())
                    }
                },
            )
            .map(|symbol| (symbol.parent, symbol.name.as_str()))
            .collect();
        Self { symbols, plain }
    }

    /// Where the bare name `reference` uses is looked for: through the imports binding it in the
    /// first scope around it that makes the name or has such imports, else by its name.
    fn bare(&self, reference: &RefRecord, bound: &Bound<'_>) -> Lookup {
        match self.first(reference, &reference.dst_name, bound) {
            Some(imports) => Lookup::Bound(imports.clone()),
            None => Lookup::ByName,
        }
    }

    /// Where the name `reference` uses after `qualifier` is looked for: in what the imports
    /// binding the qualifier reach, in the first scope around it that makes the qualifier's name
    /// or has such imports, else by its own name. A qualifier one of them binds to a definition
    /// of the repository leaves the name to its own name: that definition's members could make it.
    fn member(
        &self,
        reference: &RefRecord,
        qualifier: &Identifier,
        reaching: &Reaching<'_>,
    ) -> Lookup {
        self.first(reference, qualifier, reaching)
            .and_then(|reaches| reaches.iter().copied().collect::<Option<Vec<Reach>>>())
            .map_or(Lookup::ByName, Lookup::Member)
    }

    /// The entry of `imports` for `name` in the first scope around `reference`, from the
    /// definition it sits in outwards, that makes the name or has an entry for it; `None` when
    /// that scope makes it, or no scope does either.
    fn first<'m, T>(
        &self,
        reference: &RefRecord,
        name: &'m Identifier,
        imports: &'m BTreeMap<(Option<usize>, &Identifier), T>,
    ) -> Option<&'m T> {
        for scope in scopes_around(reference.symbol, self.symbols) {
            if self.plain.contains(&(scope, name.as_str())) {
                return None;
            }
            if let Some(entry) = imports.get(&(scope, name)) {
                return Some(entry);
            }
        }
        None
    }
}

/// The scopes a use sitting in the definition at `at` sees, innermost first: that definition,
/// then each function, method, test or module around it, then the top of the file (`None`).
fn scopes_around(at: Option<usize>, symbols: &[SymbolRecord]) -> Vec<Option<usize>> {
    let mut scopes = vec![at];
    let mut current = at.and_then(|position| symbols.get(position)?.parent);
    while let Some(position) = current {
        let Some(symbol) = symbols.get(position) else {
            break;
        };
        if is_scope(symbol.kind) {
            scopes.push(Some(position));
        }
        current = symbol.parent;
    }
    if at.is_some() {
        scopes.push(None);
    }
    scopes
}

/// Whether the body of a definition of `kind` is a scope its nested functions see bare names
/// of. A class's or an interface's is not: its methods reach its names through `self` or `this`.
fn is_scope(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Test | SymbolKind::Module
    )
}

#[cfg(test)]
mod tests {
    use cortex_core::{IdentifierError, ShortHash};

    use super::*;
    use crate::extract::LeftOut;
    use crate::test_support::{import, import_name, path};

    /// A definition of `kind` named `qual_name`'s last part, in the definition at `parent`, over
    /// the lines from `first` to `last`.
    fn symbol(
        kind: SymbolKind,
        qual_name: &str,
        parent: Option<usize>,
        (first, last): (u32, u32),
    ) -> Result<SymbolRecord, IdentifierError> {
        let short = qual_name.rsplit('.').next().unwrap_or(qual_name);
        Ok(SymbolRecord {
            parent,
            kind,
            name: Identifier::new(short)?,
            qual_name: Identifier::new(qual_name)?,
            start_byte: 0,
            end_byte: 0,
            start_line: first,
            end_line: last,
            sig_hash: ShortHash::of(b""),
            body_hash: ShortHash::of(b""),
        })
    }

    /// A use of `name` on `line`, in the definition at `symbol`.
    fn use_of(
        symbol: Option<usize>,
        name: &str,
        kind: RefKind,
        line: u32,
    ) -> Result<RefRecord, IdentifierError> {
        Ok(RefRecord {
            symbol,
            dst_name: Identifier::new(name)?,
            kind,
            line,
        })
    }

    /// An extraction of `symbols`, bare `refs` and `imports`.
    fn extraction(
        symbols: Vec<SymbolRecord>,
        refs: Vec<RefRecord>,
        imports: Vec<Import>,
    ) -> Extraction {
        Extraction {
            symbols,
            qualifiers: vec![Qualifier::None; refs.len()],
            refs,
            imports,
            left_out: LeftOut::default(),
        }
    }

    /// The bindings of `read`, whose imports name `targets` and give no module a name.
    fn bind(read: &Extraction, targets: &[Targets]) -> Bindings {
        Bindings::of(read, targets, &vec![None; targets.len()])
    }

    /// The files `module` names.
    fn files(module: &[&str]) -> Targets {
        Targets {
            module: module.iter().copied().map(path).collect(),
            submodule: None,
        }
    }

    /// Import references pair with the imports that take a name, in order: the first with
    /// `a` on line 1; the second, `c` on line 2 against an import of `b` there, stays unpaired.
    /// A bare `a` on line 3 is bound by the first import, at the top of the file.
    #[test]
    fn own_references_pair_with_their_imports_in_order() -> Result<(), IdentifierError> {
        let mut b = import_name("./b", "b")?;
        b.line = 2;
        let read = extraction(
            Vec::new(),
            vec![
                use_of(None, "a", RefKind::Import, 1)?,
                use_of(None, "c", RefKind::Import, 2)?,
                use_of(None, "a", RefKind::Call, 3)?,
            ],
            vec![import_name("./a", "a")?, b],
        );
        let bindings = bind(&read, &[files(&["a.ts"]), files(&["b.ts"])]);
        let lookups: Vec<Option<&Lookup>> = (0..4).map(|at| bindings.lookup(at)).collect();
        assert_eq!(
            lookups,
            [
                Some(&Lookup::Import(Some(0))),
                Some(&Lookup::Import(None)),
                Some(&Lookup::Bound(vec![0])),
                None,
            ]
        );
        Ok(())
    }

    /// An import of every name binds no bare name, and passes names on only when made at the top:
    /// on line 3, inside `mod tests` (lines 2 to 6), it passes nothing on; on line 2, the module's
    /// first, it counts as made at the top. Uses of `f` in the test function and at the top go by
    /// their name either way.
    #[test]
    fn imports_of_every_name_only_pass_names_on() -> Result<(), IdentifierError> {
        let mut inside = import("crate::x", Imported::All)?;
        inside.line = 3;
        let mut top = import("crate::y", Imported::All)?;
        top.line = 2;
        let read = extraction(
            vec![
                symbol(SymbolKind::Module, "tests", None, (2, 6))?,
                symbol(SymbolKind::Function, "tests.t", Some(0), (4, 5))?,
            ],
            vec![
                use_of(Some(1), "f", RefKind::Call, 5)?,
                use_of(None, "f", RefKind::Call, 8)?,
            ],
            vec![top, inside],
        );
        let bindings = bind(&read, &[files(&["src/y.rs"]), files(&["src/x.rs"])]);
        assert_eq!(bindings.lookup(0), Some(&Lookup::ByName));
        assert_eq!(bindings.lookup(1), Some(&Lookup::ByName));
        let everything: Vec<&RepoPath> = bindings.everything().iter().collect();
        assert_eq!(everything, [&path("src/y.rs")]);
        Ok(())
    }

    /// A re-export of `X` (line 1) passes it on to importers of the file but binds nothing in the
    /// file: a bare `X` on line 2 goes by its name, while the re-export's own reference stands
    /// for it.
    #[test]
    fn a_reexport_binds_nothing_in_its_file() -> Result<(), IdentifierError> {
        let mut passed = import_name("./types", "X")?;
        passed.is_reexport = true;
        let read = extraction(
            Vec::new(),
            vec![
                use_of(None, "X", RefKind::Import, 1)?,
                use_of(None, "X", RefKind::TypeRef, 2)?,
            ],
            vec![passed],
        );
        let x = Identifier::new("X")?;
        let bindings = bind(&read, &[files(&["src/types.ts"])]);
        assert_eq!(bindings.lookup(0), Some(&Lookup::Import(Some(0))));
        assert_eq!(bindings.lookup(1), Some(&Lookup::ByName));
        assert_eq!(bindings.exports(&x), [0]);
        Ok(())
    }

    /// An import that takes a name but could not be paired with its own reference takes the scope
    /// of its line: `helper`, imported on line 3 inside `run` (lines 2 to 5), binds a use on line
    /// 4 there and none on line 7, at the top, and passes nothing on.
    #[test]
    fn an_unpaired_import_takes_the_scope_of_its_line() -> Result<(), IdentifierError> {
        let mut local = import_name("./helpers", "helper")?;
        local.line = 3;
        let read = extraction(
            vec![symbol(SymbolKind::Function, "run", None, (2, 5))?],
            vec![
                use_of(Some(0), "helper", RefKind::Call, 4)?,
                use_of(None, "helper", RefKind::Call, 7)?,
            ],
            vec![local],
        );
        let helper = Identifier::new("helper")?;
        let bindings = bind(&read, &[files(&["src/helpers.ts"])]);
        assert_eq!(bindings.lookup(0), Some(&Lookup::Bound(vec![0])));
        assert_eq!(bindings.lookup(1), Some(&Lookup::ByName));
        assert_eq!(bindings.exports(&helper), &[] as &[usize]);
        Ok(())
    }
}
