//! Rust imports. The nearest Cargo.toml above the importer gives its package; one that cannot be
//! read or declares no package (a virtual workspace) still ends the search, so its files resolve
//! nothing. A `use` path that starts with `crate`, `self`, `super`, a module of the importer's own
//! module, or a crate the package names is walked one part at a time through module files: `x.rs`,
//! then `x/mod.rs`. A part with no file of its own, such as an inline module or a type, stops the
//! walk and the path names nothing; a path that starts anywhere else names a crate outside the
//! repository, and no file. `use a::b::c` also names `c`'s file when `c` is a module of its own.
//!
//! A file belongs to the crate whose root is in the nearest folder above it that holds a root:
//! the library (`[lib] path`, else `src/lib.rs`), `src/main.rs`, the build script, a `[[bin]]`,
//! `[[test]]`, `[[example]]` or `[[bench]]` path, or a file Cargo finds by its place in `src/bin`,
//! `tests`, `examples` or `benches` (directly, or as `main.rs` one folder down). Of several roots
//! in that folder it is the library's, whose modules are the same files as the others', but the
//! root file itself is unclear: `src/lib.rs` or `src/main.rs`, so a path that ends at the root
//! names only the module it takes, if any. Without the library among them, as for a
//! `tests/common` the tests beside it share, the crate is unclear and `crate` names nothing.
//!
//! The crates a package names are its own library and its dependencies on packages of the
//! repository: a dependency with a `path`, or with `workspace = true` and a `path` in the
//! workspace's dependency table. One renamed with `package` is named after its key, any other
//! after the library of the package it names; a name two of them give, one for each platform for
//! instance, names nothing.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Identifier, Language, RepoPath};
use toml::{Table, Value};

use super::{Files, ReadText, Resolver, Targets, folders_above, joined};
use crate::specifiers::{Import, Imported};

/// The file that declares a package.
const MANIFEST: &str = "Cargo.toml";

/// The folders, from a package's own, whose `.rs` files are crate roots Cargo finds by their
/// place, as are the `main.rs` files of their subfolders.
const TARGET_FOLDERS: [&str; 4] = ["src/bin", "tests", "examples", "benches"];

/// The manifest's tables of crates whose `path` gives a crate root.
const TARGET_TABLES: [&str; 4] = ["bin", "test", "example", "bench"];

/// The manifest's tables of dependencies, at the top and under each `[target]`.
const DEPENDENCY_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// Resolves Rust imports with the repository's packages.
struct Rust {
    /// The package of each folder that holds a Cargo.toml; `None` for one that declares no
    /// package or cannot be read.
    packages: BTreeMap<RepoPath, Option<Package>>,
}

/// One package of the repository.
struct Package {
    /// The library's root file, when the package has a library.
    library: Option<RepoPath>,
    /// The crate root files in each folder that holds one, in path order.
    roots: BTreeMap<RepoPath, Vec<RepoPath>>,
    /// The library root of each crate of the repository the package's code can name.
    crates: BTreeMap<String, RepoPath>,
}

/// A module a walk reaches.
#[derive(Debug, Clone)]
struct Module {
    /// The file the module is written in.
    file: RepoPath,
    /// The folder its own modules' files are in.
    children: RepoPath,
}

impl Module {
    /// The module a crate root file is: its own modules' files are beside it.
    fn root(file: RepoPath) -> Option<Self> {
        Some(Self {
            children: file.parent()?,
            file,
        })
    }
}

/// Reads every Cargo.toml.
pub(super) fn load(files: &Files, read: ReadText<'_>) -> Box<dyn Resolver> {
    let manifests: BTreeMap<RepoPath, Option<Table>> = files
        .named(MANIFEST)
        .iter()
        .filter_map(|manifest| {
            let folder = manifest.parent()?;
            Some((folder, read(manifest).and_then(|text| text.parse().ok())))
        })
        .collect();
    let libraries: BTreeMap<RepoPath, (String, RepoPath)> = manifests
        .iter()
        .filter_map(|(folder, manifest)| {
            Some((
                folder.clone(),
                library_of(files, folder, manifest.as_ref()?)?,
            ))
        })
        .collect();
    let mains = mains_by_grandparent(files);
    let context = Context {
        files,
        manifests: &manifests,
        libraries: &libraries,
        mains: &mains,
    };
    let packages = manifests
        .iter()
        .map(|(folder, manifest)| {
            let declared = manifest
                .as_ref()
                .filter(|manifest| manifest.contains_key("package"));
            (
                folder.clone(),
                declared.map(|manifest| context.package(folder, manifest)),
            )
        })
        .collect();
    Box::new(Rust { packages })
}

/// What reading one package needs from the others.
struct Context<'a> {
    /// The repository's files.
    files: &'a Files,
    /// Each Cargo.toml by its folder; `None` for one that cannot be read.
    manifests: &'a BTreeMap<RepoPath, Option<Table>>,
    /// The library name and root of each package that has a library, by its folder.
    libraries: &'a BTreeMap<RepoPath, (String, RepoPath)>,
    /// The `main.rs` files below each folder, one folder down.
    mains: &'a BTreeMap<RepoPath, Vec<RepoPath>>,
}

impl Context<'_> {
    /// The package that `manifest`, in `folder`, declares.
    fn package(&self, folder: &RepoPath, manifest: &Table) -> Package {
        let library = self.libraries.get(folder).map(|(_, root)| root.clone());
        let mut by_folder: BTreeMap<RepoPath, Vec<RepoPath>> = BTreeMap::new();
        for root in self.roots(folder, manifest, library.as_ref()) {
            if let Some(parent) = root.parent() {
                by_folder.entry(parent).or_default().push(root);
            }
        }
        Package {
            library,
            roots: by_folder,
            crates: self.crates(folder, manifest),
        }
    }

    /// The crate root files of the package that `manifest`, in `folder`, declares, its
    /// `library` included, in path order.
    fn roots(
        &self,
        folder: &RepoPath,
        manifest: &Table,
        library: Option<&RepoPath>,
    ) -> BTreeSet<RepoPath> {
        let mut roots: BTreeSet<RepoPath> = library.into_iter().cloned().collect();
        let mut add = |relative: &str| {
            roots.extend(joined(folder, relative).filter(|file| self.files.contains(file)));
        };
        add("src/main.rs");
        match manifest
            .get("package")
            .and_then(|package| package.get("build"))
        {
            Some(Value::String(build)) => add(build),
            Some(_) => {}
            None => add("build.rs"),
        }
        for table in TARGET_TABLES {
            let targets = manifest.get(table).and_then(Value::as_array);
            for target in targets.into_iter().flatten() {
                if let Some(path) = target.get("path").and_then(Value::as_str) {
                    add(path);
                }
            }
        }
        for target in TARGET_FOLDERS
            .iter()
            .filter_map(|target| joined(folder, target))
        {
            let direct = self.files.in_folder(&target).iter();
            roots.extend(
                direct
                    .filter(|file| file.extension() == Some("rs"))
                    .cloned(),
            );
            roots.extend(self.mains.get(&target).into_iter().flatten().cloned());
        }
        roots
    }

    /// The library root of each crate of the repository the package in `folder` can name: its own
    /// library and its dependencies, a name given to two roots left out.
    fn crates(&self, folder: &RepoPath, manifest: &Table) -> BTreeMap<String, RepoPath> {
        let mut crates: BTreeMap<String, BTreeSet<RepoPath>> = BTreeMap::new();
        if let Some((name, root)) = self.libraries.get(folder) {
            crates.entry(name.clone()).or_default().insert(root.clone());
        }
        let mut tables = vec![manifest];
        if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
            tables.extend(targets.values().filter_map(Value::as_table));
        }
        for table in tables {
            for section in DEPENDENCY_TABLES {
                let dependencies = table.get(section).and_then(Value::as_table);
                for (key, spec) in dependencies.into_iter().flatten() {
                    if let Some((name, root)) = self.dependency(folder, key, spec) {
                        crates.entry(name).or_default().insert(root);
                    }
                }
            }
        }
        crates
            .into_iter()
            .filter_map(|(name, roots)| match (roots.first(), roots.len()) {
                (Some(root), 1) => Some((name, root.clone())),
                _ => None,
            })
            .collect()
    }

    /// The crate the dependency `key = spec` of the package in `folder` names, when it is a
    /// package of the repository with a library.
    fn dependency(&self, folder: &RepoPath, key: &str, spec: &Value) -> Option<(String, RepoPath)> {
        let spec = spec.as_table()?;
        let inherits = spec.get("workspace").and_then(Value::as_bool) == Some(true);
        let (base, given) = if inherits {
            let (workspace, manifest) = self.workspace_of(folder)?;
            let given = manifest.get("workspace")?.get("dependencies")?.get(key)?;
            (workspace, given.as_table()?)
        } else {
            (folder, spec)
        };
        let path = joined(base, given.get("path")?.as_str()?)?;
        let (library, root) = self.libraries.get(&path)?;
        let is_renamed = spec.contains_key("package") || given.contains_key("package");
        let name = if is_renamed {
            key.replace('-', "_")
        } else {
            library.clone()
        };
        Some((name, root.clone()))
    }

    /// The folder and manifest of the workspace the package in `folder` belongs to: the nearest
    /// manifest at or above it with a `[workspace]` table.
    fn workspace_of(&self, folder: &RepoPath) -> Option<(&RepoPath, &Table)> {
        std::iter::successors(Some(folder.clone()), RepoPath::parent).find_map(|above| {
            let (at, manifest) = self.manifests.get_key_value(&above)?;
            let manifest = manifest.as_ref()?;
            manifest.contains_key("workspace").then_some((at, manifest))
        })
    }
}

/// The library name and root file a manifest in `folder` declares: named after `[lib] name`,
/// else the package with `-` as `_`, at `[lib] path`, else `src/lib.rs`; `None` when it declares
/// no package or its library's root is not a file of the repository.
fn library_of(files: &Files, folder: &RepoPath, manifest: &Table) -> Option<(String, RepoPath)> {
    let package_name = manifest.get("package")?.get("name")?.as_str()?;
    let library = manifest.get("lib");
    let name = match library.and_then(|library| library.get("name")) {
        Some(name) => name.as_str()?.to_owned(),
        None => package_name.replace('-', "_"),
    };
    let path = match library.and_then(|library| library.get("path")) {
        Some(path) => path.as_str()?,
        None => "src/lib.rs",
    };
    let root = joined(folder, path).filter(|file| files.contains(file))?;
    Some((name, root))
}

/// The `main.rs` files of the repository, by the folder one above their own.
fn mains_by_grandparent(files: &Files) -> BTreeMap<RepoPath, Vec<RepoPath>> {
    let mut mains: BTreeMap<RepoPath, Vec<RepoPath>> = BTreeMap::new();
    for main in files.named("main.rs") {
        if let Some(grandparent) = main.parent().and_then(|parent| parent.parent()) {
            mains.entry(grandparent).or_default().push(main.clone());
        }
    }
    mains
}

impl Resolver for Rust {
    fn reads(&self, language: Language) -> bool {
        language == Language::Rust
    }

    fn targets(&self, files: &Files, importer: &RepoPath, import: &Import) -> Targets {
        let path = (import.specifier.as_str(), import.module_depth);
        let Some((module, is_known)) = self.walk(files, importer, path) else {
            return Targets::default();
        };
        let submodule = match &import.imported {
            Imported::Name(name) => child(files, &module, name.as_str()).map(|inner| inner.file),
            Imported::Module | Imported::All | Imported::Default => None,
        };
        Targets {
            module: if is_known {
                vec![module.file]
            } else {
                Vec::new()
            },
            submodule,
        }
    }

    fn module_name(&self, specifier: &str) -> Option<Identifier> {
        let last = specifier.rsplit("::").next()?;
        // `crate`, `self` and `super` start paths; they are no module's own name.
        if matches!(last, "crate" | "self" | "super") {
            return None;
        }
        Identifier::new(last).ok()
    }
}

impl Rust {
    /// The module `path`, the module part of a `use` path made `depth` inline modules deep, names
    /// from the file at `importer`, and whether its file is known: not when it is a crate root the
    /// importer may not belong to, whose modules are still the files a walk reaches. `None` when
    /// the path names a crate outside the repository or a part has no file of its own. Inside an
    /// inline module, `self` is that module and the first `super`s climb the inline modules, whose
    /// items no file of their own holds, so such a path names nothing until it leaves them; a
    /// plain first name can only be a crate there.
    fn walk(
        &self,
        files: &Files,
        importer: &RepoPath,
        (path, depth): (&str, u32),
    ) -> Option<(Module, bool)> {
        let package = folders_above(importer).find_map(|folder| self.packages.get(&folder))?;
        let package = package.as_ref()?;
        let owner = package.owning_root(importer);
        let is_root_known = owner.as_ref().is_some_and(|(_, is_known)| *is_known);
        let root = owner.and_then(|(file, _)| Module::root(file));
        let own = own_module(importer, root.as_ref())?;
        let mut parts = path.split("::").peekable();
        let mut module = match parts.next()? {
            "crate" => root.clone()?,
            "self" if depth > 0 => return None,
            "self" => own,
            "super" => {
                let mut climbs: u32 = 1;
                while parts.next_if_eq(&"super").is_some() {
                    climbs += 1;
                }
                let mut module = own;
                for _ in 0..climbs.checked_sub(depth)? {
                    module = parent(files, &module, root.as_ref())?;
                }
                module
            }
            first => match child(files, &own, first).filter(|_| depth == 0) {
                Some(inner) => inner,
                None => Module::root(package.crates.get(first)?.clone())?,
            },
        };
        for part in parts {
            module = match part {
                "super" => parent(files, &module, root.as_ref())?,
                name => child(files, &module, name)?,
            };
        }
        let is_unknown_root =
            !is_root_known && root.as_ref().is_some_and(|root| root.file == module.file);
        Some((module, !is_unknown_root))
    }
}

impl Package {
    /// The root of the crate the file at `importer`, a file of this package, belongs to, and
    /// whether that root file itself is known: the importer itself when it is a root; else the
    /// root in the nearest folder above it that holds any. Of several there it is the library,
    /// whose modules are the same files as the others' though its root file may not be the
    /// importer's, and none when the library is not among them.
    fn owning_root(&self, importer: &RepoPath) -> Option<(RepoPath, bool)> {
        let here = importer.parent()?;
        if self
            .roots
            .get(&here)
            .is_some_and(|roots| roots.contains(importer))
        {
            return Some((importer.clone(), true));
        }
        for above in std::iter::successors(Some(here), RepoPath::parent) {
            if let Some(roots) = self.roots.get(&above) {
                return match roots.as_slice() {
                    [only] => Some((only.clone(), true)),
                    several => self
                        .library
                        .clone()
                        .filter(|library| several.contains(library))
                        .map(|library| (library, false)),
                };
            }
        }
        None
    }
}

/// The module the file at `importer` is, in the crate whose root is `root`, when that is known: a
/// root's and a `mod.rs` file's own modules are beside it, any other file's in the folder of its
/// name.
fn own_module(importer: &RepoPath, root: Option<&Module>) -> Option<Module> {
    if let Some(root) = root.filter(|root| root.file == *importer) {
        return Some(root.clone());
    }
    let folder = importer.parent()?;
    let children = match importer.file_name() {
        "mod.rs" => folder,
        name => folder.join(name.strip_suffix(".rs")?).ok()?,
    };
    Some(Module {
        file: importer.clone(),
        children,
    })
}

/// The module `name` of `module`, when it has a file of its own; a raw name such as `r#type`
/// is looked for without its `r#`.
fn child(files: &Files, module: &Module, name: &str) -> Option<Module> {
    let name = name.strip_prefix("r#").unwrap_or(name);
    let children = module.children.join(name).ok()?;
    let file = module_file(files, &children)?;
    Some(Module { file, children })
}

/// The module `module` belongs to, in the crate whose root is `root` when that is known; `None`
/// for the root.
fn parent(files: &Files, module: &Module, root: Option<&Module>) -> Option<Module> {
    if root.is_some_and(|root| root.file == module.file) {
        return None;
    }
    let children = module.children.parent()?;
    if let Some(root) = root.filter(|root| root.children == children) {
        return Some(root.clone());
    }
    let file = module_file(files, &children)?;
    Some(Module { file, children })
}

/// The file of the module whose own modules' files are in `children`: the `.rs` file of its
/// name beside that folder, else the folder's `mod.rs`.
fn module_file(files: &Files, children: &RepoPath) -> Option<RepoPath> {
    let beside = children
        .parent()
        .and_then(|folder| folder.join(&format!("{}.rs", children.file_name())).ok());
    let inside = children.join("mod.rs").ok();
    [beside, inside]
        .into_iter()
        .flatten()
        .find(|file| files.contains(file))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::test_support::{Named, import, import_name, named, named_by, path};

    /// The root manifest of the test repository: a package with a `[[bin]]` of its own path, a
    /// workspace with two path dependencies of its own, one renamed there, and dependencies by
    /// path, renamed by `package`, inherited from the workspace, from a registry, for tests only, for one platform,
    /// and one name given to a package for each of two platforms.
    const ROOT_MANIFEST: &str = r#"[package]
name = "shop-core"
version = "0.1.0"

[workspace]
members = ["crates/*", "bins"]

[workspace.dependencies]
logger = { path = "crates/logger" }
wlog = { package = "logger", path = "crates/logger" }

[dependencies]
util-lib = { path = "crates/util" }
fmtlib = { package = "util-lib", path = "crates/util" }
logger = { workspace = true }
wlog = { workspace = true }
config = "1.0"
custom = { path = "crates/custom" }

[dev-dependencies]
testkit = { path = "crates/testkit" }

[target.'cfg(unix)'.dependencies]
platform = { path = "crates/platform" }
shared = { package = "shared-unix", path = "crates/shared-unix" }

[target.'cfg(windows)'.dependencies]
shared = { package = "shared-win", path = "crates/shared-win" }

[[bin]]
name = "cli"
path = "src/cli/main.rs"
"#;

    /// The manifests of the test repository: the root package, a package whose library is
    /// renamed, one whose library has a path of its own, six more packages, one manifest that
    /// cannot be read, a workspace that declares no package, and a package with no library and a
    /// build script of its own path.
    const MANIFESTS: [(&str, &str); 12] = [
        ("Cargo.toml", ROOT_MANIFEST),
        (
            "crates/util/Cargo.toml",
            "[package]\nname = \"util-lib\"\n\n[lib]\nname = \"util\"\n",
        ),
        ("crates/logger/Cargo.toml", "[package]\nname = \"logger\"\n"),
        ("crates/config/Cargo.toml", "[package]\nname = \"config\"\n"),
        (
            "crates/custom/Cargo.toml",
            "[package]\nname = \"custom\"\n\n[lib]\npath = \"lib/custom.rs\"\n",
        ),
        (
            "crates/testkit/Cargo.toml",
            "[package]\nname = \"testkit\"\n",
        ),
        (
            "crates/platform/Cargo.toml",
            "[package]\nname = \"platform\"\n",
        ),
        (
            "crates/shared-unix/Cargo.toml",
            "[package]\nname = \"shared-unix\"\n",
        ),
        (
            "crates/shared-win/Cargo.toml",
            "[package]\nname = \"shared-win\"\n",
        ),
        ("crates/broken/Cargo.toml", "[package\nname = "),
        ("vroot/Cargo.toml", "[workspace]\nmembers = []\n"),
        (
            "bins/Cargo.toml",
            "[package]\nname = \"only-bin\"\nbuild = \"tools/gen.rs\"\n",
        ),
    ];

    /// The source files of the test repository.
    const SOURCES: [&str; 35] = [
        "src/lib.rs",
        "src/main.rs",
        "src/models.rs",
        "src/models/order.rs",
        "src/api/mod.rs",
        "src/api/routes.rs",
        "src/type.rs",
        "src/config.rs",
        "src/cli/main.rs",
        "src/cli/args.rs",
        "src/bin/tool/main.rs",
        "src/bin/tool/helper.rs",
        "tests/flow.rs",
        "tests/other.rs",
        "tests/helpers.rs",
        "tests/common/mod.rs",
        "crates/util/src/lib.rs",
        "crates/util/src/fmt.rs",
        "crates/logger/src/lib.rs",
        "crates/config/src/lib.rs",
        "crates/broken/src/lib.rs",
        "vroot/scripts/x.rs",
        "bins/src/main.rs",
        "bins/src/cli.rs",
        "bins.rs",
        "build.rs",
        "helper.rs",
        "crates/custom/lib/custom.rs",
        "crates/testkit/src/lib.rs",
        "crates/platform/src/lib.rs",
        "crates/shared-unix/src/lib.rs",
        "crates/shared-win/src/lib.rs",
        "bins/tools/gen.rs",
        "bins/tools/helper.rs",
        "vroot/scripts/x/helper.rs",
    ];

    /// What `use <specifier>::<name>` names from the file at `importer` in the test repository,
    /// or what an import of the whole module names when `name` is `None`.
    fn resolved(
        importer: &str,
        specifier: &str,
        name: Option<&str>,
    ) -> Result<Named, Box<dyn Error>> {
        let files = Files::new(
            SOURCES
                .iter()
                .chain(MANIFESTS.iter().map(|(at, _)| at))
                .map(|at| path(at)),
        );
        let read = |at: &RepoPath| {
            MANIFESTS
                .iter()
                .find(|(manifest, _)| path(manifest) == *at)
                .map(|(_, text)| (*text).to_owned())
        };
        let resolver = load(&files, &read);
        let import = match name {
            Some(name) => import_name(specifier, name)?,
            None => import(specifier, Imported::Module)?,
        };
        Ok(named_by(&resolver.targets(
            &files,
            &path(importer),
            &import,
        )))
    }

    /// One case: the importer, the module part of its `use` path, the name it takes or `None` for
    /// the whole module, and what the import is expected to name.
    type Case<'a> = (&'a str, &'a str, Option<&'a str>, Named);

    /// Checks each case against what the test repository resolves it to.
    fn check(cases: &[Case<'_>]) -> Result<(), Box<dyn Error>> {
        for (importer, specifier, name, expected) in cases {
            let found = resolved(importer, specifier, *name)?;
            assert_eq!(&found, expected, "{importer} {specifier} {name:?}");
        }
        Ok(())
    }

    /// Paths from `crate`, `self`, `super` and the importer's own modules walk through module
    /// files; a part with no file names nothing, and a name that is a module of its own is named
    /// too. Where `src/lib.rs` and `src/main.rs` share the folder, a path that ends at the crate
    /// root names nothing.
    #[test]
    fn use_paths_walk_through_module_files() -> Result<(), Box<dyn Error>> {
        let routes = "src/api/routes.rs";
        check(&[
            (
                routes,
                "crate::models",
                Some("Order"),
                named(&["src/models.rs"], None),
            ),
            (
                routes,
                "crate::models",
                Some("order"),
                named(&["src/models.rs"], Some("src/models/order.rs")),
            ),
            (
                routes,
                "crate::models::order",
                Some("Line"),
                named(&["src/models/order.rs"], None),
            ),
            (routes, "super", None, named(&["src/api/mod.rs"], None)),
            (
                routes,
                "super::super::models",
                Some("Order"),
                named(&["src/models.rs"], None),
            ),
            (routes, "self::inner", Some("X"), named(&[], None)),
            (routes, "crate", Some("Thing"), named(&[], None)),
            (
                routes,
                "crate::r#type",
                Some("Kind"),
                named(&["src/type.rs"], None),
            ),
            (routes, "crate::inline", Some("X"), named(&[], None)),
        ])
    }

    /// The crates a package names are its dependencies on the repository's packages (for tests,
    /// for one platform, inherited from the workspace or not), by their library's name or the key
    /// that renames them, a library's own path followed; a registry dependency named like a
    /// package of the repository, a package name that is not its library's, a name two
    /// dependencies give, the standard library and a package no one depends on name nothing.
    #[test]
    fn crates_are_named_by_the_package_dependencies() -> Result<(), Box<dyn Error>> {
        let routes = "src/api/routes.rs";
        check(&[
            (
                routes,
                "util::fmt",
                Some("Money"),
                named(&["crates/util/src/fmt.rs"], None),
            ),
            (
                routes,
                "util",
                None,
                named(&["crates/util/src/lib.rs"], None),
            ),
            (
                routes,
                "fmtlib::fmt",
                Some("Money"),
                named(&["crates/util/src/fmt.rs"], None),
            ),
            (
                routes,
                "logger",
                Some("Log"),
                named(&["crates/logger/src/lib.rs"], None),
            ),
            (
                routes,
                "wlog",
                Some("X"),
                named(&["crates/logger/src/lib.rs"], None),
            ),
            (
                routes,
                "custom",
                Some("X"),
                named(&["crates/custom/lib/custom.rs"], None),
            ),
            (
                routes,
                "platform",
                Some("X"),
                named(&["crates/platform/src/lib.rs"], None),
            ),
            (
                "tests/flow.rs",
                "testkit",
                Some("X"),
                named(&["crates/testkit/src/lib.rs"], None),
            ),
            (routes, "shared", Some("X"), named(&[], None)),
            (routes, "config", Some("Config"), named(&[], None)),
            (routes, "util_lib", Some("X"), named(&[], None)),
            (
                routes,
                "std::collections",
                Some("HashMap"),
                named(&[], None),
            ),
        ])
    }

    /// The Rust resolver reads Rust imports only, so an import in another language never reaches
    /// it, whatever its text.
    #[test]
    fn only_rust_imports_are_read() {
        let files = Files::new(Vec::new());
        let resolver = load(&files, &|_| None);
        assert!(resolver.reads(Language::Rust));
        for other in [
            Language::C,
            Language::Python,
            Language::TypeScript,
            Language::Go,
        ] {
            assert!(!resolver.reads(other), "{other:?}");
        }
    }

    /// A file walks from the crate it belongs to: a crate root (a build script at a path of its
    /// own included) is its own, with its modules beside it, as a `mod.rs` file's are, and nothing
    /// above it, even a file named after the package's folder. Any other file belongs to the root
    /// in the nearest folder above it: the one library or binary of its folder, a `[[bin]]` at a
    /// path of its own, or the library among several, though a path ending at that root names
    /// nothing; where several roots share the folder without the library, as the tests sharing
    /// `tests/common` do, `crate` names nothing. A package's own library is named by its name. A
    /// Cargo.toml that cannot be read or declares no package keeps its files from the package
    /// above it, and their paths name nothing.
    #[test]
    fn each_file_walks_from_its_own_crate() -> Result<(), Box<dyn Error>> {
        let cases: [Case<'_>; 19] = [
            (
                "bins/tools/gen.rs",
                "helper",
                Some("X"),
                named(&["bins/tools/helper.rs"], None),
            ),
            ("vroot/scripts/x.rs", "helper", Some("X"), named(&[], None)),
            (
                "src/bin/tool/main.rs",
                "helper",
                Some("X"),
                named(&["src/bin/tool/helper.rs"], None),
            ),
            (
                "src/bin/tool/helper.rs",
                "crate::config",
                Some("X"),
                named(&[], None),
            ),
            (
                "src/cli/main.rs",
                "crate::args",
                Some("Parsed"),
                named(&["src/cli/args.rs"], None),
            ),
            (
                "tests/common/mod.rs",
                "crate::helpers",
                Some("X"),
                named(&[], None),
            ),
            ("build.rs", "helper", Some("X"), named(&["helper.rs"], None)),
            (
                "src/api/mod.rs",
                "routes",
                Some("X"),
                named(&["src/api/routes.rs"], None),
            ),
            ("bins/src/main.rs", "super", Some("x"), named(&[], None)),
            (
                "src/main.rs",
                "crate",
                Some("Config"),
                named(&["src/main.rs"], None),
            ),
            (
                "src/main.rs",
                "shop_core::models",
                Some("Order"),
                named(&["src/models.rs"], None),
            ),
            (
                "src/lib.rs",
                "models",
                Some("Order"),
                named(&["src/models.rs"], None),
            ),
            ("src/lib.rs", "super", Some("x"), named(&[], None)),
            ("src/models.rs", "super", Some("Thing"), named(&[], None)),
            (
                "crates/util/src/fmt.rs",
                "super",
                Some("X"),
                named(&["crates/util/src/lib.rs"], None),
            ),
            (
                "tests/flow.rs",
                "common",
                Some("setup"),
                named(&["tests/common/mod.rs"], None),
            ),
            (
                "crates/broken/src/lib.rs",
                "crate::models",
                Some("Order"),
                named(&[], None),
            ),
            (
                "vroot/scripts/x.rs",
                "crate::models",
                Some("Order"),
                named(&[], None),
            ),
            (
                "bins/src/cli.rs",
                "crate",
                Some("run"),
                named(&["bins/src/main.rs"], None),
            ),
        ];
        check(&cases)
    }

    /// A module brought in whole is reached by its path's last part; `crate`, `self` and `super`
    /// name no module of their own.
    #[test]
    fn whole_modules_are_named_by_their_last_part() {
        let rust = load(&Files::new(Vec::new()), &|_| None);
        let cases = [
            ("serde", Some("serde")),
            ("std::io", Some("io")),
            ("crate::models", Some("models")),
            ("crate", None),
            ("super", None),
            ("self", None),
        ];
        for (specifier, expected) in cases {
            let name = rust.module_name(specifier);
            assert_eq!(
                name.as_ref().map(Identifier::as_str),
                expected,
                "{specifier}"
            );
        }
    }

    /// Inside inline modules, `self` names nothing and the first `super`s climb the inline modules
    /// of the file: one module deep in `src/api/routes.rs`, `super` is the file's own module and
    /// `super::super` its parent, while two deep `super` is still inside the file. A plain first
    /// name there can only be a crate, not one of the file's own modules: `order` is
    /// `src/models/order.rs` from `src/models.rs` only outside inline modules.
    #[test]
    fn inline_modules_take_the_first_steps_of_a_path() -> Result<(), Box<dyn Error>> {
        let files = Files::new(
            SOURCES
                .iter()
                .chain(MANIFESTS.iter().map(|(at, _)| at))
                .map(|at| path(at)),
        );
        let read = |at: &RepoPath| {
            MANIFESTS
                .iter()
                .find(|(manifest, _)| path(manifest) == *at)
                .map(|(_, text)| (*text).to_owned())
        };
        let resolver = load(&files, &read);
        let routes = "src/api/routes.rs";
        let cases = [
            (routes, "super", 0, named(&["src/api/mod.rs"], None)),
            (routes, "self", 0, named(&[routes], None)),
            (routes, "super", 1, named(&[routes], None)),
            (routes, "super::super", 1, named(&["src/api/mod.rs"], None)),
            (routes, "super", 2, named(&[], None)),
            (routes, "self", 1, named(&[], None)),
            (
                "src/models.rs",
                "order",
                0,
                named(&["src/models/order.rs"], None),
            ),
            ("src/models.rs", "order", 1, named(&[], None)),
        ];
        for (importer, specifier, depth, expected) in cases {
            let mut whole = import(specifier, Imported::Module)?;
            whole.module_depth = depth;
            let found = named_by(&resolver.targets(&files, &path(importer), &whole));
            assert_eq!(found, expected, "{importer} {specifier} {depth}");
        }
        Ok(())
    }
}
