//! Python imports. A relative import (`from .models import Order`) starts from the importer's
//! package, one folder up for each dot after the first, and climbs only through packages (folders
//! with an `__init__.py`): Python refuses one from a module outside a package or above the top
//! package. `from . import x` names the package's `__init__.py` and the submodule `x`. An
//! absolute import is looked for under the folders Python
//! would search, in order: the importer's base folder (the nearest folder above it that is not a
//! package, which running it as a script or as a test puts on the path), the repository root,
//! then the one source root that holds the module: a folder with pyproject.toml, setup.py or
//! setup.cfg, or the `src` folder beside them. A folder that is itself a package is never searched
//! as a root, so `import json` in a package never binds to a `json.py` beside the importer, and a
//! module found nowhere else names nothing.
//!
//! A module is its package's `__init__.py` before a file of its name, as Python looks for them,
//! and a plain module file holds no modules. `from X import n` also names the submodule `n` when X
//! is a package that holds one.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Language, RepoPath};

use super::{Files, ReadText, Resolver, Targets, folders_above};
use crate::specifiers::{Import, Imported};

/// The file that makes a folder a package.
const PACKAGE_FILE: &str = "__init__.py";

/// The files that make the folder holding them, and the `src` folder beside them, source roots.
const PROJECT_FILES: [&str; 3] = ["pyproject.toml", "setup.py", "setup.cfg"];

/// How many source roots may hold one top-level name before the modules under it are left
/// unresolved: every one of them costs lookups on each import of the name, and real repositories
/// have a handful of source roots.
const MAX_SOURCE_ROOTS_PER_NAME: usize = 64;

/// Resolves Python imports from the repository's paths.
struct Python {
    /// The source roots that hold each top-level name, a module or a folder, in path order.
    source_roots: BTreeMap<String, Vec<RepoPath>>,
}

/// Finds the source roots and the top-level names each holds.
pub(super) fn load(files: &Files, _read: ReadText<'_>) -> Box<dyn Resolver> {
    let roots: BTreeSet<RepoPath> = PROJECT_FILES
        .iter()
        .flat_map(|name| files.named(name))
        .filter_map(RepoPath::parent)
        .flat_map(|folder| [folder.join("src").ok(), Some(folder)])
        .flatten()
        .filter(|root| package_file(files, root).is_none())
        .collect();
    let mut holding: BTreeMap<String, BTreeSet<RepoPath>> = BTreeMap::new();
    for path in files.all() {
        for root in folders_above(path).filter(|folder| roots.contains(folder)) {
            if let Some(name) = top_level_name(path, &root) {
                holding.entry(name).or_default().insert(root);
            }
        }
    }
    let source_roots = holding
        .into_iter()
        .map(|(name, roots)| (name, roots.into_iter().collect()))
        .collect();
    Box::new(Python { source_roots })
}

impl Resolver for Python {
    fn reads(&self, language: Language) -> bool {
        language == Language::Python
    }

    fn targets(&self, files: &Files, importer: &RepoPath, import: &Import) -> Targets {
        let specifier = import.specifier.as_str();
        let name = match &import.imported {
            Imported::Name(name) => Some(name.as_str()),
            Imported::Module | Imported::All | Imported::Default => None,
        };
        let dotted = specifier.trim_start_matches('.');
        let dots = specifier.len() - dotted.len();
        let module: Vec<&str> = dotted.split('.').filter(|part| !part.is_empty()).collect();
        let Some(above) = dots.checked_sub(1) else {
            return self.absolute(files, importer, &module, name);
        };
        let climbed: Vec<RepoPath> = folders_above(importer).take(above + 1).collect();
        let is_inside_packages = climbed.len() == above + 1
            && climbed
                .iter()
                .all(|folder| package_file(files, folder).is_some());
        match climbed.last().filter(|_| is_inside_packages) {
            Some(package) => located(files, package, &module, name),
            None => Targets::default(),
        }
    }
}

impl Python {
    /// What the absolute `module`, its parts in order, names from the file at `importer`, looked
    /// for under its base folder, the repository root, then the one source root that holds it.
    fn absolute(
        &self,
        files: &Files,
        importer: &RepoPath,
        module: &[&str],
        name: Option<&str>,
    ) -> Targets {
        let Some(top) = module.first() else {
            return Targets::default();
        };
        let base = folders_above(importer).find(|folder| package_file(files, folder).is_none());
        for root in [base, Some(RepoPath::root())].into_iter().flatten() {
            if package_file(files, &root).is_some() {
                continue;
            }
            let found = located(files, &root, module, name);
            if found.files().next().is_some() {
                return found;
            }
        }
        let candidates = self.source_roots.get(*top).map_or(&[][..], Vec::as_slice);
        if candidates.len() > MAX_SOURCE_ROOTS_PER_NAME {
            return Targets::default();
        }
        let mut holding = candidates
            .iter()
            .map(|root| located(files, root, module, name))
            .filter(|found| found.files().next().is_some());
        match (holding.next(), holding.next()) {
            (Some(only), None) => only,
            _ => Targets::default(),
        }
    }
}

/// What `module`, its parts in order, names under `root`: its package's `__init__.py`, else its
/// own file, and the file of the submodule `name` when the module is a package, with an
/// `__init__.py` or without one. With no parts the module is `root` itself, the package a relative
/// import's dots reach. A part before the last that is a plain module holds no modules, so the
/// module names nothing.
fn located(files: &Files, root: &RepoPath, module: &[&str], name: Option<&str>) -> Targets {
    let mut folder = root.clone();
    for (at, part) in module.iter().enumerate() {
        let Ok(inner) = folder.join(part) else {
            return Targets::default();
        };
        folder = inner;
        if at + 1 < module.len() && is_plain_module(files, &folder) {
            return Targets::default();
        }
    }
    let file = module_file(files, &folder);
    let holds_modules = file
        .as_ref()
        .is_none_or(|file| file.file_name() == PACKAGE_FILE);
    let submodule = name
        .filter(|_| holds_modules)
        .and_then(|name| folder.join(name).ok())
        .and_then(|inner| module_file(files, &inner));
    Targets {
        module: file.into_iter().collect(),
        submodule,
    }
}

/// The file of the module whose package would be `folder`: the package's `__init__.py`, else the
/// `.py` file of the folder's name beside it.
fn module_file(files: &Files, folder: &RepoPath) -> Option<RepoPath> {
    let plain = || {
        let name = folder.file_name();
        let beside = folder.parent()?;
        beside
            .join(&format!("{name}.py"))
            .ok()
            .filter(|file| files.contains(file))
    };
    package_file(files, folder).or_else(plain)
}

/// Whether the module whose package would be `folder` is a plain module file, not a package.
fn is_plain_module(files: &Files, folder: &RepoPath) -> bool {
    module_file(files, folder).is_some_and(|file| file.file_name() != PACKAGE_FILE)
}

/// The `__init__.py` of `folder`, when it has one.
fn package_file(files: &Files, folder: &RepoPath) -> Option<RepoPath> {
    folder
        .join(PACKAGE_FILE)
        .ok()
        .filter(|file| files.contains(file))
}

/// The top-level name `path` gives the source root `root` it is under: the folder below the root
/// it is in, or its own name without `.py` when it is directly in the root.
fn top_level_name(path: &RepoPath, root: &RepoPath) -> Option<String> {
    let inside = if root.is_root() {
        path.as_str()
    } else {
        path.as_str()
            .strip_prefix(root.as_str())?
            .strip_prefix('/')?
    };
    match inside.split_once('/') {
        Some((folder, _)) => Some(folder.to_owned()),
        None => inside
            .strip_suffix(".py")
            .filter(|stem| !stem.is_empty())
            .map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::test_support::{Named, import, import_name as from, named, named_by, path};

    /// What each import names when the file at `importer` makes it, in a repository of the files
    /// at `paths`.
    fn resolved(paths: &[&str], importer: &str, imports: &[Import]) -> Vec<Named> {
        let files = Files::new(paths.iter().map(|at| path(at)));
        let resolver = load(&files, &|_| None);
        imports
            .iter()
            .map(|import| named_by(&resolver.targets(&files, &path(importer), import)))
            .collect()
    }

    /// A relative import starts from the importer's package, one folder up for each dot after the
    /// first, and climbs only through packages: above the top package, as from a module outside
    /// any package, it names nothing. A package's module is its `__init__.py`, before a file of
    /// the package's name beside it, and a name it holds as a file is a submodule; a plain module
    /// file holds no submodule, even with a folder of its name beside it.
    #[test]
    fn relative_imports_climb_one_folder_per_dot() -> Result<(), Box<dyn Error>> {
        let paths = [
            "shop/__init__.py",
            "shop/orders.py",
            "shop/orders/place.py",
            "shop/both.py",
            "shop/both/__init__.py",
            "shop/models/__init__.py",
            "shop/models/order.py",
            "shop/api/__init__.py",
            "shop/api/views.py",
            "scripts/run.py",
            "scripts/helper.py",
        ];
        let imports = [
            from(".helpers", "x")?,
            from("..models", "Order")?,
            from("..models", "order")?,
            from("..", "orders")?,
            from("..orders", "place")?,
            from("..both", "x")?,
            from("...", "shop")?,
            from("....", "x")?,
        ];
        let expected = [
            named(&[], None),
            named(&["shop/models/__init__.py"], None),
            named(&["shop/models/__init__.py"], Some("shop/models/order.py")),
            named(&["shop/__init__.py"], Some("shop/orders.py")),
            named(&["shop/orders.py"], None),
            named(&["shop/both/__init__.py"], None),
            named(&[], None),
            named(&[], None),
        ];
        assert_eq!(resolved(&paths, "shop/api/views.py", &imports), expected);
        let outside = resolved(&paths, "shop/orders/place.py", &[from(".", "helper")?]);
        assert_eq!(outside, [named(&[], None)]);
        let script = resolved(&paths, "scripts/run.py", &[from(".", "helper")?]);
        assert_eq!(script, [named(&[], None)]);
        Ok(())
    }

    /// An absolute module is found under the importer's base folder, else the repository root,
    /// else the one source root that holds it; a package without an `__init__.py` is found by the
    /// submodule a name takes. A package is never a root, a plain module holds no modules, and a
    /// module found nowhere else names nothing, so `json` stays the standard library's and a
    /// dotted module that only some other folder holds stays unresolved.
    #[test]
    fn absolute_imports_look_in_the_roots_python_would() -> Result<(), Box<dyn Error>> {
        let paths = [
            "pyproject.toml",
            "src/mypkg/__init__.py",
            "src/mypkg/core.py",
            "tests/test_core.py",
            "scripts/run.py",
            "scripts/helper.py",
            "app/handlers/__init__.py",
            "app/handlers/json.py",
            "app/handlers/api.py",
            "utils/json.py",
            "vendor_a/pkg/mod.py",
            "vendor_b/pkg/mod.py",
            "libs/only/thing.py",
            "services/api/app/__init__.py",
            "services/api/app/models.py",
            "services/api/app/views.py",
            "services/worker/app/models.py",
            "nsroot/main.py",
            "nsroot/ns/sub.py",
            "plain.py",
            "plain/b.py",
        ];
        let cases: [(&str, Import, Named); 12] = [
            (
                "tests/test_core.py",
                import("plain.b", Imported::Module)?,
                named(&[], None),
            ),
            (
                "nsroot/main.py",
                from("ns", "sub")?,
                named(&[], Some("nsroot/ns/sub.py")),
            ),
            (
                "tests/test_core.py",
                import("mypkg", Imported::Module)?,
                named(&["src/mypkg/__init__.py"], None),
            ),
            (
                "tests/test_core.py",
                from("mypkg", "core")?,
                named(&["src/mypkg/__init__.py"], Some("src/mypkg/core.py")),
            ),
            (
                "tests/test_core.py",
                import("mypkg.core", Imported::Module)?,
                named(&["src/mypkg/core.py"], None),
            ),
            (
                "scripts/run.py",
                import("helper", Imported::Module)?,
                named(&["scripts/helper.py"], None),
            ),
            (
                "app/handlers/api.py",
                import("json", Imported::Module)?,
                named(&[], None),
            ),
            (
                "scripts/run.py",
                import("json", Imported::Module)?,
                named(&[], None),
            ),
            (
                "tests/test_core.py",
                import("pkg.mod", Imported::Module)?,
                named(&[], None),
            ),
            (
                "tests/test_core.py",
                import("only.thing", Imported::All)?,
                named(&[], None),
            ),
            (
                "tests/test_core.py",
                from("app.handlers", "api")?,
                named(&["app/handlers/__init__.py"], Some("app/handlers/api.py")),
            ),
            (
                "services/api/app/views.py",
                from("app.models", "Order")?,
                named(&["services/api/app/models.py"], None),
            ),
        ];
        for (importer, import, expected) in cases {
            let found = resolved(&paths, importer, std::slice::from_ref(&import));
            assert_eq!(found, [expected], "{importer} {:?}", import.specifier);
        }
        Ok(())
    }

    /// A module directly in a source root, or in a folder of one, is found under the one source
    /// root that holds it; the repository root is searched for an importer in a folder of its
    /// own. Two source roots that hold one module leave it unresolved, and so do more source roots
    /// holding its top-level name than the limit allows, even when only one holds the module; as
    /// many as the limit allows still resolve it.
    #[test]
    fn source_roots_resolve_only_a_clear_choice() -> Result<(), Box<dyn Error>> {
        let mut paths = vec![
            "a/pyproject.toml".to_owned(),
            "a/src/shared/x.py".to_owned(),
            "b/setup.py".to_owned(),
            "b/shared/x.py".to_owned(),
            "c/setup.cfg".to_owned(),
            "c/solo/y.py".to_owned(),
            "d/setup.py".to_owned(),
            "d/flat.py".to_owned(),
            "lib/util.py".to_owned(),
            "tools/run.py".to_owned(),
            "main.py".to_owned(),
        ];
        for at in 0..=MAX_SOURCE_ROOTS_PER_NAME {
            paths.push(format!("many{at}/pyproject.toml"));
            paths.push(format!("many{at}/common/other.py"));
        }
        paths.push("many0/common/target.py".to_owned());
        for at in 0..MAX_SOURCE_ROOTS_PER_NAME {
            paths.push(format!("edge{at}/pyproject.toml"));
            paths.push(format!("edge{at}/edge/other.py"));
        }
        paths.push("edge0/edge/target.py".to_owned());
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        let imports = [
            import("shared.x", Imported::Module)?,
            import("solo.y", Imported::Module)?,
            import("flat", Imported::Module)?,
            import("common.target", Imported::Module)?,
            import("edge.target", Imported::Module)?,
        ];
        let expected = [
            named(&[], None),
            named(&["c/solo/y.py"], None),
            named(&["d/flat.py"], None),
            named(&[], None),
            named(&["edge0/edge/target.py"], None),
        ];
        assert_eq!(resolved(&paths, "main.py", &imports), expected);
        let from_tools = resolved(
            &paths,
            "tools/run.py",
            &[import("lib.util", Imported::Module)?],
        );
        assert_eq!(from_tools, [named(&["lib/util.py"], None)]);
        Ok(())
    }

    /// A repository root that is itself a package is searched neither as the root nor as a source
    /// root: its modules are imported under its own name. A relative import from a package inside
    /// it climbs to it, and no further.
    #[test]
    fn a_repository_root_that_is_a_package_is_not_searched() -> Result<(), Box<dyn Error>> {
        let paths = [
            "__init__.py",
            "pyproject.toml",
            "json.py",
            "sub/x.py",
            "pkg/__init__.py",
            "pkg/y.py",
        ];
        let found = resolved(&paths, "sub/x.py", &[import("json", Imported::Module)?]);
        assert_eq!(found, [named(&[], None)]);
        let climbed = resolved(
            &paths,
            "pkg/y.py",
            &[from("..", "json")?, from("...", "json")?],
        );
        assert_eq!(
            climbed,
            [named(&["__init__.py"], Some("json.py")), named(&[], None)]
        );
        Ok(())
    }
}
