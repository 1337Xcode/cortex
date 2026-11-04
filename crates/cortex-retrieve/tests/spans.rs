//! Which lines of a file to point at, on outlines written by hand, and a file's outline as the
//! store gives it.

mod common;

use common::{Def, Repo, TestResult, function};
use cortex_core::{
    FileFlags, Identifier, InterestTier, LineRange, RepoPath, ShortHash, SymbolKind,
};
use cortex_retrieve::{
    GraphSettings, Outlined, ReadSpan, ScoredFile, Span, is_in_context, outline_of, read_graph,
    spans_of,
};

/// Lines `start` to `end`.
fn lines(start: u32, end: u32) -> TestResult<LineRange> {
    Ok(LineRange::new(start, end)?)
}

/// Symbol `node` of `kind` over lines `start` to `end`, inside the symbol at `parent`, of
/// `strength`.
fn symbol(
    (node, kind): (u32, SymbolKind),
    (start, end): (u32, u32),
    parent: Option<usize>,
    strength: f64,
) -> TestResult<Outlined> {
    Ok(Outlined {
        node,
        name: Identifier::new(&format!("s{node}"))?,
        parent,
        kind,
        lines: lines(start, end)?,
        strength,
    })
}

/// A span of lines `start` to `end` for symbol `node`.
fn span(start: u32, end: u32, node: u32) -> TestResult<Span> {
    Ok(Span {
        lines: lines(start, end)?,
        symbol: Some(node),
    })
}

/// A function of 20 lines that scored is pointed at whole.
#[test]
fn a_short_function_is_pointed_at_whole() -> TestResult {
    let outline = [symbol((7, SymbolKind::Function), (10, 29), None, 0.4)?];
    assert_eq!(spans_of(&outline, 200, 60), [span(10, 29, 7)?]);
    Ok(())
}

/// Worked by hand with a cap of 60: a constant on line 40 (0.5) inside a class of lines 5 to 100
/// points at the class, 96 lines, cut to the 60 lines centred on the constant: from 40 - 29 = 11 to
/// 70. A method of the class on lines 60 to 70 (0.2) falls within that window and adds nothing.
#[test]
fn a_constant_points_at_its_class_cut_around_it() -> TestResult {
    let outline = [
        symbol((1, SymbolKind::Class), (5, 100), None, 0.0)?,
        symbol((2, SymbolKind::Constant), (40, 40), Some(0), 0.5)?,
        symbol((3, SymbolKind::Method), (60, 70), Some(0), 0.2)?,
    ];
    assert_eq!(spans_of(&outline, 120, 60), [span(11, 70, 2)?]);
    Ok(())
}

/// A function of 200 lines is cut to the 60 centred on its middle line, 1 + 199 / 2 = 100: lines
/// 71 to 130. A window near an end stays inside the function: centred on a nested function's
/// middle line, 3, it runs from 1 to 60, and the nested function's own span (2 to 4) joins it.
#[test]
fn a_long_function_is_cut_to_the_cap() -> TestResult {
    let outline = [symbol((5, SymbolKind::Function), (1, 200), None, 1.0)?];
    assert_eq!(spans_of(&outline, 200, 60), [span(71, 130, 5)?]);
    let near_start = [
        symbol((5, SymbolKind::Function), (1, 200), None, 0.1)?,
        symbol((6, SymbolKind::Function), (2, 4), Some(0), 1.0)?,
    ];
    assert_eq!(spans_of(&near_start, 200, 60), [span(1, 60, 6)?]);
    Ok(())
}

/// At most two spans a file, the strongest first: `f` (0.9) and `g` (0.5), not `h` (0.3). Two
/// spans that touch are one: `f` (1 to 10) and `g` (11 to 20) make 1 to 20, and `h` is second.
#[test]
fn two_spans_at_most_and_touching_ones_join() -> TestResult {
    let apart = [
        symbol((1, SymbolKind::Function), (1, 10), None, 0.9)?,
        symbol((2, SymbolKind::Function), (50, 60), None, 0.5)?,
        symbol((3, SymbolKind::Function), (80, 90), None, 0.3)?,
    ];
    assert_eq!(
        spans_of(&apart, 100, 60),
        [span(1, 10, 1)?, span(50, 60, 2)?]
    );
    let touching = [
        symbol((1, SymbolKind::Function), (1, 10), None, 0.9)?,
        symbol((2, SymbolKind::Function), (11, 20), None, 0.5)?,
        symbol((3, SymbolKind::Function), (80, 90), None, 0.3)?,
    ];
    assert_eq!(
        spans_of(&touching, 100, 60),
        [span(1, 20, 1)?, span(80, 90, 3)?]
    );
    Ok(())
}

/// Two spans that overlap but would together pass the cap stay apart: the weaker is passed over
/// rather than joined. `f` (1 to 40) and `g` (30 to 70) would make 70 lines.
#[test]
fn spans_too_long_together_are_not_joined() -> TestResult {
    let outline = [
        symbol((1, SymbolKind::Function), (1, 40), None, 0.9)?,
        symbol((2, SymbolKind::Function), (30, 70), None, 0.5)?,
    ];
    assert_eq!(spans_of(&outline, 100, 60), [span(1, 40, 1)?]);
    Ok(())
}

/// A file whose symbols have no strength gets no span, to be listed by path; a file with no
/// symbols is shown whole when it has no more lines than the cap, and not at all past it.
#[test]
fn a_file_without_strong_symbols_gets_no_span() -> TestResult {
    let weak = [symbol((1, SymbolKind::Function), (1, 10), None, 0.0)?];
    assert!(spans_of(&weak, 10, 60).is_empty());
    assert_eq!(
        spans_of(&[], 30, 60),
        [Span {
            lines: lines(1, 30)?,
            symbol: None,
        }]
    );
    assert!(spans_of(&[], 61, 60).is_empty());
    assert!(spans_of(&[], 0, 60).is_empty());
    Ok(())
}

/// A span is in context when a read of the same file, while the file holds what was read, covered
/// its lines or the whole file; not when the file changed since, the read covered only part of it,
/// or the read was of another file.
#[test]
fn a_span_read_while_the_file_is_unchanged_is_in_context() -> TestResult {
    let held = ShortHash::of(b"held");
    let changed = ShortHash::of(b"changed");
    let read = |path: &str, range: Option<(u32, u32)>, content| -> TestResult<ReadSpan> {
        Ok(ReadSpan {
            path: RepoPath::new(path)?,
            range: range.map(|(start, end)| lines(start, end)).transpose()?,
            content: Some(content),
        })
    };
    let target = span(10, 29, 1)?;
    let at = ("a.py", held);
    assert!(is_in_context(
        &target,
        at,
        &[read("a.py", Some((5, 40)), held)?]
    ));
    assert!(is_in_context(&target, at, &[read("a.py", None, held)?]));
    assert!(!is_in_context(
        &target,
        at,
        &[read("a.py", Some((5, 40)), changed)?]
    ));
    assert!(!is_in_context(
        &target,
        at,
        &[read("a.py", Some((15, 40)), held)?]
    ));
    assert!(!is_in_context(&target, at, &[read("b.py", None, held)?]));
    let unknown = ReadSpan {
        content: None,
        ..read("a.py", None, held)?
    };
    assert!(!is_in_context(&target, at, &[unknown]));
    Ok(())
}

/// A file's outline read from the store: a class `C` (lines 1 to 20) holding a method `m` (5 to
/// 10), and a function `f` (22 to 30), each with its kind, lines and the symbol around it. When a
/// symbol of the file scored (`m`, 0.5), each symbol is as strong as its score, the others 0; when
/// none did, as strong as its interest (`f`, 0.7, as of the latest task, which is task 0).
#[test]
fn a_file_s_outline_is_read_with_scores_or_interest() -> TestResult {
    let mut repo = Repo::new()?;
    let defs = [
        Def {
            name: "C",
            kind: SymbolKind::Class,
            start: 1,
            end: 20,
            parent: None,
        },
        Def {
            name: "m",
            kind: SymbolKind::Method,
            start: 5,
            end: 10,
            parent: Some(0),
        },
        function("f", 22, 30),
    ];
    let nodes = repo.file("a.py", &"line\n".repeat(30), &defs, FileFlags::empty())?;
    let f = *nodes.get(3).ok_or("f")?;
    repo.interest(&[(f, (0.7, 0), InterestTier::Interesting)])?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let row = reader
        .queries()
        .file(&RepoPath::new("a.py")?)?
        .ok_or("the file")?;
    let n = |at: usize| -> TestResult<u32> {
        let node = *nodes.get(at).ok_or("a node")?;
        Ok(graph.index_of(node).ok_or("a node of the graph")?)
    };
    let symbol = |(at, parent): (usize, Option<usize>), kind, (start, end), strength| {
        let name = ["C", "m", "f"].get(at - 1).ok_or("a name")?;
        Ok(Outlined {
            node: n(at)?,
            name: Identifier::new(name)?,
            parent,
            kind,
            lines: lines(start, end)?,
            strength,
        })
    };
    let scored = ScoredFile {
        file: n(0)?,
        score: 0.5,
        activation: 0.5,
        symbols: vec![(n(2)?, 0.5)],
    };
    let expected: Vec<Outlined> = [
        symbol((1, None), SymbolKind::Class, (1, 20), 0.0),
        symbol((2, Some(0)), SymbolKind::Method, (5, 10), 0.5),
        symbol((3, None), SymbolKind::Function, (22, 30), 0.0),
    ]
    .into_iter()
    .collect::<TestResult<_>>()?;
    assert_eq!(
        outline_of(reader.queries(), &graph, (&scored, &row))?,
        expected
    );
    let unscored = ScoredFile {
        symbols: Vec::new(),
        ..scored
    };
    let strengths: Vec<f64> = outline_of(reader.queries(), &graph, (&unscored, &row))?
        .iter()
        .map(|symbol| symbol.strength)
        .collect();
    assert_eq!(strengths, [0.0, 0.0, 0.7]);
    Ok(())
}
