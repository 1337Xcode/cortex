//! Import resolution: the files an import names. Each language family whose imports name files
//! has a resolver of its own in this folder; an import in any other language names none.
//!
//! A resolver reads the project files that decide how its imports resolve, a tsconfig.json for one,
//! once, when the project is loaded, and then answers from the repository's paths alone. An import
//! of something outside the repository (a package, the standard library) names no file, and an
//! import costs a bounded number of lookups whatever the repository holds.
//!
//! Adding a family means adding its file and one line in [`LOADERS`].

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Language, RepoPath};

use crate::specifiers::Import;

mod ecmascript;
mod jsonc;

/// Reads one project file as text; `None` when it cannot be read.
pub(crate) type ReadText<'a> = &'a dyn Fn(&RepoPath) -> Option<String>;

/// Loads one family's resolver from the repository's files and the project files it reads.
type Loader = fn(&Files, ReadText<'_>) -> Box<dyn Resolver>;

/// Every language family whose imports name files, by its loader.
const LOADERS: &[Loader] = &[ecmascript::load];

/// Turns the imports of one language family into the files they name.
trait Resolver: Send + Sync {
    /// Whether this resolver reads imports written in `language`.
    fn reads(&self, language: Language) -> bool;

    /// The files `import`, written in the file at `importer`, names. Only the import's module and,
    /// when it takes one name, that name are read: imports alike in those name the same files.
    fn targets(&self, files: &Files, importer: &RepoPath, import: &Import) -> Targets;
}

/// The files one import names.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Targets {
    /// The files of the module the import's specifier names, where a name it takes is defined: one
    /// file, or every file of a Go package. Empty when the module is outside the repository.
    pub(crate) module: Vec<RepoPath>,
    /// The file of the name the import takes, when that name is a module of its own: Python's
    /// `from shop import orders` names `shop/orders.py` this way.
    pub(crate) submodule: Option<RepoPath>,
}

impl Targets {
    /// Every file named: the module's, then the submodule.
    pub(crate) fn files(&self) -> impl Iterator<Item = &RepoPath> {
        self.module.iter().chain(&self.submodule)
    }
}

/// The repository's files, as resolution looks them up.
pub(crate) struct Files {
    /// Every path.
    paths: BTreeSet<RepoPath>,
    /// The paths with each file name, in path order.
    by_name: BTreeMap<String, Vec<RepoPath>>,
}

impl Files {
    /// The files at `paths`.
    pub(crate) fn new(paths: impl IntoIterator<Item = RepoPath>) -> Self {
        let paths: BTreeSet<RepoPath> = paths.into_iter().collect();
        let mut by_name: BTreeMap<String, Vec<RepoPath>> = BTreeMap::new();
        for path in &paths {
            by_name
                .entry(path.file_name().to_owned())
                .or_default()
                .push(path.clone());
        }
        Self { paths, by_name }
    }

    /// Whether a file is at `path`.
    fn contains(&self, path: &RepoPath) -> bool {
        self.paths.contains(path)
    }

    /// The file at the path `text` spells, if there is one.
    fn at(&self, text: &str) -> Option<RepoPath> {
        RepoPath::new(text).ok().filter(|path| self.contains(path))
    }

    /// The files named `name`, in path order.
    fn named(&self, name: &str) -> &[RepoPath] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }
}

/// What import resolution knows about a repository: its files and every family's resolver.
pub(crate) struct Project {
    /// The files.
    files: Files,
    /// One resolver per language family, in [`LOADERS`] order.
    resolvers: Vec<Box<dyn Resolver>>,
}

impl Project {
    /// The project of the repository whose files are at `paths`, reading its project files with
    /// `read`.
    pub(crate) fn load(paths: impl IntoIterator<Item = RepoPath>, read: ReadText<'_>) -> Self {
        let files = Files::new(paths);
        let resolvers = LOADERS.iter().map(|load| load(&files, read)).collect();
        Self { files, resolvers }
    }

    /// The files `import`, written in `language` in the file at `importer`, names; none when no
    /// resolver reads the language.
    pub(crate) fn targets(
        &self,
        importer: &RepoPath,
        language: Language,
        import: &Import,
    ) -> Targets {
        self.resolvers
            .iter()
            .find(|resolver| resolver.reads(language))
            .map_or_else(Targets::default, |resolver| {
                resolver.targets(&self.files, importer, import)
            })
    }
}

/// The path `relative`, written with `/`, reaches from `folder`, its `.` parts skipped and each
/// `..` taking one folder off; `None` when it is rooted, climbs above the root or names nothing a
/// path can hold.
fn joined(folder: &RepoPath, relative: &str) -> Option<RepoPath> {
    if relative.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = folder
        .as_str()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    RepoPath::new(&parts.join("/")).ok()
}

/// The folders from `path`'s own folder up to the root, nearest first.
fn folders_above(path: &RepoPath) -> impl Iterator<Item = RepoPath> {
    std::iter::successors(path.parent(), RepoPath::parent)
}

#[cfg(test)]
mod tests {
    use cortex_core::IdentifierError;

    use super::*;
    use crate::specifiers::Imported;
    use crate::test_support::{import, path};

    /// Relative paths are joined part by part; `..` takes a folder off and may not climb above
    /// the root, and a rooted path is not relative.
    #[test]
    fn relative_paths_join_part_by_part() {
        let cases = [
            ("src/shop", "./orders", Some("src/shop/orders")),
            ("src/shop", "../util/money", Some("src/util/money")),
            ("src/shop", "../../lib", Some("lib")),
            ("src/shop", "../../../escape", None),
            ("src/shop", ".", Some("src/shop")),
            ("", "./a/./b", Some("a/b")),
            ("src", "/etc/passwd", None),
        ];
        for (folder, relative, expected) in cases {
            let joined = joined(&path(folder), relative);
            assert_eq!(
                joined.as_ref().map(RepoPath::as_str),
                expected,
                "{folder} {relative}"
            );
        }
    }

    /// A lookup by name holds every file of that name, in path order.
    #[test]
    fn files_are_found_by_name() {
        let files = Files::new(
            [
                "src/shop/orders.py",
                "src/myshop/orders.py",
                "shop/orders.py",
                "src/shop/cart.py",
            ]
            .map(path),
        );
        let named: Vec<&str> = files
            .named("orders.py")
            .iter()
            .map(RepoPath::as_str)
            .collect();
        assert_eq!(
            named,
            [
                "shop/orders.py",
                "src/myshop/orders.py",
                "src/shop/orders.py"
            ]
        );
        assert_eq!(files.named("none.py"), &[] as &[RepoPath]);
    }

    /// An import goes to the resolver of its file's language: the same `./b` names `b.ts` beside
    /// a TypeScript importer and nothing from a C file, whose imports no resolver reads.
    #[test]
    fn imports_go_to_the_resolver_of_their_language() -> Result<(), IdentifierError> {
        let project = Project::load(["src/a.ts", "src/b.ts", "src/c.c"].map(path), &|_| None);
        let module = import("./b", Imported::Module)?;
        let typescript = project.targets(&path("src/a.ts"), Language::TypeScript, &module);
        assert_eq!(typescript.module, [path("src/b.ts")]);
        let c = project.targets(&path("src/c.c"), Language::C, &module);
        assert_eq!(c, Targets::default());
        Ok(())
    }
}
