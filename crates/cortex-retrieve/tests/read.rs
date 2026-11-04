//! The graph read from a store: what the index and the learner wrote, weighed for spreading.

mod common;

use common::{Repo, TestResult, function};
use cortex_core::{EdgeType, FileFlags, InterestTier};
use cortex_retrieve::{GraphSettings, Standing, read_graph};

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
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
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

/// Each node's standing as of the latest task, worked out by hand at the default decay of 0.98 a
/// task, ten tasks after its interest was written at task 0 (0.98^10 = 0.8170728068875467): `f`,
/// 0.5 stored as interesting, stands at 0.4085, interesting; `b.py`, 0.22 stored as interesting,
/// at 0.1798, fog; `l.py`, a landmark, stays one; `g` has no interest, dark at 0. A generated or a
/// vendored file is not listed; every other node is.
#[test]
fn each_node_stands_as_of_the_latest_task() -> TestResult {
    let mut repo = Repo::new()?;
    let a = repo.file(
        "a.py",
        "def f():\n    pass\n",
        &[function("f", 1, 2)],
        FileFlags::empty(),
    )?;
    let b = repo.file(
        "b.py",
        "def g():\n    pass\n",
        &[function("g", 1, 2)],
        FileFlags::empty(),
    )?;
    let l = repo.file("l.py", "x = 1\n", &[], FileFlags::empty())?[0];
    let generated = repo.file("gen.py", "x = 1\n", &[], FileFlags::GENERATED)?[0];
    let vendored = repo.file("vendor/x.py", "x = 1\n", &[], FileFlags::VENDORED)?[0];
    repo.interest(&[
        (a[1], (0.5, 0), InterestTier::Interesting),
        (b[0], (0.22, 0), InterestTier::Interesting),
        (l, (3.0, 0), InterestTier::Landmark),
    ])?;
    repo.tasks(10)?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let standing = |node| {
        graph
            .index_of(node)
            .and_then(|index| graph.standing(index))
            .ok_or("a node of the graph")
    };
    let faded = 0.817_072_806_887_546_7;
    let f = standing(a[1])?;
    assert!((f.interest - 0.5 * faded).abs() < 1e-12);
    assert_eq!(f.tier, InterestTier::Interesting);
    let file_b = standing(b[0])?;
    assert!((file_b.interest - 0.22 * faded).abs() < 1e-12);
    assert_eq!(file_b.tier, InterestTier::Dark);
    assert_eq!(standing(l)?.tier, InterestTier::Landmark);
    assert_eq!(standing(b[1])?, Standing::default());
    assert!(!standing(generated)?.is_listed);
    assert!(!standing(vendored)?.is_listed);
    assert!(standing(a[0])?.is_listed);
    assert!(standing(l)?.is_listed);
    Ok(())
}
