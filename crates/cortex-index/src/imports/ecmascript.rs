//! TypeScript and JavaScript imports. A relative specifier (`./orders`, `../util`) starts from the
//! importer's folder. Any other starts from the nearest tsconfig.json or jsconfig.json above the
//! importer: the `paths` pattern that matches it, then `baseUrl`. Without either it names a
//! package, and no file.
//!
//! A config is read with the configs it `extends`, comments and trailing commas allowed, each
//! file read once and its `paths` built once however many configs extend it. A config from a package is left out: it cannot
//! be read, and it sets nothing about the repository's paths. A config that cannot be read or
//! followed, or whose `paths` pass the limits below, leaves only relative specifiers to resolve.
//!
//! The path a specifier reaches is tried as TypeScript tries it: a script extension replaced by
//! its TypeScript twins first (`./x.js` stands for `x.ts`), any other extension as written, then
//! `.ts`, `.tsx`, `.d.ts`, `.js` and `.jsx` appended, then the same after `/index`.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use cortex_core::{Identifier, Language, RepoPath};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};

use super::{Files, ReadText, Resolver, Targets, folders_above, joined, jsonc};
use crate::specifiers::Import;

/// The files that set how a folder's imports resolve, in the order one folder's are tried.
const CONFIG_NAMES: [&str; 2] = ["tsconfig.json", "jsconfig.json"];

/// How many files one config may be built from, itself and everything it extends included. Real
/// chains hold two or three; a loop reaches the cap and leaves the config unreadable.
const MAX_CONFIG_FILES: usize = 16;

/// How many `paths` patterns with a `*` a config may hold. Every specifier is matched against
/// each, so a config with more is treated as unreadable; real configs hold a handful.
const MAX_WILDCARD_PATTERNS: usize = 256;

/// How many targets one `paths` pattern may list. Every target is tried in turn, so a config with
/// a longer list is treated as unreadable; real lists hold one to three.
const MAX_TARGETS: usize = 16;

/// The extensions appended to a path, in the order TypeScript tries them.
const APPENDED: [&str; 5] = [".ts", ".tsx", ".d.ts", ".js", ".jsx"];

/// For a path ending in a script extension, the extensions tried in its place, in the order
/// TypeScript tries them.
const REPLACED: [(&str, &[&str]); 8] = [
    ("ts", &[".ts", ".tsx", ".d.ts", ".js", ".jsx"]),
    ("js", &[".ts", ".tsx", ".d.ts", ".js", ".jsx"]),
    ("tsx", &[".tsx", ".ts", ".d.ts", ".jsx", ".js"]),
    ("jsx", &[".tsx", ".ts", ".d.ts", ".jsx", ".js"]),
    ("mts", &[".mts", ".d.mts", ".mjs"]),
    ("mjs", &[".mts", ".d.mts", ".mjs"]),
    ("cts", &[".cts", ".d.cts", ".cjs"]),
    ("cjs", &[".cts", ".d.cts", ".cjs"]),
];

/// Resolves TypeScript and JavaScript imports with what each folder's config says.
struct Ecmascript {
    /// What the config of each folder that holds one says.
    configs: BTreeMap<RepoPath, Settings>,
}

/// What a config says about specifiers that are not relative; nothing for one that cannot be read
/// or followed.
#[derive(Debug, Default)]
struct Settings {
    /// The folder `baseUrl` names.
    base_url: Option<RepoPath>,
    /// The `paths` patterns.
    paths: Option<Paths>,
}

/// The `paths` of a config: the patterns, and the folder their targets start from.
#[derive(Debug)]
struct Paths {
    /// The folder the targets start from: `baseUrl` when one is set, else the folder of the config
    /// that sets `paths`.
    base: RepoPath,
    /// The patterns, shared by every config that takes them from the same file.
    patterns: Arc<Patterns>,
}

/// The patterns one config file's `paths` sets.
#[derive(Debug)]
struct Patterns {
    /// The patterns without `*`, each with its targets.
    exact: BTreeMap<String, Vec<String>>,
    /// The patterns with one `*`, in the order the config lists them.
    wildcard: Vec<Pattern>,
}

/// One `paths` pattern with a `*`: a specifier it matches is tried at each of its targets in turn.
#[derive(Debug)]
struct Pattern {
    /// The text before the `*`.
    prefix: String,
    /// The text after the `*`.
    suffix: String,
    /// The paths to try, each with at most one `*`, which stands for what the pattern's `*`
    /// matched.
    targets: Vec<String>,
}

/// One config file as written, before what it extends is applied. Settings Cortex does not read
/// are ignored; one it reads with the wrong type makes the file unreadable.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ConfigFile {
    /// What it extends.
    extends: Option<Extends>,
    /// Its compiler options.
    #[serde(rename = "compilerOptions")]
    compiler_options: Option<CompilerOptions>,
}

/// The configs an `extends` value names: one path, or a list of them.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Extends {
    /// One path.
    One(String),
    /// Several paths, applied in order.
    Several(Vec<String>),
}

/// The compiler options a config sets that decide how specifiers resolve.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CompilerOptions {
    /// `baseUrl`.
    #[serde(rename = "baseUrl")]
    base_url: Option<String>,
    /// `paths`.
    paths: Option<ListedPaths>,
}

/// A `paths` value as the config lists it: each pattern with its targets, in order, which a JSON
/// object read into a map would not keep.
#[derive(Debug, Default)]
struct ListedPaths(Vec<(String, Vec<String>)>);

impl<'de> Deserialize<'de> for ListedPaths {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ListedPathsVisitor)
    }
}

/// Reads a `paths` object entry by entry.
struct ListedPathsVisitor;

impl<'de> Visitor<'de> for ListedPathsVisitor {
    type Value = ListedPaths;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object of patterns, each with a list of paths")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<ListedPaths, A::Error> {
        let mut listed = Vec::new();
        while let Some(entry) = entries.next_entry::<String, Vec<String>>()? {
            listed.push(entry);
        }
        Ok(ListedPaths(listed))
    }
}

/// Reads every tsconfig.json and jsconfig.json with what each extends.
pub(super) fn load(files: &Files, read: ReadText<'_>) -> Box<dyn Resolver> {
    Box::new(Ecmascript {
        configs: settings_by_folder(files, read),
    })
}

/// What the config of each folder that holds one says; a folder's jsconfig.json counts only when
/// it has no tsconfig.json.
fn settings_by_folder(files: &Files, read: ReadText<'_>) -> BTreeMap<RepoPath, Settings> {
    let mut parsed = BTreeMap::new();
    let mut patterns = BTreeMap::new();
    let mut configs = BTreeMap::new();
    for name in CONFIG_NAMES {
        for config in files.named(name) {
            let Some(folder) = config.parent() else {
                continue;
            };
            if let Entry::Vacant(slot) = configs.entry(folder) {
                let mut chain = Chain {
                    files,
                    read,
                    parsed: &mut parsed,
                    patterns: &mut patterns,
                    steps: 0,
                    layers: Vec::new(),
                };
                slot.insert(chain.settings(config).unwrap_or_default());
            }
        }
    }
    configs
}

impl Resolver for Ecmascript {
    fn reads(&self, language: Language) -> bool {
        matches!(
            language,
            Language::TypeScript | Language::Tsx | Language::JavaScript
        )
    }

    fn targets(&self, files: &Files, importer: &RepoPath, import: &Import) -> Targets {
        let specifier = import.specifier.as_str();
        let found = if is_relative(specifier) {
            importer
                .parent()
                .and_then(|folder| joined(&folder, specifier))
                .and_then(|base| file_at(files, &base))
        } else {
            folders_above(importer)
                .find_map(|folder| self.configs.get(&folder))
                .and_then(|settings| settings.resolve(files, specifier))
        };
        Targets {
            module: found.into_iter().collect(),
            submodule: None,
        }
    }

    fn module_name(&self, _specifier: &str) -> Option<Identifier> {
        // A module imported whole, `import "./styles.css"`, is loaded for what loading it does; a
        // namespace import is always given its name.
        None
    }
}

impl Settings {
    /// The file `specifier`, which is not relative, names: by the `paths` pattern it matches,
    /// else from `baseUrl`.
    fn resolve(&self, files: &Files, specifier: &str) -> Option<RepoPath> {
        let by_pattern = self
            .paths
            .as_ref()
            .and_then(|paths| paths.resolve(files, specifier));
        by_pattern.or_else(|| file_at(files, &joined(self.base_url.as_ref()?, specifier)?))
    }
}

impl Patterns {
    /// The patterns `listed` sets. A pattern or a target with more than one `*` is left out, as
    /// TypeScript leaves it out.
    fn new(listed: &ListedPaths) -> Self {
        let mut exact = BTreeMap::new();
        let mut wildcard = Vec::new();
        for (key, targets) in &listed.0 {
            let targets: Vec<String> = targets
                .iter()
                .filter(|target| target.matches('*').count() <= 1)
                .cloned()
                .collect();
            match key.split_once('*') {
                None => {
                    exact.insert(key.clone(), targets);
                }
                Some((_, after)) if after.contains('*') => {}
                Some((prefix, suffix)) => wildcard.push(Pattern {
                    prefix: prefix.to_owned(),
                    suffix: suffix.to_owned(),
                    targets,
                }),
            }
        }
        Self { exact, wildcard }
    }
}

impl Paths {
    /// The file the first target of the pattern TypeScript picks for `specifier` reaches: a
    /// pattern without `*` equal to it, else the matching one with the longest prefix, the first
    /// listed when two are as long.
    fn resolve(&self, files: &Files, specifier: &str) -> Option<RepoPath> {
        let reach = |target: &str| file_at(files, &joined(&self.base, target)?);
        if let Some(targets) = self.patterns.exact.get(specifier) {
            return targets.iter().find_map(|target| reach(target));
        }
        let mut best: Option<(&Pattern, &str)> = None;
        for pattern in &self.patterns.wildcard {
            let star = specifier
                .strip_prefix(pattern.prefix.as_str())
                .and_then(|rest| rest.strip_suffix(pattern.suffix.as_str()));
            let is_longer =
                best.is_none_or(|(chosen, _)| pattern.prefix.len() > chosen.prefix.len());
            if let Some(star) = star.filter(|_| is_longer) {
                best = Some((pattern, star));
            }
        }
        let (pattern, star) = best?;
        pattern
            .targets
            .iter()
            .find_map(|target| reach(&target.replacen('*', star, 1)))
    }
}

/// Whether `specifier` starts from the importer's folder.
fn is_relative(specifier: &str) -> bool {
    matches!(specifier, "." | "..") || specifier.starts_with("./") || specifier.starts_with("../")
}

/// The file at `base` as TypeScript looks for it.
fn file_at(files: &Files, base: &RepoPath) -> Option<RepoPath> {
    candidates(base)
        .iter()
        .find_map(|candidate| files.at(candidate))
}

/// The paths TypeScript tries for `base`, in order.
fn candidates(base: &RepoPath) -> Vec<String> {
    let text = base.as_str();
    let mut tried = Vec::new();
    if !base.is_root() {
        if let Some(extension) = base.extension() {
            let replaced = REPLACED.iter().find(|(from, _)| *from == extension);
            let stem = text
                .strip_suffix(extension)
                .and_then(|rest| rest.strip_suffix('.'));
            match (replaced, stem) {
                (Some((_, replacements)), Some(stem)) => {
                    tried.extend(replacements.iter().map(|to| format!("{stem}{to}")));
                }
                _ => tried.push(text.to_owned()),
            }
        }
        tried.extend(APPENDED.iter().map(|to| format!("{text}{to}")));
    }
    let index = if base.is_root() {
        "index".to_owned()
    } else {
        format!("{text}/index")
    };
    tried.extend(APPENDED.iter().map(|to| format!("{index}{to}")));
    tried
}

/// The config files one config is built from, gathered bases first.
struct Chain<'a> {
    /// The repository's files.
    files: &'a Files,
    /// How a config file is read.
    read: ReadText<'a>,
    /// Every config file read so far, by path: what it says, or `None` when it cannot be read.
    parsed: &'a mut BTreeMap<RepoPath, Option<Rc<ConfigFile>>>,
    /// The patterns built so far, by the path of the config file that sets them.
    patterns: &'a mut BTreeMap<RepoPath, Arc<Patterns>>,
    /// How many files this chain has taken.
    steps: usize,
    /// Each file taken, with its path and folder, bases before the configs that extend them.
    layers: Vec<(RepoPath, RepoPath, Rc<ConfigFile>)>,
}

impl Chain<'_> {
    /// What the config at `config` says with what it extends applied; `None` when it or a file it
    /// extends cannot be read, the chain takes more than [`MAX_CONFIG_FILES`] files, or `baseUrl`
    /// leaves the repository.
    fn settings(&mut self, config: &RepoPath) -> Option<Settings> {
        self.add(config)?;
        let mut base_url = None;
        let mut paths = None;
        for (config, folder, file) in &self.layers {
            let Some(options) = &file.compiler_options else {
                continue;
            };
            if let Some(url) = &options.base_url {
                base_url = Some(joined(folder, url)?);
            }
            if let Some(listed) = &options.paths {
                paths = Some((config, folder, listed));
            }
        }
        let paths = paths.map(|(config, folder, listed)| {
            let patterns = self
                .patterns
                .entry(config.clone())
                .or_insert_with(|| Arc::new(Patterns::new(listed)));
            Paths {
                base: base_url.clone().unwrap_or_else(|| folder.clone()),
                patterns: Arc::clone(patterns),
            }
        });
        Some(Settings { base_url, paths })
    }

    /// Adds the config at `config` after everything it extends; `None` when a file cannot be read
    /// or followed, or one more would pass [`MAX_CONFIG_FILES`].
    fn add(&mut self, config: &RepoPath) -> Option<()> {
        if !self.files.contains(config) || self.steps >= MAX_CONFIG_FILES {
            return None;
        }
        self.steps += 1;
        let file = self.parse(config)?;
        let folder = config.parent()?;
        let bases: Vec<String> = match &file.extends {
            None => Vec::new(),
            Some(Extends::One(base)) => vec![base.clone()],
            Some(Extends::Several(bases)) => bases.clone(),
        };
        for base in &bases {
            match extended(self.files, &folder, base)? {
                Base::File(base) => self.add(&base)?,
                Base::Package => {}
            }
        }
        self.layers.push((config.clone(), folder, file));
        Some(())
    }

    /// What the config file at `config` says, read and parsed the first time it is asked for.
    fn parse(&mut self, config: &RepoPath) -> Option<Rc<ConfigFile>> {
        if let Some(known) = self.parsed.get(config) {
            return known.clone();
        }
        let file = (self.read)(config)
            .and_then(|text| parse(&text))
            .map(Rc::new);
        self.parsed.insert(config.clone(), file.clone());
        file
    }
}

/// What an `extends` entry names.
enum Base {
    /// The config file at this path, which the chain then reads; one that is not there cannot be
    /// read, and refuses the chain.
    File(RepoPath),
    /// A config from a package, which is left out.
    Package,
}

/// What an `extends` entry of a config in `folder` names; `None` when its path is rooted or leaves
/// the repository. As in TypeScript, a path without `.json` that names no file stands for the same
/// path with `.json` appended.
fn extended(files: &Files, folder: &RepoPath, base: &str) -> Option<Base> {
    if !base.starts_with('/') && !is_relative(base) {
        return Some(Base::Package);
    }
    let path = joined(folder, base)?;
    if files.contains(&path) || path.extension() == Some("json") {
        return Some(Base::File(path));
    }
    joined(folder, &format!("{base}.json")).map(Base::File)
}

/// Reads a config's text, a byte order mark, comments and trailing commas allowed; `None` when it
/// is not a JSON object, a setting read here has the wrong type, or its `paths` pass
/// [`MAX_WILDCARD_PATTERNS`] or [`MAX_TARGETS`].
fn parse(text: &str) -> Option<ConfigFile> {
    let config: ConfigFile = serde_json::from_str(&jsonc::strip(text)).ok()?;
    let listed = config
        .compiler_options
        .as_ref()
        .and_then(|options| options.paths.as_ref());
    let is_within_limits = listed.is_none_or(|listed| {
        let wildcards = listed.0.iter().filter(|(key, _)| key.contains('*')).count();
        let longest = listed.0.iter().map(|(_, targets)| targets.len()).max();
        wildcards <= MAX_WILDCARD_PATTERNS && longest.unwrap_or(0) <= MAX_TARGETS
    });
    is_within_limits.then_some(config)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::error::Error;

    use super::*;
    use crate::specifiers::Imported;
    use crate::test_support::{import, path};

    /// The file each of `specifiers` names when the file at `importer` imports it, in a
    /// repository of the files at `paths` and the config files `configs`, given by path and text.
    fn resolved(
        paths: &[&str],
        configs: &[(&str, &str)],
        importer: &str,
        specifiers: &[&str],
    ) -> Result<Vec<Option<String>>, Box<dyn Error>> {
        let texts: BTreeMap<RepoPath, String> = configs
            .iter()
            .map(|(at, text)| (path(at), (*text).to_owned()))
            .collect();
        let files = Files::new(paths.iter().map(|at| path(at)).chain(texts.keys().cloned()));
        let read = |at: &RepoPath| texts.get(at).cloned();
        let resolver = load(&files, &read);
        let mut found = Vec::new();
        for specifier in specifiers {
            let import = import(specifier, Imported::Module)?;
            let targets = resolver.targets(&files, &path(importer), &import);
            assert_eq!(targets.submodule, None, "{specifier}");
            assert!(targets.module.len() <= 1, "{specifier}: {targets:?}");
            found.push(targets.module.first().map(|at| at.as_str().to_owned()));
        }
        Ok(found)
    }

    /// Specifiers, each with the file it is expected to name.
    type Expected<'a> = [(&'a str, Option<&'a str>)];

    /// The expected file of each specifier, as `resolved` reports it.
    fn expected(cases: &Expected<'_>) -> Vec<Option<String>> {
        cases
            .iter()
            .map(|(_, file)| file.map(str::to_owned))
            .collect()
    }

    /// The specifiers of `cases`.
    fn specifiers<'c>(cases: &[(&'c str, Option<&str>)]) -> Vec<&'c str> {
        cases.iter().map(|(specifier, _)| *specifier).collect()
    }

    /// A relative specifier is tried as TypeScript tries it: a script extension replaced by its
    /// TypeScript twins first, any other extension as written, then the extensions appended, then
    /// the folder's index; it may not climb above the root. A bare specifier names no file
    /// without a config, even when a file of its name is beside the importer.
    #[test]
    fn relative_specifiers_are_tried_as_typescript_tries_them() -> Result<(), Box<dyn Error>> {
        let paths = [
            "src/app.ts",
            "src/index.ts",
            "src/util.ts",
            "src/compiled.ts",
            "src/compiled.js",
            "src/legacy.js",
            "src/widget.tsx",
            "src/lib/index.ts",
            "src/styles.css",
            "src/user.service.ts",
            "src/data.json",
            "src/types.d.ts",
            "src/esm.mts",
            "src/react.ts",
            "outside.ts",
        ];
        let cases = [
            ("./util", Some("src/util.ts")),
            ("./util.js", Some("src/util.ts")),
            ("./compiled.js", Some("src/compiled.ts")),
            ("./legacy", Some("src/legacy.js")),
            ("./legacy.js", Some("src/legacy.js")),
            ("./widget", Some("src/widget.tsx")),
            ("./widget.jsx", Some("src/widget.tsx")),
            ("./lib", Some("src/lib/index.ts")),
            ("./styles.css", Some("src/styles.css")),
            ("./user.service", Some("src/user.service.ts")),
            ("./data.json", Some("src/data.json")),
            ("./types", Some("src/types.d.ts")),
            ("./esm.mjs", Some("src/esm.mts")),
            (".", Some("src/index.ts")),
            ("../outside", Some("outside.ts")),
            ("../../escape", None),
            ("./missing", None),
            ("react", None),
            ("src/util", None),
        ];
        let found = resolved(&paths, &[], "src/app.ts", &specifiers(&cases))?;
        assert_eq!(found, expected(&cases));
        Ok(())
    }

    /// The pattern TypeScript picks maps a specifier: one equal to it, else the longest prefix,
    /// the first listed of two as long, its targets tried in order, before `baseUrl` even when
    /// `baseUrl` reaches a file too; when its targets name nothing, `baseUrl` is tried, and a
    /// specifier neither reaches names nothing. A byte order mark, comments and trailing commas
    /// are allowed.
    #[test]
    fn configs_map_specifiers_through_paths_then_base_url() -> Result<(), Box<dyn Error>> {
        let config = "\u{feff}{
  // where the app's imports start
  \"compilerOptions\": {
    \"baseUrl\": \".\",
    \"paths\": {
      \"@/*\": [\"src/*\"],
      \"@app/*\": [\"apps/web/*\", \"shared/*\",],
      \"*x\": [\"first/*\"],
      \"*\": [\"types/*\"],
      \"exact\": [\"src/exact-target.ts\"],
    },
  },
}";
        let paths = [
            "src/main.ts",
            "src/util.ts",
            "src/exact-target.ts",
            "apps/web/x.ts",
            "shared/x.ts",
            "shared/menu.ts",
            "types/anything.d.ts",
            "anything.ts",
            "types/box.d.ts",
            "first/bo.ts",
            "lib/tool.ts",
        ];
        let cases = [
            ("@/util", Some("src/util.ts")),
            ("@app/x", Some("apps/web/x.ts")),
            ("@app/menu", Some("shared/menu.ts")),
            ("exact", Some("src/exact-target.ts")),
            ("anything", Some("types/anything.d.ts")),
            ("box", Some("first/bo.ts")),
            ("lib/tool", Some("lib/tool.ts")),
            ("lodash", None),
        ];
        let configs = [("tsconfig.json", config)];
        let found = resolved(&paths, &configs, "src/main.ts", &specifiers(&cases))?;
        assert_eq!(found, expected(&cases));
        Ok(())
    }

    /// Each importer follows the nearest config above it. An extended config's `baseUrl` starts
    /// from that config's folder, and `paths` targets start from `baseUrl` when one is set, else
    /// from the folder of the config that sets them; `extends` may leave out `.json`, names a file
    /// of another extension as written, and of two configs it lists the later one wins. A
    /// folder's tsconfig.json comes before its jsconfig.json.
    #[test]
    fn each_importer_follows_its_nearest_config() -> Result<(), Box<dyn Error>> {
        let configs = [
            (
                "tsconfig.base.json",
                r#"{"compilerOptions": {"baseUrl": "./shared"}}"#,
            ),
            (
                "packages/ui/tsconfig.json",
                r##"{"extends": "../../tsconfig.base", "compilerOptions": {"paths": {"#/*": ["./src/*"]}}}"##,
            ),
            (
                "apps/web/jsconfig.json",
                r#"{"compilerOptions": {"paths": {"~/*": ["./lib/*"]}}}"#,
            ),
            (
                "apps/both/tsconfig.json",
                r#"{"compilerOptions": {"baseUrl": "ts"}}"#,
            ),
            (
                "apps/both/jsconfig.json",
                r#"{"compilerOptions": {"baseUrl": "js"}}"#,
            ),
            (
                "apps/list/tsconfig.json",
                r#"{"extends": ["./one.json", "./two.json"]}"#,
            ),
            (
                "apps/list/one.json",
                r#"{"compilerOptions": {"baseUrl": "one"}}"#,
            ),
            (
                "apps/list/two.json",
                r#"{"compilerOptions": {"baseUrl": "two"}}"#,
            ),
            ("apps/jsonc/tsconfig.json", r#"{"extends": "./base.jsonc"}"#),
            (
                "apps/jsonc/base.jsonc",
                r#"{"compilerOptions": {"baseUrl": "lib"}}"#,
            ),
        ];
        let paths = [
            "shared/tokens.ts",
            "shared/src/button.ts",
            "packages/ui/src/button.ts",
            "packages/ui/src/view.ts",
            "apps/web/lib/api.js",
            "apps/web/main.js",
            "apps/both/ts/x.ts",
            "apps/both/js/x.ts",
            "apps/both/main.ts",
            "apps/list/one/x.ts",
            "apps/list/two/x.ts",
            "apps/list/main.ts",
            "apps/jsonc/lib/x.ts",
            "apps/jsonc/main.ts",
        ];
        let cases: [(&str, &Expected<'_>); 5] = [
            (
                "packages/ui/src/view.ts",
                &[
                    ("#/button", Some("shared/src/button.ts")),
                    ("tokens", Some("shared/tokens.ts")),
                ],
            ),
            (
                "apps/web/main.js",
                &[("~/api", Some("apps/web/lib/api.js")), ("api", None)],
            ),
            ("apps/both/main.ts", &[("x", Some("apps/both/ts/x.ts"))]),
            ("apps/list/main.ts", &[("x", Some("apps/list/two/x.ts"))]),
            ("apps/jsonc/main.ts", &[("x", Some("apps/jsonc/lib/x.ts"))]),
        ];
        for (importer, cases) in cases {
            let found = resolved(&paths, &configs, importer, &specifiers(cases))?;
            assert_eq!(found, expected(cases), "{importer}");
        }
        Ok(())
    }

    /// A config that cannot be read or followed leaves only relative specifiers to resolve, its
    /// `paths` included: a syntax error, a loop of `extends`, an extended file that is missing or
    /// above the root, a `baseUrl` above the root, a setting of the wrong type, more wildcard patterns or targets
    /// than the limits allow. A config extended from a package is left out and the rest applies,
    /// and a pattern or a target with two `*` is left out alone.
    #[test]
    fn configs_that_cannot_be_followed_leave_relative_specifiers() -> Result<(), Box<dyn Error>> {
        let many: Vec<String> = (0..=MAX_WILDCARD_PATTERNS)
            .map(|at| format!("\"p{at}/*\": [\"./*\"]"))
            .collect();
        let many = format!(
            r#"{{"compilerOptions": {{"baseUrl": ".", "paths": {{{}}}}}}}"#,
            many.join(", ")
        );
        let wide: Vec<String> = (0..=MAX_TARGETS).map(|at| format!("\"t{at}/*\"")).collect();
        let wide = format!(
            r#"{{"compilerOptions": {{"baseUrl": ".", "paths": {{"*": [{}]}}}}}}"#,
            wide.join(", ")
        );
        let configs = [
            (
                "broken/tsconfig.json",
                r#"{"compilerOptions": {"baseUrl": "."#,
            ),
            (
                "loop/tsconfig.json",
                r#"{"extends": "./a.json", "compilerOptions": {"baseUrl": "."}}"#,
            ),
            ("loop/a.json", r#"{"extends": "./tsconfig.json"}"#),
            (
                "missing/tsconfig.json",
                r#"{"extends": "./gone.json", "compilerOptions": {"baseUrl": "."}}"#,
            ),
            (
                "climb/tsconfig.json",
                r#"{"extends": "../../outside.json", "compilerOptions": {"baseUrl": "."}}"#,
            ),
            (
                "above/tsconfig.json",
                r#"{"compilerOptions": {"baseUrl": "../..", "paths": {"*": ["./*"]}}}"#,
            ),
            (
                "typed/tsconfig.json",
                r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@/*": "src/*"}}}"#,
            ),
            ("many/tsconfig.json", &many),
            ("wide/tsconfig.json", &wide),
            (
                "package/tsconfig.json",
                r#"{"extends": "@tsconfig/node20/tsconfig.json", "compilerOptions": {"baseUrl": "."}}"#,
            ),
            (
                "stars/tsconfig.json",
                r#"{"compilerOptions": {"paths": {"x*y*": ["hit/*"], "@/*": ["b/*/*", "src/*"]}}}"#,
            ),
        ];
        let mut paths = vec!["stars/hit/A.ts", "stars/src/b.ts", "stars/b/b/*.ts"];
        let folders = [
            "broken", "loop", "missing", "climb", "above", "typed", "many", "wide", "package",
            "stars",
        ];
        let files: Vec<(String, String)> = folders
            .iter()
            .map(|folder| (format!("{folder}/main.ts"), format!("{folder}/x.ts")))
            .collect();
        for (main, x) in &files {
            paths.push(main);
            paths.push(x);
        }
        for folder in folders {
            let found = resolved(
                &paths,
                &configs,
                &format!("{folder}/main.ts"),
                &["./x", "x"],
            )?;
            let bare = (folder == "package").then(|| format!("{folder}/x.ts"));
            assert_eq!(found, [Some(format!("{folder}/x.ts")), bare], "{folder}");
        }
        let found = resolved(&paths, &configs, "stars/main.ts", &["xAy*", "@/b"])?;
        assert_eq!(found, [None, Some("stars/src/b.ts".to_owned())]);
        Ok(())
    }

    /// A config file that several configs extend is read once, and its `paths` are built once
    /// and shared.
    #[test]
    fn a_config_extended_by_many_is_read_and_built_once() -> Result<(), Box<dyn Error>> {
        let configs = [
            (
                "base.json",
                r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@/*": ["lib/*"]}}}"#,
            ),
            ("a/tsconfig.json", r#"{"extends": "../base.json"}"#),
            ("b/tsconfig.json", r#"{"extends": "../base.json"}"#),
        ];
        let texts: BTreeMap<RepoPath, String> = configs
            .iter()
            .map(|(at, text)| (path(at), (*text).to_owned()))
            .collect();
        let paths = ["x.ts", "a/main.ts", "b/main.ts"].map(path);
        let files = Files::new(paths.into_iter().chain(texts.keys().cloned()));
        let reads = Cell::new(0);
        let read = |at: &RepoPath| {
            reads.set(reads.get() + 1);
            texts.get(at).cloned()
        };
        let settings = settings_by_folder(&files, &read);
        assert_eq!(reads.get(), 3);
        let built = |folder: &str| {
            let paths = settings
                .get(&path(folder))
                .and_then(|settings| settings.paths.as_ref());
            paths.map(|paths| Arc::clone(&paths.patterns))
        };
        let (a, b) = (
            built("a").ok_or("no paths for a")?,
            built("b").ok_or("no paths for b")?,
        );
        assert!(Arc::ptr_eq(&a, &b));
        let resolver = Ecmascript { configs: settings };
        for importer in ["a/main.ts", "b/main.ts"] {
            let import = import("x", Imported::Module)?;
            let targets = resolver.targets(&files, &path(importer), &import);
            assert_eq!(targets.module, [path("x.ts")], "{importer}");
        }
        Ok(())
    }
}
