//! The graph's structural edges between files: from each file to each file its imports name.
//!
//! An import edge points one way, from the importer. It is as strong as a reference tied by its
//! import path, and its count is how many of the importer's imports name the target.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{EdgeType, Language, RepoPath, Resolution};
use cortex_store::{EdgeRecord, NodeId, StoreError, Writer};

use crate::imports::Project;
use crate::specifiers::{Import, Imported};

/// How many importers have their edges written in one transaction: a commit costs little next to
/// this many files' edges, and the transaction stays short.
const IMPORTERS_PER_TRANSACTION: usize = 4096;

/// The files one file's imports name, each with how many of its imports name it.
pub(crate) type ImportCounts = BTreeMap<RepoPath, u32>;

/// The files `imports`, written in `language` in the file at `importer`, name, each with how
/// many of the imports name it. Alike imports are resolved once, so the work grows with a file's
/// distinct imports however often it repeats one; an import that names a file twice counts once,
/// and the importer is left out.
pub(crate) fn imported_files(
    project: &Project,
    importer: &RepoPath,
    language: Language,
    imports: &[Import],
) -> ImportCounts {
    let mut counts = ImportCounts::new();
    for (import, times) in alike(imports) {
        let targets = project.targets(importer, language, import);
        let named: BTreeSet<&RepoPath> = targets.files().filter(|file| *file != importer).collect();
        for file in named {
            let count = counts.entry(file.clone()).or_insert(0);
            *count = count.saturating_add(times);
        }
    }
    counts
}

/// Each kind of import in `imports` once, with how many times it appears. Imports are alike when
/// they take the same name, or no name, from the same module, which is all a resolver reads of
/// them, so alike imports name the same files.
fn alike(imports: &[Import]) -> Vec<(&Import, u32)> {
    let mut kinds: BTreeMap<(&str, Option<&str>), (&Import, u32)> = BTreeMap::new();
    for import in imports {
        let taken = match &import.imported {
            Imported::Name(name) => Some(name.as_str()),
            Imported::Module | Imported::All | Imported::Default => None,
        };
        let kind = kinds
            .entry((import.specifier.as_str(), taken))
            .or_insert((import, 0));
        kind.1 = kind.1.saturating_add(1);
    }
    kinds.into_values().collect()
}

/// Replaces the structural edges from each file of `importers` with edges to the files its
/// imports name. A file or target without a node gets no edge.
///
/// # Errors
///
/// Store errors; the transactions committed before stay written.
pub(crate) fn write_import_edges(
    writer: &mut Writer,
    importers: &[(RepoPath, ImportCounts)],
) -> Result<(), StoreError> {
    let nodes: BTreeMap<RepoPath, NodeId> = writer.queries().file_nodes()?.into_iter().collect();
    for batch in importers.chunks(IMPORTERS_PER_TRANSACTION) {
        let mut transaction = writer.transaction()?;
        for (importer, counts) in batch {
            let Some(src) = nodes.get(importer) else {
                continue;
            };
            let edges: Vec<EdgeRecord> = counts
                .iter()
                .filter_map(|(target, count)| {
                    Some(EdgeRecord {
                        dst: *nodes.get(target)?,
                        weight: Resolution::ImportPath.confidence(),
                        count: *count,
                        // No task observed it: the code itself says so.
                        last_seq: 0,
                        is_directed: true,
                    })
                })
                .collect();
            transaction.replace_edges(*src, EdgeType::Structural, &edges)?;
        }
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use cortex_core::{Identifier, IdentifierError};

    use super::*;
    use crate::test_support::{import, import_name, path};

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
        let counts = imported_files(
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
        let counts = imported_files(
            &project,
            &path("pkg/a.py"),
            Language::Python,
            &[import_name(".", "__init__")?],
        );
        assert_eq!(counts, ImportCounts::from([(path("pkg/__init__.py"), 1)]));
        Ok(())
    }
}
