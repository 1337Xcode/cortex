//! A repository's SCIP index, built in each test with the `scip` crate's own types, read by a
//! full index: references tied at full confidence to the definitions the index names.

mod common;

use std::fs::File;
use std::path::Path;
use std::time::{Duration, SystemTime};

use common::{TestResult, expected, index, index_changed, ties, write_all};
use cortex_index::{ScipOutcome, ScipProblem};
use protobuf::Message;
use scip::types::{Document, Index, MultiLineRange, Occurrence, SingleLineRange};

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// The symbol a test's indexer gives the definition `descriptors`, in the package `shop`.
fn symbol(descriptors: &str) -> String {
    format!("scip-python python shop 0.1 {descriptors}")
}

/// An occurrence of `symbol` on the 0-based `line`, a definition when `is_definition`, its range
/// written in the packed form.
fn packed(symbol: &str, line: i32, is_definition: bool) -> Occurrence {
    let mut occurrence = Occurrence::new();
    occurrence.symbol = symbol.to_owned();
    occurrence.range = vec![line, 4, 8];
    occurrence.symbol_roles = i32::from(is_definition);
    occurrence
}

/// A document for the file at `path` with `occurrences`.
fn document(path: &str, occurrences: Vec<Occurrence>) -> Document {
    let mut document = Document::new();
    document.relative_path = path.to_owned();
    document.occurrences = occurrences;
    document
}

/// Writes `documents` as the SCIP index of the repository at `root`.
fn write_index(root: &Path, documents: Vec<Document>) -> TestResult {
    let mut index = Index::new();
    index.documents = documents;
    std::fs::write(root.join("index.scip"), index.write_to_bytes()?)?;
    Ok(())
}

/// Sets the modification time of the files at `paths` of the repository at `root` an hour back,
/// before the SCIP index was written.
fn backdate(root: &Path, paths: &[&str]) -> TestResult {
    let earlier = SystemTime::now() - Duration::from_secs(3600);
    for path in paths {
        File::options()
            .write(true)
            .open(root.join(path))?
            .set_modified(earlier)?;
    }
    Ok(())
}

/// The index ties what the rules here cannot, and ties what they can more surely. `lib.helper()`
/// (line 5 of `app.py`) is tied through `import lib` too, and the index makes it `scip`; `run()`
/// (line 6) has a definition in `tools.py` and in `jobs.py`, too many for its name alone, and the
/// index says `jobs.py`. Each definition's range takes another form: packed, a single line, lines.
/// `late.py` changed after the index was written, so what the index says of its `run()` is left
/// out and the call stays unresolved; `missing.py` is no file of the repository. Both documents
/// count as left out.
#[test]
fn the_index_ties_references_at_full_confidence() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("lib.py", "def helper():\n    pass\n"),
            ("tools.py", "def run():\n    pass\n"),
            ("jobs.py", "def run():\n    pass\n"),
            (
                "app.py",
                "import lib\n\n\ndef main():\n    lib.helper()\n    run()\n",
            ),
            ("late.py", "def later():\n    run()\n"),
        ],
    )?;
    let helper = symbol("lib/helper().");
    let jobs_run = symbol("jobs/run().");
    let mut single = packed(&symbol("tools/run()."), 0, true);
    single.range.clear();
    let mut line = SingleLineRange::new();
    line.line = 0;
    single.set_single_line_range(line);
    let mut multi = packed(&jobs_run, 0, true);
    multi.range.clear();
    let mut lines = MultiLineRange::new();
    lines.start_line = 0;
    lines.end_line = 1;
    multi.set_multi_line_range(lines);
    write_index(
        root,
        vec![
            document("lib.py", vec![packed(&helper, 0, true)]),
            document("tools.py", vec![single]),
            document("jobs.py", vec![multi]),
            document(
                "app.py",
                vec![packed(&helper, 4, false), packed(&jobs_run, 5, false)],
            ),
            document("late.py", vec![packed(&jobs_run, 1, false)]),
            document("missing.py", vec![packed(&symbol("missing/x()."), 0, true)]),
        ],
    )?;
    let later = SystemTime::now() + Duration::from_secs(3600);
    File::options()
        .write(true)
        .open(root.join("late.py"))?
        .set_modified(later)?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        report.scip,
        ScipOutcome::Read {
            documents: 6,
            left_out: 2,
            tied: 2
        }
    );
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("call", "helper", 5, Some(("lib.py", "helper", "scip"))),
            ("call", "run", 6, Some(("jobs.py", "run", "scip"))),
        ])
    );
    assert_eq!(
        ties(root, "late.py")?,
        expected(&[("call", "run", 2, None)])
    );
    Ok(())
}

/// What the index cannot settle one for one is left to the rules here: a symbol defined twice in
/// the index, a symbol local to a document, and a line where one name stands for two symbols.
/// `twice()` (line 3 of `app.py`) is defined in `a.py` and `b.py` by the index's own account; the
/// local symbol on line 2 and the two symbols named `go` on line 5, `b.py`'s and `c.py`'s, tie
/// nothing. The calls go by their names: `run` to its one definition (0.3), `twice` and `go` to
/// none, each having two.
#[test]
fn what_the_index_cannot_settle_is_left_to_the_rules() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("a.py", "def twice():\n    pass\n"),
            ("b.py", "def twice():\n    pass\n\n\ndef go():\n    pass\n"),
            ("c.py", "def run():\n    pass\n\n\ndef go():\n    pass\n"),
            (
                "app.py",
                "def main():\n    run()\n    twice()\n\n    go()\n",
            ),
        ],
    )?;
    let twice = symbol("twice().");
    let go_b = symbol("b/go().");
    let go_c = symbol("c/go().");
    write_index(
        root,
        vec![
            document("a.py", vec![packed(&twice, 0, true)]),
            document(
                "b.py",
                vec![packed(&twice, 0, true), packed(&go_b, 4, true)],
            ),
            document("c.py", vec![packed(&go_c, 4, true)]),
            document(
                "app.py",
                vec![
                    packed("local 1", 1, false),
                    packed(&twice, 2, false),
                    packed(&go_b, 4, false),
                    packed(&go_c, 4, false),
                ],
            ),
        ],
    )?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        report.scip,
        ScipOutcome::Read {
            documents: 4,
            left_out: 0,
            tied: 0
        }
    );
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("call", "run", 2, Some(("c.py", "run", "name_only"))),
            ("call", "twice", 3, None),
            ("call", "go", 5, None),
        ])
    );
    Ok(())
}

/// An index that is no SCIP index is refused and reported, and the rules tie as they would
/// without it; so is a folder in its place.
#[test]
fn an_index_that_cannot_be_used_is_refused() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("lib.py", "def helper():\n    pass\n"),
            ("app.py", "def main():\n    helper()\n"),
        ],
    )?;
    std::fs::write(root.join("index.scip"), [0xff, 0xff, 0xff, 0xff])?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(report.scip, ScipOutcome::Refused(ScipProblem::NotAnIndex));
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[("call", "helper", 2, Some(("lib.py", "helper", "name_only")))])
    );
    std::fs::remove_file(root.join("index.scip"))?;
    std::fs::create_dir(root.join("index.scip"))?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(report.scip, ScipOutcome::Refused(ScipProblem::NotAFile));
    Ok(())
}

/// A line the index does not settle one for one ties nothing from it. Line 6 of `app.py` calls
/// `ext.run`, which the index does not know, and `lib.run`, which it does: one occurrence of `run`
/// against two stored references, so the rules tie them, `lib.run` through its import and
/// `ext.run` to nothing. In `m.py` the index says line 2 calls the parameter `run`, a definition
/// Cortex does not store, so the call is left to the rules, which find the function of that name
/// in the file.
#[test]
fn lines_the_index_does_not_settle_tie_nothing() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("lib.py", "def run():\n    pass\n"),
            (
                "app.py",
                "import lib\nimport ext\n\n\ndef main():\n    x = ext.run(lib.run())\n",
            ),
            ("m.py", "def run(run):\n    run()\n"),
        ],
    )?;
    let lib_run = symbol("lib/run().");
    let parameter = "scip-java maven shop 0.1 m/run().(run)";
    write_index(
        root,
        vec![
            document("lib.py", vec![packed(&lib_run, 0, true)]),
            document("app.py", vec![packed(&lib_run, 5, false)]),
            document(
                "m.py",
                vec![
                    packed(&symbol("m/run()."), 0, true),
                    packed(parameter, 0, true),
                    packed(parameter, 1, false),
                ],
            ),
        ],
    )?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        report.scip,
        ScipOutcome::Read {
            documents: 3,
            left_out: 0,
            tied: 0
        }
    );
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("call", "run", 6, None),
            ("call", "run", 6, Some(("lib.py", "run", "import_path"))),
        ])
    );
    assert_eq!(
        ties(root, "m.py")?,
        expected(&[("call", "run", 2, Some(("m.py", "run", "same_file")))])
    );
    Ok(())
}

/// An edit to a file the SCIP index describes makes its word on that file out of date: indexing
/// the edited file's path drops the ties the index gave into it, as a full index drops them. The
/// call `run()` in `app.py` was tied to `jobs.py` by the index; once `jobs.py` changes, `run` has
/// two definitions in reach and stays unresolved, while `lib.helper()` keeps its tie.
#[test]
fn an_edit_drops_the_index_s_ties_into_the_edited_file() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("lib.py", "def helper():\n    pass\n"),
            ("tools.py", "def run():\n    pass\n"),
            ("jobs.py", "def run():\n    pass\n"),
            (
                "app.py",
                "import lib\n\n\ndef main():\n    lib.helper()\n    run()\n",
            ),
        ],
    )?;
    let helper = symbol("lib/helper().");
    let jobs_run = symbol("jobs/run().");
    write_index(
        root,
        vec![
            document("lib.py", vec![packed(&helper, 0, true)]),
            document("jobs.py", vec![packed(&jobs_run, 0, true)]),
            document(
                "app.py",
                vec![packed(&helper, 4, false), packed(&jobs_run, 5, false)],
            ),
        ],
    )?;
    backdate(root, &["lib.py", "tools.py", "jobs.py", "app.py"])?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("call", "helper", 5, Some(("lib.py", "helper", "scip"))),
            ("call", "run", 6, Some(("jobs.py", "run", "scip"))),
        ])
    );
    std::fs::write(root.join("jobs.py"), "def run():\n    return 1\n")?;
    index_changed(root, &["jobs.py"])?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("call", "helper", 5, Some(("lib.py", "helper", "scip"))),
            ("call", "run", 6, None),
        ])
    );
    Ok(())
}

/// Writes a repository whose SCIP index ties every reference of `app.py`, indexes it whole and
/// checks the ties. `app.py` takes `run` from `jobs.py` by an import (line 2) and calls it (line 5),
/// and calls `helper` after the module `lib` it imports (line 4); without the index, each would be
/// tied through the imports, as `import_path`.
fn index_imports_the_index_ties(root: &Path) -> TestResult {
    write_all(
        root,
        &[
            ("lib.py", "def helper():\n    pass\n"),
            ("jobs.py", "def run():\n    pass\n"),
            (
                "app.py",
                "import lib\nfrom jobs import run\n\nlib.helper()\nrun()\n",
            ),
        ],
    )?;
    let helper = symbol("lib/helper().");
    let run = symbol("jobs/run().");
    write_index(
        root,
        vec![
            document("lib.py", vec![packed(&helper, 0, true)]),
            document("jobs.py", vec![packed(&run, 0, true)]),
            document(
                "app.py",
                vec![
                    packed(&run, 1, false),
                    packed(&helper, 3, false),
                    packed(&run, 4, false),
                ],
            ),
        ],
    )?;
    backdate(root, &["lib.py", "jobs.py", "app.py"])?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "run", 2, Some(("jobs.py", "run", "scip"))),
            ("call", "helper", 4, Some(("lib.py", "helper", "scip"))),
            ("call", "run", 5, Some(("jobs.py", "run", "scip"))),
        ])
    );
    Ok(())
}

/// Once a file changes, the SCIP index's word on it is out of date: the references the index tied
/// into it are tied again as a full index ties them, through the imports of their own files, which
/// are read again for it though they did not change. Once `jobs.py` no longer defines `run`, its
/// import and its call find nothing.
#[test]
fn ties_into_an_updated_file_go_through_imports_again() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    index_imports_the_index_ties(root)?;
    std::fs::write(root.join("lib.py"), "def helper():\n    return 1\n")?;
    std::fs::write(root.join("jobs.py"), "def run():\n    return 1\n")?;
    index_changed(root, &["lib.py", "jobs.py"])?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "run", 2, Some(("jobs.py", "run", "import_path"))),
            (
                "call",
                "helper",
                4,
                Some(("lib.py", "helper", "import_path"))
            ),
            ("call", "run", 5, Some(("jobs.py", "run", "import_path"))),
        ])
    );
    std::fs::write(root.join("jobs.py"), "def other():\n    pass\n")?;
    index_changed(root, &["jobs.py"])?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "run", 2, None),
            (
                "call",
                "helper",
                4,
                Some(("lib.py", "helper", "import_path"))
            ),
            ("call", "run", 5, None),
        ])
    );
    Ok(())
}

/// A full index over a store that holds the SCIP index's ties does the same for the files changed
/// since, here `lib.py`, and, once the SCIP index is gone, for every tie it gave.
#[test]
fn a_full_index_ties_again_what_the_index_no_longer_ties() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    index_imports_the_index_ties(root)?;
    std::fs::write(root.join("lib.py"), "def helper():\n    return 1\n")?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "run", 2, Some(("jobs.py", "run", "scip"))),
            (
                "call",
                "helper",
                4,
                Some(("lib.py", "helper", "import_path"))
            ),
            ("call", "run", 5, Some(("jobs.py", "run", "scip"))),
        ])
    );
    std::fs::remove_file(root.join("index.scip"))?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "run", 2, Some(("jobs.py", "run", "import_path"))),
            (
                "call",
                "helper",
                4,
                Some(("lib.py", "helper", "import_path"))
            ),
            ("call", "run", 5, Some(("jobs.py", "run", "import_path"))),
        ])
    );
    Ok(())
}

/// A file removed takes the SCIP index's ties into it along: the references tied there are tied
/// again as a full index ties them. `app.py` defines `run` and calls it (line 5), and the index tied
/// the call to `jobs.py`; once `jobs.py` is gone, the call is tied to the definition in its own
/// file, though `app.py` neither imports `jobs.py` nor changes, and `run` keeps two definitions.
#[test]
fn ties_into_a_removed_file_are_tied_again() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("jobs.py", "def run():\n    pass\n"),
            ("tools.py", "def run():\n    pass\n"),
            ("app.py", "def run():\n    pass\n\n\nrun()\n"),
        ],
    )?;
    let run = symbol("jobs/run().");
    write_index(
        root,
        vec![
            document("jobs.py", vec![packed(&run, 0, true)]),
            document("app.py", vec![packed(&run, 4, false)]),
        ],
    )?;
    backdate(root, &["jobs.py", "tools.py", "app.py"])?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[("call", "run", 5, Some(("jobs.py", "run", "scip")))])
    );
    std::fs::remove_file(root.join("jobs.py"))?;
    index_changed(root, &["jobs.py"])?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[("call", "run", 5, Some(("app.py", "run", "same_file")))])
    );
    Ok(())
}
