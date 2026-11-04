//! The graph read from a store: what the index and the learner wrote, weighed for spreading.

mod common;

use common::{Repo, TestResult, function};
use cortex_core::{EdgeType, FileFlags, TypeWeights};
use cortex_retrieve::read_graph;

/// Worked by hand. In `src`, `a.py` holds `f` and `b.py` holds `g`; `a.py` imports `b.py` at
/// confidence 0.8, `f` calls `g` at 0.6, `a.py` changes with `b.py` at 0.5, and the two were used
/// together to a weight of 2.5 at task 0, which is also the latest (no task is stored), so nothing
/// fades. `a.py`'s links: `b.py` structurally at 0.8, by co-change at 0.6 x 0.5 = 0.3, by co-access
/// at 0.8 x 2.5 / 5 = 0.4, and `f` by membership at 0.1, 1.6 in all. `f`'s: `g` at 0.6 and `a.py`
/// at 0.1. `b.py` has no co-change link back.
#[test]
fn the_store_s_graph_is_weighed_as_worked_out() -> TestResult {
    let mut repo = Repo::new()?;
    let a = repo.file(
        "src/a.py",
        "def f():\n    g()\n",
        &[function("f", 1, 2)],
        FileFlags::empty(),
    )?;
    let b = repo.file(
        "src/b.py",
        "def g():\n    pass\n",
        &[function("g", 1, 2)],
        FileFlags::empty(),
    )?;
    let (file_a, f) = (a[0], a[1]);
    let (file_b, g) = (b[0], b[1]);
    repo.link((file_a, file_b), EdgeType::Structural, (0.8, 1, 0))?;
    repo.link((f, g), EdgeType::Structural, (0.6, 1, 0))?;
    repo.link((file_a, file_b), EdgeType::CoChange, (0.5, 3, 0))?;
    repo.link((file_a, file_b), EdgeType::CoAccess, (2.5, 4, 0))?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), TypeWeights::default(), 0.995)?;
    let index = |node| graph.index_of(node).ok_or("a node of the store");
    let links = |node, kind| -> TestResult<Vec<(u32, f64)>> {
        Ok(graph.links(index(node)?, kind).collect())
    };
    let near = |found: Vec<(u32, f64)>, expected: &[(u32, f64)]| {
        found.len() == expected.len()
            && found
                .iter()
                .zip(expected)
                .all(|((node, weight), (want, wanted))| {
                    node == want && (weight - wanted).abs() < 1e-6
                })
    };
    assert_eq!(graph.len(), 5);
    assert!(near(
        links(file_a, EdgeType::Structural)?,
        &[(index(file_b)?, 0.8)]
    ));
    assert!(near(
        links(file_a, EdgeType::CoChange)?,
        &[(index(file_b)?, 0.3)]
    ));
    assert!(near(
        links(file_a, EdgeType::CoAccess)?,
        &[(index(file_b)?, 0.4)]
    ));
    assert!(near(
        links(file_a, EdgeType::Containment)?,
        &[(index(f)?, 0.1)]
    ));
    assert!((graph.out_weight(index(file_a)?) - 1.6).abs() < 1e-6);
    assert!(near(links(f, EdgeType::Structural)?, &[(index(g)?, 0.6)]));
    assert!(near(
        links(f, EdgeType::Containment)?,
        &[(index(file_a)?, 0.1)]
    ));
    assert!(links(file_b, EdgeType::CoChange)?.is_empty());
    assert_eq!(graph.file_of(index(g)?), Some(index(file_b)?));
    Ok(())
}
