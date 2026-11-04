//! The seeds of a retrieval: earlier work like the prompt's, the dark code next to the strongest
//! seeds, and the one vector every source adds to, each worked out by hand.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{Repo, TestResult, event, function, record, tool};
use cortex_core::{EdgeType, FileFlags, HookKind, Identifier, InterestTier, ToolKind};
use cortex_retrieve::{
    GraphSettings, PromptTerms, SeedQuery, exploration, read_graph, seeds, shares, similar, summed,
};
use cortex_store::NodeId;
use proptest::prelude::*;

/// Whether `found` is `expected` to within rounding.
fn near(found: Option<&f64>, expected: f64) -> bool {
    found.is_some_and(|found| (found - expected).abs() < 1e-12)
}

/// Clusters written by hand against the prompt "fix the parser token handling in the lexer", whose
/// words are parser, token, handling and lexer. Each cluster's first five words are compared, the
/// prompt having four. The first cluster has three words, all the prompt's, of four between them,
/// 3/4 = 0.75, so `t.py` and `l.py` get 0.6 x 0.75 = 0.45; the second shares parser alone of
/// eight, 1/8, below the bar of 0.3; the third shares parser, token and lexer of six words between
/// them, 3/6 = 0.5, so its files `p.py` and `t.py` get 0.6 x 0.5 = 0.3, and `t.py` keeps the better
/// 0.45 it had already. A cluster's file the store no longer holds is passed over. The best score
/// is 0.75.
#[test]
fn clusters_like_the_prompt_seed_their_files() -> TestResult {
    let mut repo = Repo::new()?;
    let p = repo.file("p.py", "x = 1\n", &[], FileFlags::empty())?[0];
    let t = repo.file("t.py", "x = 1\n", &[], FileFlags::empty())?[0];
    let l = repo.file("l.py", "x = 1\n", &[], FileFlags::empty())?[0];
    let b = repo.file("b.py", "x = 1\n", &[], FileFlags::empty())?[0];
    let gone = repo.file("gone.py", "x = 1\n", &[], FileFlags::empty())?[0];
    repo.remove("gone.py")?;
    repo.clusters(&[
        (&["token", "lexer", "handling"], &[t, l], 1),
        (&["billing", "invoice", "tax", "parser", "total"], &[b], 4),
        (
            &[
                "parser", "token", "lexer", "grammar", "ast", "node", "visitor",
            ],
            &[p, t, gone],
            3,
        ),
    ])?;
    let terms = PromptTerms::of("fix the parser token handling in the lexer");
    assert_eq!(terms.words, ["parser", "token", "handling", "lexer"]);
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let found = similar(reader.queries(), &graph, &terms)?;
    let index = |node: NodeId| graph.index_of(node).ok_or("a node of the graph");
    let (p, t, l) = (index(p)?, index(t)?, index(l)?);
    assert!(near(found.seeds.get(&p), 0.3));
    assert!(near(found.seeds.get(&t), 0.45));
    assert!(near(found.seeds.get(&l), 0.45));
    assert_eq!(found.seeds.len(), 3);
    assert_eq!(found.files.get(&p), Some(&(0.5, 3)));
    assert_eq!(found.files.get(&t), Some(&(0.75, 1)));
    assert_eq!(found.files.get(&l), Some(&(0.75, 1)));
    assert!((found.best - 0.75).abs() < 1e-12);
    Ok(())
}

/// A prompt of one word shares it with a cluster's first five words: 1/5 = 0.2, below the bar, so
/// nothing is seeded, though the best score is still 0.2.
#[test]
fn one_shared_word_is_not_a_match() -> TestResult {
    let mut repo = Repo::new()?;
    let p = repo.file("p.py", "x = 1\n", &[], FileFlags::empty())?[0];
    repo.clusters(&[(&["parser", "token", "lexer", "grammar", "ast"], &[p], 2)])?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let found = similar(reader.queries(), &graph, &PromptTerms::of("parser"))?;
    assert!(found.seeds.is_empty());
    assert!((found.best - 0.2).abs() < 1e-12);
    Ok(())
}

/// The dark structural neighbours of the strongest seeds, worked out by hand at task 100 and the
/// default decay of 0.98 a task: `f` (seeded 1.0) calls `h`, `g` and `m` and is called by `k`; `a.py`
/// (seeded 0.5) imports `e.py`. `h` was interesting at task 1 (0.3, now 0.3 x 0.98^99 = 0.04,
/// fog); `g` stands at 5.0 x 0.98 = 4.9, interesting; `e.py` is a landmark; `k` and `m` have no
/// interest at all. So `h`, `k` and `m` are explored at 0.05 each, and `g` and `e.py`, lit, are not.
#[test]
fn the_dark_neighbours_of_the_strongest_seeds_are_explored() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "line\n".repeat(10);
    let defs = [function("f", 1, 4), function("g", 6, 9)];
    let a = repo.file("a.py", &text, &defs, FileFlags::empty())?;
    let h = repo.file("b.py", &text, &[function("h", 1, 3)], FileFlags::empty())?[1];
    let k = repo.file("c.py", &text, &[function("k", 1, 3)], FileFlags::empty())?[1];
    let m = repo.file("d.py", &text, &[function("m", 1, 3)], FileFlags::empty())?[1];
    let e = repo.file("e.py", "x = 1\n", &[], FileFlags::empty())?[0];
    let (file_a, f, g) = (a[0], a[1], a[2]);
    for (src, dst) in [(f, h), (f, g), (k, f), (f, m), (file_a, e)] {
        repo.link((src, dst), EdgeType::Structural, (1.0, 1, 0))?;
    }
    repo.interest(&[
        (h, (0.3, 1), InterestTier::Interesting),
        (g, (5.0, 99), InterestTier::Interesting),
        (e, (3.5, 1), InterestTier::Landmark),
    ])?;
    repo.tasks(100)?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let index = |node: NodeId| graph.index_of(node).ok_or("a node of the graph");
    let seeded = BTreeMap::from([(index(f)?, 1.0), (index(file_a)?, 0.5)]);
    let explored = exploration(&graph, &seeded);
    let expected = BTreeMap::from([(index(h)?, 0.05), (index(k)?, 0.05), (index(m)?, 0.05)]);
    assert_eq!(explored, expected);
    Ok(())
}

/// Of a symbol's 25 callers, none lit, the first 20 by node are explored: their links weigh alike,
/// so the order is the nodes'.
#[test]
fn at_most_twenty_neighbours_are_explored() -> TestResult {
    let mut repo = Repo::new()?;
    let defs = [function("hub", 1, 1)];
    let hub = repo.file("hub.py", "line\n", &defs, FileFlags::empty())?[1];
    let mut callers = Vec::new();
    for at in 0..25 {
        let path = format!("c{at:02}.py");
        let caller = repo.file(
            &path,
            "line\n",
            &[function("call", 1, 1)],
            FileFlags::empty(),
        )?[1];
        repo.link((caller, hub), EdgeType::Structural, (1.0, 1, 0))?;
        callers.push(caller);
    }
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let index = |node: NodeId| graph.index_of(node).ok_or("a node of the graph");
    let seeded = BTreeMap::from([(index(hub)?, 1.0)]);
    let explored = exploration(&graph, &seeded);
    let mut expected = BTreeMap::new();
    for caller in callers.iter().take(20) {
        expected.insert(index(*caller)?, 0.05);
    }
    assert_eq!(explored, expected);
    Ok(())
}

/// The parts add per node and the whole is divided by its sum, worked out by hand: node 1 has 1.0,
/// node 2 has 0.5 + 1.0 = 1.5, node 3 0.4, node 4 0.3 and node 5 0.05, 3.25 in all, which is 13/4;
/// so the shares are 4/13, 6/13, 1.6/13, 1.2/13 and 0.2/13. Nothing, or nothing of any weight,
/// gives no shares, and a node of no weight has none.
#[test]
fn the_parts_add_and_the_vector_sums_to_one() {
    let parts = [
        BTreeMap::from([(1, 1.0), (2, 0.5)]),
        BTreeMap::from([(2, 1.0), (3, 0.4)]),
        BTreeMap::from([(4, 0.3)]),
        BTreeMap::from([(5, 0.05)]),
    ];
    let vector = shares(&summed(&parts.iter().collect::<Vec<_>>()));
    let expected = [
        (1, 4.0 / 13.0),
        (2, 6.0 / 13.0),
        (3, 1.6 / 13.0),
        (4, 1.2 / 13.0),
        (5, 0.2 / 13.0),
    ];
    assert_eq!(vector.len(), expected.len());
    for (node, share) in expected {
        assert!(near(vector.get(&node), share), "{node}");
    }
    assert!(shares(&BTreeMap::new()).is_empty());
    assert!(shares(&BTreeMap::from([(1, 0.0)])).is_empty());
    assert_eq!(
        shares(&BTreeMap::from([(1, 0.0), (2, 2.0)])),
        BTreeMap::from([(2, 1.0)])
    );
}

/// One retrieval's seeds from every source, worked out by hand. The prompt's words (quokka,
/// wombat, numbat) are in no row of the index, so they find nothing; the session edited `g`, 1.0;
/// the work tree holds `u.py` changed and not committed, 0.8; a cluster's first five words hold all
/// three of the prompt's, of five between them, 3/5 = 0.6, so its file `t.py` gets 0.6 x 0.6 =
/// 0.36; `g` calls `h`, which no task has touched, 0.05. In all 2.21: `g` 1/2.21, `u.py` 0.8/2.21,
/// `t.py` 0.36/2.21, `h` 0.05/2.21.
#[test]
fn every_source_adds_to_one_vector() -> TestResult {
    let mut repo = Repo::new()?;
    let text = "line\n".repeat(10);
    let defs = [function("f", 1, 4), function("g", 6, 9)];
    let g = repo.file("a.py", &text, &defs, FileFlags::empty())?[2];
    let h = repo.file("b.py", &text, &[function("h", 1, 3)], FileFlags::empty())?[1];
    let t = repo.file("t.py", "x = 1\n", &[], FileFlags::empty())?[0];
    let u = repo.file("u.py", "y = 1\n", &[], FileFlags::empty())?[0];
    repo.link((g, h), EdgeType::Structural, (1.0, 1, 0))?;
    repo.clusters(&[(&["quokka", "wombat", "numbat", "dingo", "emu"], &[t], 2)])?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(("s1", 2_100), ToolKind::Edit, ("a.py", &[(7, 8)]))?,
        ],
    )?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let terms = PromptTerms::of("quokka wombat numbat");
    let session = Identifier::new("s1")?;
    let query = SeedQuery {
        terms: &terms,
        session: Some(&session),
        uncommitted: &[u],
    };
    let found = seeds(reader.queries(), &graph, &query)?;
    let index = |node: NodeId| graph.index_of(node).ok_or("a node of the graph");
    let (g, h, t, u) = (index(g)?, index(h)?, index(t)?, index(u)?);
    assert!(found.lexical.seeds.is_empty());
    assert_eq!(found.uncommitted.files, BTreeSet::from([u]));
    assert_eq!(found.exploration, BTreeMap::from([(h, 0.05)]));
    assert_eq!(found.vector.len(), 4);
    assert!(near(found.vector.get(&g), 1.0 / 2.21));
    assert!(near(found.vector.get(&u), 0.8 / 2.21));
    assert!(near(found.vector.get(&t), 0.36 / 2.21));
    assert!(near(found.vector.get(&h), 0.05 / 2.21));
    Ok(())
}

/// With nothing to go on, a retrieval has no seeds and no vector: no word of the prompt is in the
/// index, the session has done nothing and there is no earlier work.
#[test]
fn nothing_found_is_no_vector() -> TestResult {
    let mut repo = Repo::new()?;
    repo.file(
        "a.py",
        "x = 1\n",
        &[function("f", 1, 1)],
        FileFlags::empty(),
    )?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let terms = PromptTerms::of("quokka wombat numbat");
    let session = Identifier::new("s1")?;
    let query = SeedQuery {
        terms: &terms,
        session: Some(&session),
        uncommitted: &[],
    };
    let found = seeds(reader.queries(), &graph, &query)?;
    assert!(found.vector.is_empty());
    assert!(found.exploration.is_empty());
    Ok(())
}

proptest! {
    /// Any weights give one share for each node of some weight, each above 0 and at most 1, and
    /// the shares sum to 1; weights of which none weighs anything give none.
    #[test]
    fn shares_always_sum_to_one(
        weights in prop::collection::btree_map(0_u32..1_000, 0.0_f64..10.0, 0..60),
    ) {
        let vector = shares(&weights);
        let weighing: Vec<u32> = weights
            .iter()
            .filter(|(_, weight)| **weight > 0.0)
            .map(|(node, _)| *node)
            .collect();
        prop_assert_eq!(vector.keys().copied().collect::<Vec<_>>(), weighing.clone());
        if !weighing.is_empty() {
            let total: f64 = vector.values().sum();
            prop_assert!((total - 1.0).abs() < 1e-9);
            prop_assert!(vector.values().all(|share| *share > 0.0 && *share <= 1.0));
        }
    }
}
