//! Go imports. Each go.mod gives its folder a module path. An import path that starts with a
//! module path, the longest when several do, names the package folder below that module's
//! folder, and the import names the package's `.go` files, leaving out its `_test.go` files and
//! those whose names start with `_` or `.`, as the Go tool does. Other import paths (the standard
//! library, other modules) name no file, and neither does a path Go would refuse.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Language, RepoPath};

use super::{Files, ReadText, Resolver, Targets, folders_above, joined};
use crate::specifiers::Import;

/// The file that declares a module.
const MODULE_FILE: &str = "go.mod";

/// Resolves Go imports with the repository's module paths.
struct Go {
    /// The folders of the go.mod files that declare each module path.
    modules: BTreeMap<String, BTreeSet<RepoPath>>,
}

/// Reads the module path of every go.mod; one without a `module` line is left out.
pub(super) fn load(files: &Files, read: ReadText<'_>) -> Box<dyn Resolver> {
    let mut modules: BTreeMap<String, BTreeSet<RepoPath>> = BTreeMap::new();
    for file in files.named(MODULE_FILE) {
        let path = read(file).and_then(|text| module_path(&text));
        if let (Some(path), Some(folder)) = (path, file.parent()) {
            modules.entry(path).or_default().insert(folder);
        }
    }
    Box::new(Go { modules })
}

impl Resolver for Go {
    fn reads(&self, language: Language) -> bool {
        language == Language::Go
    }

    fn targets(&self, files: &Files, importer: &RepoPath, import: &Import) -> Targets {
        let module = self
            .package_folder(importer, import.specifier.as_str())
            .map(|folder| {
                files
                    .in_folder(&folder)
                    .iter()
                    .filter(|file| is_package_file(file))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Targets {
            module,
            submodule: None,
        }
    }
}

impl Go {
    /// The folder of the package at `path`: below the folder of the longest module path it
    /// starts with, cut at a `/`. When two go.mod files declare that path, the one above the
    /// importer counts, and none when neither is.
    fn package_folder(&self, importer: &RepoPath, path: &str) -> Option<RepoPath> {
        if !is_import_path(path) {
            return None;
        }
        let mut module = path;
        let folders = loop {
            if let Some(folders) = self.modules.get(module) {
                break folders;
            }
            module = module.rsplit_once('/')?.0;
        };
        let folder = match (folders.first(), folders.len()) {
            (Some(only), 1) => only.clone(),
            _ => folders_above(importer).find(|above| folders.contains(above))?,
        };
        let rest = path.strip_prefix(module)?.trim_start_matches('/');
        joined(&folder, rest)
    }
}

/// Whether `path` is an import path Go accepts: elements between `/`, none of them empty or made
/// of dots only, so no element climbs out of the module.
fn is_import_path(path: &str) -> bool {
    path.split('/')
        .all(|element| !element.is_empty() && !element.chars().all(|mark| mark == '.'))
}

/// Whether `file`, in a package folder, belongs to the package as the Go tool builds it: a `.go`
/// file, not a test, and not one whose name starts with `_` or `.`.
fn is_package_file(file: &RepoPath) -> bool {
    let name = file.file_name();
    file.extension() == Some("go")
        && !name.ends_with("_test.go")
        && !name.starts_with('_')
        && !name.starts_with('.')
}

/// The module path a go.mod's text declares: the one word after the word `module` that starts a
/// line, quoted or not, comments left out; `None` when no line declares one that way, as for the
/// block form `module (`, which is left unread.
fn module_path(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.split("//").next().unwrap_or(line);
        let mut words = line.split_whitespace();
        if words.next()? != "module" {
            return None;
        }
        let path = words
            .next()?
            .trim_matches(|mark| mark == '"' || mark == '`');
        let is_alone = words.next().is_none() && !path.starts_with('(');
        is_alone.then(|| path.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;

    use super::*;
    use crate::specifiers::Imported;
    use crate::test_support::{import, path};

    /// The files each import path names when the file at `importer` imports it, in a repository
    /// of the files at `paths` and the go.mod files `modules`, given by path and text.
    fn resolved(
        paths: &[&str],
        modules: &[(&str, &str)],
        importer: &str,
        imports: &[&str],
    ) -> Result<Vec<Vec<String>>, Box<dyn Error>> {
        let texts: BTreeMap<RepoPath, String> = modules
            .iter()
            .map(|(at, text)| (path(at), (*text).to_owned()))
            .collect();
        let files = Files::new(paths.iter().map(|at| path(at)).chain(texts.keys().cloned()));
        let resolver = load(&files, &|at| texts.get(at).cloned());
        let mut found = Vec::new();
        for specifier in imports {
            let targets = resolver.targets(
                &files,
                &path(importer),
                &import(specifier, Imported::Module)?,
            );
            assert_eq!(targets.submodule, None, "{specifier}");
            found.push(
                targets
                    .module
                    .iter()
                    .map(|at| at.as_str().to_owned())
                    .collect(),
            );
        }
        Ok(found)
    }

    /// An import path names the `.go` files of its package folder below the module with the
    /// longest path it starts with, test files, files the Go tool ignores, other files and
    /// subfolders left out. A path that only shares a module's first letters, the standard
    /// library, other modules, a path Go would refuse (an element `..` or empty) and a module
    /// declared by a line Go would refuse name nothing.
    #[test]
    fn import_paths_name_package_folders() -> Result<(), Box<dyn Error>> {
        let modules = [
            ("go.mod", "module example.com/shop\n\ngo 1.22\n"),
            (
                "third/go.mod",
                "// the generator\nmodule example.com/shop/tools // pinned\n",
            ),
            ("quoted/go.mod", "module \"example.com/quoted\"\n"),
            ("blank/go.mod", "go 1.22\nmodulexyz example.com/bogus\n"),
            ("two/go.mod", "module example.com/two extra\n"),
            ("block/go.mod", "module (\n\texample.com/block\n)\n"),
        ];
        let paths = [
            "main.go",
            "orders/order.go",
            "orders/order_test.go",
            "orders/_draft.go",
            "orders/.hidden.go",
            "orders/doc.txt",
            "orders/internal/db.go",
            "third/gen/gen.go",
            "tools/gen/other.go",
            "quoted/x/x.go",
            "api/handler.go",
            "ping/cart/cart.go",
            "blank/x/x.go",
            "two/x/x.go",
            "block/x/x.go",
        ];
        let cases: [(&str, &[&str]); 14] = [
            ("example.com/shop/orders", &["orders/order.go"]),
            ("example.com/shop", &["main.go"]),
            ("example.com/shop/tools/gen", &["third/gen/gen.go"]),
            ("example.com/quoted/x", &["quoted/x/x.go"]),
            ("example.com/shopping/cart", &[]),
            ("fmt", &[]),
            ("github.com/other/lib", &[]),
            ("example.com/shop/orders/../tools/gen", &[]),
            ("example.com/shop//orders", &[]),
            ("example.com/shop/orders/", &[]),
            ("example.com/bogus/x", &[]),
            ("example.com/two/x", &[]),
            ("example.com/block/x", &[]),
            ("(/x", &[]),
        ];
        let specifiers: Vec<&str> = cases.iter().map(|(specifier, _)| *specifier).collect();
        let found = resolved(&paths, &modules, "api/handler.go", &specifiers)?;
        let expected: Vec<Vec<String>> = cases
            .iter()
            .map(|(_, files)| files.iter().map(|at| (*at).to_owned()).collect())
            .collect();
        assert_eq!(found, expected);
        Ok(())
    }

    /// When two go.mod files declare one module path, an importer below one of them uses it, and
    /// an importer below neither resolves nothing.
    #[test]
    fn a_module_path_declared_twice_counts_only_above_the_importer() -> Result<(), Box<dyn Error>> {
        let modules = [
            ("copy1/go.mod", "module example.com/dup\n"),
            ("copy2/go.mod", "module example.com/dup\n"),
        ];
        let paths = [
            "copy1/a/a.go",
            "copy1/main.go",
            "copy2/a/a.go",
            "other/main.go",
        ];
        let found = resolved(&paths, &modules, "copy1/main.go", &["example.com/dup/a"])?;
        assert_eq!(found, [["copy1/a/a.go"]]);
        let found = resolved(&paths, &modules, "other/main.go", &["example.com/dup/a"])?;
        assert_eq!(found, [Vec::<String>::new()]);
        Ok(())
    }
}
