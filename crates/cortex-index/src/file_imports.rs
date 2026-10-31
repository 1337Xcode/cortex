//! What one file's imports name, each kind of import resolved once: the files each import names,
//! counted for the file's import edges and kept, import by import, for tying its references.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Language, RepoPath};

use crate::bindings::Bindings;
use crate::file_edges::ImportCounts;
use crate::imports::{Project, Targets};
use crate::specifiers::{Import, Imported};

/// What one file's imports name.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FileImports {
    /// The files they name, each with how many of the imports name it.
    pub(crate) counts: ImportCounts,
    /// What they bring into the file and pass on.
    pub(crate) bindings: Bindings,
}

/// The files `imports`, written in `language` in the file at `importer`, name: each with how
/// many of the imports name it, and each import's in the order of `imports`. Alike imports are
/// resolved once, so the work of resolving grows with a file's distinct imports however often it
/// repeats one. An import counts once for each file it names, even one it names twice, and the
/// importer is left out of the counts.
pub(crate) fn resolve_imports(
    project: &Project,
    importer: &RepoPath,
    language: Language,
    imports: &[Import],
) -> (ImportCounts, Vec<Targets>) {
    let mut resolved: BTreeMap<Kind<'_>, Targets> = BTreeMap::new();
    let targets: Vec<Targets> = imports
        .iter()
        .map(|import| {
            resolved
                .entry(kind_of(import))
                .or_insert_with(|| project.targets(importer, language, import))
                .clone()
        })
        .collect();
    let mut counts = ImportCounts::new();
    for (import, times) in alike(imports) {
        let Some(found) = resolved.get(&kind_of(import)) else {
            continue;
        };
        let named: BTreeSet<&RepoPath> = found.files().filter(|file| *file != importer).collect();
        for file in named {
            let count = counts.entry(file.clone()).or_insert(0);
            *count = count.saturating_add(times);
        }
    }
    (counts, targets)
}

/// What a resolver reads of an import: its module, and the name it takes, if any.
type Kind<'a> = (&'a str, Option<&'a str>);

/// What a resolver reads of `import`.
fn kind_of(import: &Import) -> Kind<'_> {
    let taken = match &import.imported {
        Imported::Name(name) => Some(name.as_str()),
        Imported::Module | Imported::All | Imported::Default => None,
    };
    (import.specifier.as_str(), taken)
}

/// Each kind of import in `imports` once, with how many times it appears. Imports are alike when
/// they take the same name, or no name, from the same module, which is all a resolver reads of
/// them, so alike imports name the same files.
fn alike(imports: &[Import]) -> Vec<(&Import, u32)> {
    let mut kinds: BTreeMap<Kind<'_>, (&Import, u32)> = BTreeMap::new();
    for import in imports {
        let kind = kinds.entry(kind_of(import)).or_insert((import, 0));
        kind.1 = kind.1.saturating_add(1);
    }
    kinds.into_values().collect()
}

#[cfg(test)]
mod tests {
    use cortex_core::{Identifier, IdentifierError};

    use super::*;
    use crate::test_support::{import, import_name, named, named_by, path};

    /// Each import adds one to each file it names: three imports from `./a`, two of them alike,
    /// count three, an import of the importer itself is left out, and a package names nothing.
    #[test]
    fn each_import_adds_one_to_each_file_it_names() -> Result<(), IdentifierError> {
        let project = Project::load(["src/index.ts", "src/a.ts", "src/b.ts"].map(path), &|_| {
            None
        });
        let imports = [
            import_name("./a", "a")?,
            import_name("./b", "b")?,
            import_name("./a", "x")?,
            import_name("./a", "a")?,
            import_name("./index", "y")?,
            import("react", Imported::Module)?,
        ];
        let (counts, _) = resolve_imports(
            &project,
            &path("src/index.ts"),
            Language::TypeScript,
            &imports,
        );
        let expected = ImportCounts::from([(path("src/a.ts"), 3), (path("src/b.ts"), 1)]);
        assert_eq!(counts, expected);
        Ok(())
    }

    /// Alike imports are kept once with how often they appear: the same name, or no name, taken
    /// from the same module. Another name or another module is another kind; the first of each
    /// kind stands for it, and kinds come in order of module, then name.
    #[test]
    fn alike_imports_are_kept_once_with_their_count() -> Result<(), IdentifierError> {
        let imports = [
            import_name("./a", "a")?,
            import("./a", Imported::Module)?,
            import_name("./a", "a")?,
            import("./a", Imported::All)?,
            import_name("./a", "x")?,
            import_name("./b", "a")?,
        ];
        let kinds: Vec<(&str, &Imported, u32)> = alike(&imports)
            .into_iter()
            .map(|(import, times)| (import.specifier.as_str(), &import.imported, times))
            .collect();
        let a = Imported::Name(Identifier::new("a")?);
        let x = Imported::Name(Identifier::new("x")?);
        assert_eq!(
            kinds,
            [
                ("./a", &Imported::Module, 2),
                ("./a", &a, 2),
                ("./a", &x, 1),
                ("./b", &a, 1),
            ]
        );
        Ok(())
    }

    /// An import that names one file twice counts once: Python's `from . import __init__`, from
    /// `pkg/a.py`, names `pkg/__init__.py` as the module and as the submodule.
    #[test]
    fn an_import_that_names_a_file_twice_counts_once() -> Result<(), IdentifierError> {
        let project = Project::load(["pkg/__init__.py", "pkg/a.py"].map(path), &|_| None);
        let (counts, _) = resolve_imports(
            &project,
            &path("pkg/a.py"),
            Language::Python,
            &[import_name(".", "__init__")?],
        );
        assert_eq!(counts, ImportCounts::from([(path("pkg/__init__.py"), 1)]));
        Ok(())
    }

    /// Every import gets what its kind names, in the order of the imports, alike ones included:
    /// two imports of `Order` from `./models`, one renamed, name `models.ts` each, and a package
    /// names nothing.
    #[test]
    fn each_import_gets_the_files_its_kind_names() -> Result<(), IdentifierError> {
        let project = Project::load(["src/app.ts", "src/models.ts"].map(path), &|_| None);
        let mut renamed = import_name("./models", "Order")?;
        renamed.alias = Some(Identifier::new("O")?);
        let imports = [
            import_name("./models", "Order")?,
            renamed,
            import("react", Imported::Module)?,
        ];
        let (_, targets) = resolve_imports(
            &project,
            &path("src/app.ts"),
            Language::TypeScript,
            &imports,
        );
        let found: Vec<_> = targets.iter().map(named_by).collect();
        assert_eq!(
            found,
            [
                named(&["src/models.ts"], None),
                named(&["src/models.ts"], None),
                named(&[], None),
            ]
        );
        Ok(())
    }
}
