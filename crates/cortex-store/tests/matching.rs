//! Which stored symbols a new reading of a file would change the signature of, asked without
//! writing, matched as the writer matches; the references tied to one symbol; and a file's tests.

mod common;

use common::{Scratch, TestResult, index, reference, symbol};
use cortex_core::{EdgeType, RefKind, Resolution, SymbolKind};
use cortex_store::{ChangeKind, EdgeRecord, FileChange, Resolved, SymbolId, Target};

/// `a.py` as stored, then read again. By hand: `f` keeps its kind and name with a new header
/// (signature `s1` to `s9`): changed. `g` keeps its header with a new body: not a signature change.
/// The two methods `C.m` (signatures `m1`, `m2`) match the new reading's two `C.m` in stored order,
/// so the first (`m1` against the new first, `m2`) and the second (`m2` against `m1`) both change.
/// `h` is new and `gone` is removed: neither is a stored symbol whose signature changed. The writer,
/// writing the same reading next, reports exactly those signature changes.
#[test]
fn signature_changes_are_found_without_writing() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let stored = transaction.replace_file(&index(
        "a.py",
        vec![
            symbol(SymbolKind::Function, "f", None, ("s1", "b1"))?,
            symbol(SymbolKind::Function, "g", None, ("s2", "b2"))?,
            symbol(SymbolKind::Method, "C.m", None, ("m1", "b3"))?,
            symbol(SymbolKind::Method, "C.m", None, ("m2", "b4"))?,
            symbol(SymbolKind::Function, "gone", None, ("s5", "b5"))?,
        ],
        Vec::new(),
    )?)?;
    transaction.commit()?;
    let [f, _g, first_m, second_m, _gone] = stored.symbols[..] else {
        return Err("five symbols".into());
    };
    let again = index(
        "a.py",
        vec![
            symbol(SymbolKind::Function, "f", None, ("s9", "b1"))?,
            symbol(SymbolKind::Function, "g", None, ("s2", "b9"))?,
            symbol(SymbolKind::Method, "C.m", None, ("m2", "b3"))?,
            symbol(SymbolKind::Method, "C.m", None, ("m1", "b4"))?,
            symbol(SymbolKind::Function, "h", None, ("s6", "b6"))?,
        ],
        Vec::new(),
    )?;
    let asked = writer
        .queries()
        .signature_changes(stored.file, &again.symbols)?;
    assert_eq!(asked, [f, first_m, second_m]);
    let mut transaction = writer.transaction()?;
    let written = transaction.replace_file(&again)?;
    transaction.commit()?;
    let reported: Vec<SymbolId> = written
        .changes
        .iter()
        .filter(|change| change.kind == ChangeKind::Signature)
        .map(|change| change.symbol)
        .collect();
    assert_eq!(reported, asked);
    // Written, the same reading changes nothing more.
    assert_eq!(
        writer
            .queries()
            .signature_changes(stored.file, &again.symbols)?,
        []
    );
    Ok(())
}

/// The references tied to `f`, by file and line: `c.py`'s line 1 and `b.py`'s lines 2 and 4, read
/// back as `b.py` (stored first) 2 and 4, then `c.py` 1, though its line comes first; `b.py`'s
/// untied reference and its reference to `g` are not among them.
#[test]
fn the_references_tied_to_a_symbol_are_read_by_file_and_line() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let a = transaction.replace_file(&index(
        "a.py",
        vec![
            symbol(SymbolKind::Function, "f", None, ("s1", "b1"))?,
            symbol(SymbolKind::Function, "g", None, ("s2", "b2"))?,
        ],
        Vec::new(),
    )?)?;
    let b = transaction.replace_file(&index(
        "b.py",
        Vec::new(),
        vec![
            reference(None, "f", RefKind::Call, 4)?,
            reference(None, "f", RefKind::Call, 2)?,
            reference(None, "f", RefKind::Call, 9)?,
            reference(None, "g", RefKind::Call, 3)?,
        ],
    )?)?;
    let c = transaction.replace_file(&index(
        "c.py",
        Vec::new(),
        vec![reference(None, "f", RefKind::Call, 1)?],
    )?)?;
    let [f, g] = a.symbols[..] else {
        return Err("two symbols".into());
    };
    let tie = |reference, symbol, resolution| Resolved {
        reference,
        target: Some(Target { symbol, resolution }),
        goes_through_import: false,
    };
    let (b_refs, c_refs) = (&b.refs, &c.refs);
    let [b4, b2, _b9, b3] = b_refs[..] else {
        return Err("four references".into());
    };
    let [c1] = c_refs[..] else {
        return Err("one reference".into());
    };
    transaction.set_resolutions(&[
        tie(c1, f, Resolution::ImportPath),
        tie(b4, f, Resolution::NameOnly),
        tie(b2, f, Resolution::ImportPath),
        tie(b3, g, Resolution::ImportPath),
    ])?;
    transaction.commit()?;
    let tied: Vec<(String, u32, f64)> = writer
        .queries()
        .refs_tied_to(f)?
        .iter()
        .map(|row| {
            let file = if row.file == b.file { "b.py" } else { "c.py" };
            (file.to_owned(), row.line, row.confidence)
        })
        .collect();
    assert_eq!(
        tied,
        [
            ("b.py".to_owned(), 2, 0.8),
            ("b.py".to_owned(), 4, 0.3),
            ("c.py".to_owned(), 1, 0.8),
        ]
    );
    Ok(())
}

/// The test files with a structural edge into `a.py`, by node: `tests/test_a.py` (a test, an
/// import edge at 0.8) and `tests/test_a2.py` (a test, an edge by name at 0.3). Not `b.py` (an edge,
/// and a function but no test), `tests/test_b.py` (a test but no edge from the file), nor
/// `tests/test_c.py` (a test whose only edge into `a.py` is co-change). The test in
/// `tests/test_b.py` has an edge of its own into `a.py`, from a symbol node, which is no file: the
/// symbols are stored in the order the files are, so its id, 3, is the id of `tests/test_a.py`'s
/// file, which holds a test, and a query that took any node for a file would name it.
#[test]
fn the_tests_of_a_file_are_the_test_files_tied_to_it() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let mut stored = |path: &str, symbols| -> TestResult<FileChange> {
        Ok(transaction.replace_file(&index(path, symbols, Vec::new())?)?)
    };
    let test = |name: &str| symbol(SymbolKind::Test, name, None, ("s", "b"));
    let a = stored("a.py", Vec::new())?.file_node;
    let b = stored(
        "b.py",
        vec![symbol(SymbolKind::Function, "helper", None, ("s", "b"))?],
    )?;
    let test_a = stored("tests/test_a.py", vec![test("test_a")?])?;
    let test_b = stored("tests/test_b.py", vec![test("test_b")?])?;
    let test_c = stored("tests/test_c.py", vec![test("test_c")?])?.file_node;
    let test_a2 = stored("tests/test_a2.py", vec![test("test_a2")?])?.file_node;
    assert_eq!(
        (test_b.symbols.first().map(|id| id.get()), test_a.file.get()),
        (Some(3), 3)
    );
    let edge = |weight| EdgeRecord {
        dst: a,
        weight,
        count: 1,
        last_seq: 0,
        is_directed: true,
    };
    let [test_b_symbol] = test_b.symbol_nodes[..] else {
        return Err("one symbol node".into());
    };
    transaction.replace_edges(test_a.file_node, EdgeType::Structural, &[edge(0.8)])?;
    transaction.replace_edges(test_a2, EdgeType::Structural, &[edge(0.3)])?;
    transaction.replace_edges(b.file_node, EdgeType::Structural, &[edge(0.8)])?;
    transaction.replace_edges(test_b_symbol, EdgeType::Structural, &[edge(0.8)])?;
    transaction.replace_edges(test_c, EdgeType::CoChange, &[edge(0.5)])?;
    transaction.commit()?;
    assert_eq!(
        writer.queries().test_files_into(a)?,
        [(test_a.file_node, 0.8), (test_a2, 0.3)]
    );
    Ok(())
}
