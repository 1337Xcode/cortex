//! Indexing chosen paths against the oracle of indexing everything: after any sequence of edits,
//! a store kept by `index_paths` over the changed paths, and a store kept by `index_repository`
//! after each edit, both hold exactly what a store built from scratch holds, row for row (ids
//! aside, which depend on the order rows were written): files, definitions, references with their
//! targets, and every edge of the graph.

mod common;

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use common::{NOW_MS, TestResult, grammars};
use cortex_core::{EdgeType, ModeSetting, RepoId, RepoPath, Resolution};
use cortex_index::{DiscoverOptions, IndexOptions, ScipOutcome, index_paths, index_repository};
use cortex_store::{NodeId, Queries, Reader, SymbolId, Writer, WriterOptions};
use proptest::prelude::*;

/// The repository every case starts from: Python with a package that passes a name on and a test,
/// TypeScript with an index file that passes names on, by name and by `*`, a chain of Python
/// files passing `x` on, as deep as a search for a name follows (`chain_user.py` takes `x` from
/// `c1`, which takes it from `c2`, and so on to `c5`, which defines it), and `worker.py`, which
/// calls names after the module it imports, one that module defines and one it does not.
const FIXTURE: &[(&str, &str)] = &[
    ("chain/__init__.py", ""),
    ("chain/c1.py", "from chain.c2 import x\n"),
    ("chain/c2.py", "from chain.c3 import x\n"),
    ("chain/c3.py", "from chain.c4 import x\n"),
    ("chain/c4.py", "from chain.c5 import x\n"),
    ("chain/c5.py", "def x():\n    pass\n"),
    ("chain/c6.py", "def x():\n    pass\n"),
    ("chain_user.py", "from chain.c1 import x\n\nx()\n"),
    ("pkg/__init__.py", "from pkg.core import helper\n"),
    (
        "pkg/core.py",
        "def helper():\n    pass\n\n\nclass Model:\n    def run(self):\n        helper()\n",
    ),
    (
        "app.py",
        "from pkg import helper\nfrom pkg.core import Model\n\n\ndef main():\n    helper()\n    Model().run()\n",
    ),
    ("util.py", "def tool():\n    pass\n"),
    (
        "tests/test_core.py",
        "from pkg.core import helper\n\n\ndef test_helper():\n    helper()\n",
    ),
    (
        "web/index.ts",
        "export { greet } from \"./greet\";\nexport * from \"./more\";\n",
    ),
    ("web/greet.ts", "export function greet() {}\n"),
    ("web/more.ts", "export function more() {}\n"),
    (
        "web/app.ts",
        "import { greet, more } from \"./index\";\ngreet();\nmore();\n",
    ),
    ("worker.py", "import util\n\nutil.tool()\nutil.more()\n"),
];

/// Names edits define and call: some already defined, so that definitions come and go under names
/// in use and references find one definition, several or none.
const NAMES: [&str; 7] = ["helper", "tool", "greet", "more", "fresh", "x", "run"];

/// One edit of the repository.
#[derive(Debug, Clone)]
enum Edit {
    /// Adds a definition of the name to the file.
    Define(usize, usize),
    /// Adds a call of the name to the file.
    Call(usize, usize),
    /// Gives the file its first content back.
    Restore(usize),
    /// Removes the file.
    Remove(usize),
    /// Adds a new file that defines a name and calls another.
    AddFile(usize, usize),
    /// Removes the `web` folder whole, as a checkout can.
    RemoveFolder,
    /// Adds to the first file an import of the name from the second, when both are Python.
    Import(usize, usize, usize),
}

/// Any edit; files and names are picked by index, modulo what there is.
fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        3 => (0..16_usize, 0..NAMES.len()).prop_map(|(file, name)| Edit::Define(file, name)),
        3 => (0..16_usize, 0..NAMES.len()).prop_map(|(file, name)| Edit::Call(file, name)),
        1 => (0..16_usize).prop_map(Edit::Restore),
        1 => (0..16_usize).prop_map(Edit::Remove),
        2 => (0..NAMES.len(), 0..NAMES.len()).prop_map(|(define, call)| Edit::AddFile(define, call)),
        1 => Just(Edit::RemoveFolder),
        2 => (0..24_usize, 0..24_usize, 0..NAMES.len())
            .prop_map(|(file, from, name)| Edit::Import(file, from, name)),
    ]
}

/// The repository's files as the edits leave them, written to disk with a newer time on every
/// write so that each change is seen.
struct Tree<'a> {
    /// The repository's root.
    root: &'a Path,
    /// Each file's content, by path.
    files: BTreeMap<String, String>,
    /// How many files were written, for their times.
    writes: u64,
    /// How many files were added, for their names.
    added: usize,
}

impl Tree<'_> {
    /// Writes `content` to the file at `path`, one second newer than the last write.
    fn write(&mut self, path: &str, content: String) -> TestResult {
        let native = self.root.join(path);
        if let Some(folder) = native.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&native, &content)?;
        self.writes += 1;
        let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000 + self.writes);
        std::fs::File::options()
            .write(true)
            .open(&native)?
            .set_modified(time)?;
        self.files.insert(path.to_owned(), content);
        Ok(())
    }

    /// The path of the file `index` picks, if any file is left.
    fn pick(&self, index: usize) -> Option<String> {
        let count = self.files.len();
        (count > 0).then(|| self.files.keys().nth(index % count).cloned())?
    }

    /// Applies `edit`; returns the paths it changed, as the watcher would report them.
    fn apply(&mut self, edit: &Edit) -> TestResult<Vec<String>> {
        match *edit {
            Edit::Define(file, name) | Edit::Call(file, name) => {
                let Some(path) = self.pick(file) else {
                    return Ok(Vec::new());
                };
                let name = NAMES.get(name).copied().unwrap_or("fresh");
                let is_python = path.ends_with(".py");
                let line = match (edit, is_python) {
                    (Edit::Define(..), true) => format!("\n\ndef {name}():\n    pass\n"),
                    (Edit::Define(..), false) => format!("\nexport function {name}() {{}}\n"),
                    (_, true) => format!("\n{name}()\n"),
                    (_, false) => format!("\n{name}();\n"),
                };
                let old = self.files.get(&path).cloned().unwrap_or_default();
                let content = format!("{old}{line}");
                self.write(&path, content)?;
                Ok(vec![path])
            }
            Edit::Restore(file) => {
                let Some(path) = self.pick(file) else {
                    return Ok(Vec::new());
                };
                let first = FIXTURE.iter().find(|(fixture, _)| *fixture == path);
                let content = first.map_or_else(String::new, |(_, content)| (*content).to_owned());
                self.write(&path, content)?;
                Ok(vec![path])
            }
            Edit::Remove(file) => {
                let Some(path) = self.pick(file) else {
                    return Ok(Vec::new());
                };
                std::fs::remove_file(self.root.join(&path))?;
                self.files.remove(&path);
                Ok(vec![path])
            }
            Edit::AddFile(define, call) => {
                self.added += 1;
                let path = format!("extra{}.py", self.added);
                let content = format!(
                    "from util import tool\n\n\ndef {}():\n    {}()\n",
                    NAMES.get(define).copied().unwrap_or("fresh"),
                    NAMES.get(call).copied().unwrap_or("fresh")
                );
                self.write(&path, content)?;
                Ok(vec![path])
            }
            Edit::Import(file, from, name) => {
                let (Some(path), Some(source)) = (self.pick(file), self.pick(from)) else {
                    return Ok(Vec::new());
                };
                let (Some(module), true) = (source.strip_suffix(".py"), path.ends_with(".py"))
                else {
                    return Ok(Vec::new());
                };
                let name = NAMES.get(name).copied().unwrap_or("fresh");
                let old = self.files.get(&path).cloned().unwrap_or_default();
                let module = module.trim_end_matches("/__init__").replace('/', ".");
                self.write(&path, format!("from {module} import {name}\n{old}"))?;
                Ok(vec![path])
            }
            Edit::RemoveFolder => {
                let had_files = self.files.keys().any(|path| path.starts_with("web/"));
                if !had_files {
                    return Ok(Vec::new());
                }
                std::fs::remove_dir_all(self.root.join("web"))?;
                self.files.retain(|path, _| !path.starts_with("web/"));
                Ok(vec!["web".to_owned()])
            }
        }
    }
}

/// Opens the writer of a store in `folder` for the repository at `root`.
fn writer(folder: &Path, root: &Path) -> TestResult<Writer> {
    Ok(Writer::open(&WriterOptions {
        folder,
        repo_id: &RepoId::of(root),
        mmap_bytes: 0,
        now_ms: NOW_MS,
    })?)
}

/// The options of every index here: one thread, generous time, the mode left to the size.
fn options() -> IndexOptions<'static> {
    IndexOptions {
        discover: DiscoverOptions {
            max_file_bytes: 1_048_576,
            extra_ignores: &[],
            global_gitignore: None,
        },
        parse_timeout: Duration::from_secs(60),
        threads: NonZeroUsize::MIN,
        now_ms: NOW_MS,
        mode: ModeSetting::Auto,
    }
}

/// A node, by what it stands for rather than its id.
type NodeKey = (String, String);

/// An edge: its source, target, type, weight and count.
type Edge = (NodeKey, NodeKey, EdgeType, String, u32);

/// A store's rows without their ids: files, definitions, references and edges.
#[derive(Debug, PartialEq)]
struct Snapshot {
    /// Each file: path, language, size, lines, hash, time and flags.
    files: Vec<String>,
    /// Each definition: file, qualified name, kind and lines.
    symbols: Vec<String>,
    /// Each reference: file, line, name, kind, the definition it sits in, and its target with how
    /// it was found.
    refs: Vec<String>,
    /// Each edge: its source, target, type, weight and count.
    edges: Vec<Edge>,
}

/// What the store in `folder` holds.
fn snapshot(folder: &Path) -> TestResult<Snapshot> {
    let reader = Reader::open(folder, 0)?;
    let queries = reader.queries();
    let mut files = Vec::new();
    let mut symbols = Vec::new();
    let mut refs = Vec::new();
    let mut names: BTreeMap<SymbolId, String> = BTreeMap::new();
    let rows = queries.file_rows()?;
    for row in &rows {
        for symbol in queries.symbols_in_file(row.id)? {
            let key = format!(
                "{}#{}:{:?}",
                row.file.path,
                symbol.qual_name.as_str(),
                symbol.kind
            );
            symbols.push(format!("{key} {}-{}", symbol.start_line, symbol.end_line));
            names.insert(symbol.id, key);
        }
    }
    for row in &rows {
        let file = &row.file;
        files.push(format!(
            "{} {:?} {} {:?} {:?} {} {}",
            file.path,
            file.language,
            file.size_bytes,
            file.line_count,
            file.content_hash,
            file.mtime_ns,
            file.flags.bits()
        ));
        for reference in queries.refs_in_file(row.id)? {
            let target = reference.target.map(|target| {
                let name = names.get(&target.symbol).cloned().unwrap_or_default();
                (name, resolution_name(target.resolution))
            });
            let source = reference
                .symbol
                .and_then(|symbol| names.get(&symbol).cloned());
            let through = if reference.goes_through_import {
                " through an import"
            } else {
                ""
            };
            refs.push(format!(
                "{}:{} {} {:?} in {source:?} -> {target:?}{through}",
                file.path,
                reference.line,
                reference.dst_name.as_str(),
                reference.kind
            ));
        }
    }
    Ok(Snapshot {
        files,
        symbols,
        refs,
        edges: edges(queries, &names)?,
    })
}

/// How a target was found, by name.
fn resolution_name(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::Scip => "scip",
        Resolution::ImportPath => "import_path",
        Resolution::SameFile => "same_file",
        Resolution::NameOnly => "name_only",
    }
}

/// Every edge of the graph, each end by what its node stands for, in order.
fn edges(queries: Queries<'_>, names: &BTreeMap<SymbolId, String>) -> TestResult<Vec<Edge>> {
    let mut keys: BTreeMap<NodeId, NodeKey> = BTreeMap::new();
    for (path, node) in queries.file_nodes()? {
        keys.insert(node, ("file".to_owned(), path.to_string()));
    }
    for (path, node) in queries.folder_nodes()? {
        keys.insert(node, ("folder".to_owned(), path.to_string()));
    }
    for (symbol, node) in queries.symbol_nodes()? {
        let name = names.get(&symbol).cloned().unwrap_or_default();
        keys.insert(node, ("symbol".to_owned(), name));
    }
    let mut edges = Vec::new();
    for (node, key) in &keys {
        for row in queries.edges_from(*node)? {
            let target = keys.get(&row.edge.dst).cloned().unwrap_or_default();
            let weight = format!("{:.3}", row.edge.weight);
            edges.push((key.clone(), target, row.edge_type, weight, row.edge.count));
        }
    }
    edges.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
    Ok(edges)
}

/// The rows only one of `by_paths` and `whole` holds, each list in turn.
fn differences(by_paths: &Snapshot, whole: &Snapshot) -> String {
    let lists = [
        ("files", by_paths.files.clone(), whole.files.clone()),
        ("symbols", by_paths.symbols.clone(), whole.symbols.clone()),
        ("refs", by_paths.refs.clone(), whole.refs.clone()),
        (
            "edges",
            by_paths
                .edges
                .iter()
                .map(|edge| format!("{edge:?}"))
                .collect(),
            whole.edges.iter().map(|edge| format!("{edge:?}")).collect(),
        ),
    ];
    let mut text = String::new();
    for (list, left, right) in lists {
        for row in left.iter().filter(|row| !right.contains(row)) {
            text.push_str(&format!(
                "{list} only by paths: {row}
"
            ));
        }
        for row in right.iter().filter(|row| !left.contains(row)) {
            text.push_str(&format!(
                "{list} only whole: {row}
"
            ));
        }
    }
    text
}

/// Applies `edits` to a fresh copy of the fixture, keeping one store by `index_paths` over each
/// edit's paths and one by `index_repository`, and checks after each edit that they hold the same.
fn check_edits(edits: &[Edit]) -> TestResult {
    let scratch = tempfile::tempdir()?;
    let root = scratch.path().join("repo");
    std::fs::create_dir(&root)?;
    let mut tree = Tree {
        root: &root,
        files: BTreeMap::new(),
        writes: 0,
        added: 0,
    };
    for (path, content) in FIXTURE {
        tree.write(path, (*content).to_owned())?;
    }
    let grammars = grammars()?;
    let (paths_folder, whole_folder) = (scratch.path().join("paths"), scratch.path().join("whole"));
    let mut by_paths = writer(&paths_folder, &root)?;
    let mut whole = writer(&whole_folder, &root)?;
    index_repository(&root, &grammars, &mut by_paths, &options())?;
    index_repository(&root, &grammars, &mut whole, &options())?;
    for (step, edit) in edits.iter().enumerate() {
        let changed: Vec<RepoPath> = tree
            .apply(edit)?
            .iter()
            .map(|path| RepoPath::new(path))
            .collect::<Result<_, _>>()?;
        index_paths(&root, &grammars, &mut by_paths, &options(), &changed)?;
        index_repository(&root, &grammars, &mut whole, &options())?;
        let fresh_folder = scratch.path().join(format!("fresh{step}"));
        index_repository(
            &root,
            &grammars,
            &mut writer(&fresh_folder, &root)?,
            &options(),
        )?;
        let fresh = snapshot(&fresh_folder)?;
        for (name, folder) in [("by paths", &paths_folder), ("whole", &whole_folder)] {
            let kept = snapshot(folder)?;
            if kept != fresh {
                return Err(format!(
                    "after edit {step} of {edits:?}, kept {name} against fresh:\n{}",
                    differences(&kept, &fresh)
                )
                .into());
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Any sequence of up to six edits leaves the two stores alike.
    #[test]
    fn paths_index_as_everything_does(edits in prop::collection::vec(edit(), 1..7)) {
        check_edits(&edits).map_err(|error| TestCaseError::fail(error.to_string()))?;
    }
}

/// Writes `files` to a fresh repository and indexes it, renames the file at `from` to `to` and
/// indexes both paths, as the watcher reports a rename, and checks that the store holds what a
/// store built from scratch holds.
fn check_rename(files: &[(&str, &str)], (from, to): (&str, &str)) -> TestResult {
    let scratch = tempfile::tempdir()?;
    let root = scratch.path().join("repo");
    std::fs::create_dir(&root)?;
    for (path, content) in files {
        std::fs::write(root.join(path), content)?;
    }
    let grammars = grammars()?;
    let kept_folder = scratch.path().join("kept");
    let mut kept = writer(&kept_folder, &root)?;
    index_repository(&root, &grammars, &mut kept, &options())?;
    std::fs::rename(root.join(from), root.join(to))?;
    let renamed = [RepoPath::new(from)?, RepoPath::new(to)?];
    index_paths(&root, &grammars, &mut kept, &options(), &renamed)?;
    let fresh_folder = scratch.path().join("fresh");
    index_repository(
        &root,
        &grammars,
        &mut writer(&fresh_folder, &root)?,
        &options(),
    )?;
    let (left, right) = (snapshot(&kept_folder)?, snapshot(&fresh_folder)?);
    assert!(left == right, "{}", differences(&left, &right));
    Ok(())
}

/// A file of rules in a folder the walk leaves out governs nothing the walk finds: indexing its path
/// indexes that path alone, which leaves the SCIP index unread. A file of rules the walk reads, at
/// the root or the repository's own under `.git/info`, makes the whole repository indexed again,
/// which looks for the SCIP index and finds none.
#[test]
fn rules_in_an_ignored_folder_index_only_their_path() -> TestResult {
    let scratch = tempfile::tempdir()?;
    let root = scratch.path().join("repo");
    std::fs::create_dir_all(root.join("node_modules/pkg"))?;
    std::fs::write(root.join(".gitignore"), "node_modules/\n")?;
    std::fs::write(root.join("node_modules/pkg/index.js"), "x\n")?;
    std::fs::write(root.join("app.py"), "app = 1\n")?;
    let grammars = grammars()?;
    let mut kept = writer(&scratch.path().join("kept"), &root)?;
    index_repository(&root, &grammars, &mut kept, &options())?;
    std::fs::write(root.join("node_modules/pkg/.gitignore"), "dist/\n")?;
    let ignored = [RepoPath::new("node_modules/pkg/.gitignore")?];
    let report = index_paths(&root, &grammars, &mut kept, &options(), &ignored)?;
    assert_eq!(report.scip, ScipOutcome::NotRead);
    std::fs::write(root.join(".gitignore"), "node_modules/\ndist/\n")?;
    let read = [RepoPath::new(".gitignore")?];
    let report = index_paths(&root, &grammars, &mut kept, &options(), &read)?;
    assert_eq!(report.scip, ScipOutcome::Absent);
    std::fs::create_dir_all(root.join(".git/info"))?;
    std::fs::write(
        root.join(".git/info/exclude"),
        "app.py
",
    )?;
    let own = [RepoPath::new(".git/info/exclude")?];
    let report = index_paths(&root, &grammars, &mut kept, &options(), &own)?;
    assert_eq!(report.scip, ScipOutcome::Absent);
    Ok(())
}

/// A file renamed in case alone, as the watcher reports it with both spellings, is stored under its
/// new name only, as a fresh index stores it, whether or not the file system tells the two apart.
#[test]
fn a_file_renamed_in_case_is_stored_once() -> TestResult {
    check_rename(
        &[("Foo.py", "def foo():\n    pass\n"), ("app.py", "foo()\n")],
        ("Foo.py", "foo.py"),
    )
}

/// The one definition of `tool` moves to a file of another name: the name still has one
/// definition, but a call that found it by name alone must now find it at the new path.
#[test]
fn a_definition_moved_to_another_file_is_found_there() -> TestResult {
    check_rename(
        &[
            ("util.py", "def tool():\n    pass\n"),
            ("app.py", "tool()\n"),
        ],
        ("util.py", "tools.py"),
    )
}

/// `util.py` comes to define `more` and then no longer does: `worker.py`'s call of `more` after
/// the module `util` is tied to that definition and untied again, though `worker.py` is not edited.
#[test]
fn a_name_after_an_imported_module_indexes_as_everything_does() -> TestResult {
    // util.py sorts 12th, before web/ and worker.py; name 3 is `more`.
    check_edits(&[Edit::Define(12, 3), Edit::Restore(12)])
}

/// `app.py` calls `tool`, which it does not import, while the name's definitions go from one (in
/// `util.py`) to two, three, two, one and none. The call finds the one definition, and nothing
/// while there are several or none: the counts from one to two, two to one and one to none change
/// what it finds, though `app.py` is not edited; two to three and three to two do not.
#[test]
fn a_name_defined_once_then_several_times_indexes_as_everything_does() -> TestResult {
    // Files sort as: app.py 0, ... chain/c6.py 7, chain_user.py 8, ... util.py 12. Name 1 is
    // `tool`.
    check_edits(&[
        Edit::Call(0, 1),
        Edit::Define(8, 1),
        Edit::Define(7, 1),
        Edit::Restore(8),
        Edit::Restore(7),
        Edit::Remove(12),
    ])
}

/// `c5` comes to import `x` from `c6` as well as define it, so `x` is ambiguous there, five files
/// from `chain_user.py`: its tie to `c5` must go, and stay gone when `chain_user.py` is edited.
#[test]
fn a_name_passed_on_five_deep_indexes_as_everything_does() -> TestResult {
    // Files sort as: app.py 0, chain/__init__.py 1, chain/c1.py 2, ... chain/c5.py 6,
    // chain/c6.py 7, chain_user.py 8.
    check_edits(&[Edit::Import(6, 7, 5), Edit::Call(8, 5)])
}

/// A file restored, a name defined where it was not, and a file removed: the stores stay alike,
/// and the hand-picked sequence keeps running whatever the random cases draw.
#[test]
fn chosen_edits_index_as_everything_does() -> TestResult {
    check_edits(&[
        Edit::Define(0, 1),
        Edit::Call(2, 4),
        Edit::Define(5, 4),
        Edit::AddFile(4, 0),
        Edit::Remove(3),
        Edit::RemoveFolder,
        Edit::Restore(0),
    ])
}
