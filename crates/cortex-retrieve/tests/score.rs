//! What each file scores and which kinds of evidence point at it, on graphs built by hand.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{EdgeType, InterestTier, NodeKind, TypeWeights};
use cortex_retrieve::{
    Activation, Evidence, FileMatch, Graph, LexicalMatch, LinkInput, NodeInput, Seeds,
    SessionMatch, SimilarMatch, Standing, Support, UncommittedMatch, Weighing, scored_files,
};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A node of `kind`, in file `file` for a symbol, standing as `standing`.
fn node(id: i64, kind: NodeKind, file: Option<i64>, standing: Standing) -> NodeInput<i64> {
    NodeInput {
        id,
        kind,
        file,
        standing,
    }
}

/// A node agents have not worked on.
fn plain(id: i64, kind: NodeKind, file: Option<i64>) -> NodeInput<i64> {
    node(id, kind, file, Standing::default())
}

/// A node in tier `tier`.
fn tiered(id: i64, kind: NodeKind, file: Option<i64>, tier: InterestTier) -> NodeInput<i64> {
    node(
        id,
        kind,
        file,
        Standing {
            tier,
            ..Standing::default()
        },
    )
}

/// A link of `kind` stored at `weight`, as of task 0, the latest.
fn link(src: i64, dst: i64, kind: EdgeType, weight: f64) -> LinkInput<i64> {
    LinkInput {
        src,
        dst,
        kind,
        weight,
        last_seq: 0,
    }
}

/// The settings' default weights, nothing faded: the latest task is 0.
fn defaults() -> Weighing {
    Weighing {
        type_weights: TypeWeights::default(),
        latest_seq: 0,
        link_decay: 0.995,
    }
}

/// The graph of `nodes` and `links`, weighed by the settings' defaults with nothing faded.
fn graph(nodes: &[NodeInput<i64>], links: &[LinkInput<i64>]) -> TestResult<Graph<i64>> {
    Ok(Graph::build(nodes, links, &defaults())?)
}

/// Seeds of the prompt's words at the nodes `lexical`, the session at `edited`, and exploration at
/// `explored`, every node of weight 1.
fn seeds_at(lexical: &[u32], edited: &[u32], explored: &[u32]) -> Seeds {
    let weighed = |nodes: &[u32]| nodes.iter().map(|node| (*node, 1.0)).collect();
    Seeds {
        lexical: LexicalMatch {
            seeds: weighed(lexical),
            ..LexicalMatch::default()
        },
        session: SessionMatch {
            seeds: weighed(edited),
            ..SessionMatch::default()
        },
        exploration: weighed(explored),
        ..Seeds::default()
    }
}

/// The kinds of evidence behind node `id` of `graph`, with `seeds`.
fn evidence(graph: &Graph<i64>, seeds: &Seeds, id: i64) -> TestResult<Vec<Evidence>> {
    Ok(Support::new(graph, seeds)
        .of(at(graph, id)?)
        .into_iter()
        .collect())
}

/// The number `graph` gives node `id`.
fn at(graph: &Graph<i64>, id: i64) -> TestResult<u32> {
    Ok(graph.index_of(id).ok_or("a node of the graph")?)
}

/// Files scored by hand. `A` (2) holds `a1` (3, interesting) and `a2` (4); `B` (5) holds `b1` (6);
/// `C` (7) is generated; `D` (8) is a landmark; the folder (1) is in no file. Kept activation: `A`
/// 0.10, `a1` 0.20, `a2` 0.05, `B` 0.30, `b1` 0.12, `C` 0.50, `D` 0.20, the folder 0.40. So `A`
/// scores 0.10 + 0.20 x 1.25 + 0.05 = 0.40, `a1` within it 0.25 and `a2` 0.05; `B` 0.30 + 0.12 =
/// 0.42; `D` 0.20 x 1.5 = 0.30; `C` is left out, and so is the folder. Best first: `B`, `A`, `D`,
/// whose activation before the priors is 0.42, 0.10 + 0.20 + 0.05 = 0.35 and 0.20.
#[test]
fn files_score_their_nodes_activation_times_their_priors() -> TestResult {
    let nodes = [
        plain(1, NodeKind::Dir, None),
        plain(2, NodeKind::File, None),
        tiered(3, NodeKind::Symbol, Some(2), InterestTier::Interesting),
        plain(4, NodeKind::Symbol, Some(2)),
        plain(5, NodeKind::File, None),
        plain(6, NodeKind::Symbol, Some(5)),
        node(
            7,
            NodeKind::File,
            None,
            Standing {
                is_listed: false,
                ..Standing::default()
            },
        ),
        tiered(8, NodeKind::File, None, InterestTier::Landmark),
    ];
    let graph = graph(&nodes, &[])?;
    let kept = [
        (1, 0.40),
        (2, 0.10),
        (3, 0.20),
        (4, 0.05),
        (5, 0.30),
        (6, 0.12),
        (7, 0.50),
        (8, 0.20),
    ];
    let activation = Activation {
        kept: kept
            .iter()
            .map(|(id, kept)| Ok((at(&graph, *id)?, *kept)))
            .collect::<TestResult<_>>()?,
        pushes: 0,
        left: 0.0,
        is_cut_short: false,
        is_out_of_time: false,
    };
    let scored = scored_files(&graph, &activation);
    let files: Vec<u32> = scored.iter().map(|file| file.file).collect();
    assert_eq!(files, [at(&graph, 5)?, at(&graph, 2)?, at(&graph, 8)?]);
    let scores: Vec<f64> = scored.iter().map(|file| file.score).collect();
    for (found, expected) in scores.iter().zip([0.42, 0.40, 0.30]) {
        assert!((found - expected).abs() < 1e-12, "{scores:?}");
    }
    let activation: Vec<f64> = scored.iter().map(|file| file.activation).collect();
    for (found, expected) in activation.iter().zip([0.42, 0.35, 0.20]) {
        assert!((found - expected).abs() < 1e-12, "{activation:?}");
    }
    let a_symbols = &scored.get(1).ok_or("a second file")?.symbols;
    let symbols: Vec<u32> = a_symbols.iter().map(|symbol| symbol.0).collect();
    assert_eq!(symbols, [at(&graph, 3)?, at(&graph, 4)?]);
    let a1 = a_symbols.first().ok_or("a symbol")?;
    assert!((a1.1 - 0.25).abs() < 1e-12);
    Ok(())
}

/// The evidence behind each file, worked out by hand. Files `A` (1, with `a1`, 2), `B` (3, with
/// `b1`, 4), `C` (5, with `c1` and `c2`, 8 and 9), `D` (6), `E` (7). The prompt's words found `a1`,
/// `B` and `C`; earlier work changed `D`; the session edited `E`: so every file is a seed file.
/// `a1` calls `b1` at a confidence of 1.0; `A` names `C` by name alone, 0.3, below the 0.8 a
/// reference needs; `c1` calls `c2`, which says nothing of another file; `A` changes with `D` (the
/// miner wrote `A` to `D` only); `A` and `E` were used together to 0.04, below the 0.05 a co-access
/// link needs, `B` and `E` to 0.25. So `A` has the words, a reference and co-change; `B` the words,
/// a reference and co-access; `C` the words alone; `D` co-change, written the other way, and
/// earlier work; `E` co-access.
#[test]
fn each_file_s_evidence_is_counted_by_kind() -> TestResult {
    let nodes = [
        plain(1, NodeKind::File, None),
        plain(2, NodeKind::Symbol, Some(1)),
        plain(3, NodeKind::File, None),
        plain(4, NodeKind::Symbol, Some(3)),
        plain(5, NodeKind::File, None),
        plain(6, NodeKind::File, None),
        plain(7, NodeKind::File, None),
        plain(8, NodeKind::Symbol, Some(5)),
        plain(9, NodeKind::Symbol, Some(5)),
    ];
    let links = [
        link(2, 4, EdgeType::Structural, 1.0),
        link(1, 5, EdgeType::Structural, 0.3),
        link(8, 9, EdgeType::Structural, 1.0),
        link(1, 6, EdgeType::CoChange, 0.5),
        link(1, 7, EdgeType::CoAccess, 0.04),
        link(3, 7, EdgeType::CoAccess, 0.25),
    ];
    let graph = graph(&nodes, &links)?;
    let n = |id| at(&graph, id);
    let matched = |coverage| FileMatch {
        coverage,
        words: vec!["word".to_owned()],
        is_named: false,
    };
    let seeds = Seeds {
        lexical: LexicalMatch {
            seeds: BTreeMap::from([(n(2)?, 1.0), (n(3)?, 0.5), (n(5)?, 0.2)]),
            files: BTreeMap::from([
                (n(1)?, matched(0.6)),
                (n(3)?, matched(0.3)),
                (n(5)?, matched(0.1)),
            ]),
            top_lexical: 0.6,
        },
        session: SessionMatch {
            seeds: BTreeMap::from([(n(7)?, 1.0)]),
            edited_files: BTreeSet::from([n(7)?]),
            in_context: Vec::new(),
        },
        similar: SimilarMatch {
            seeds: BTreeMap::from([(n(6)?, 0.3)]),
            files: BTreeMap::from([(n(6)?, (0.5, 2))]),
            best: 0.5,
        },
        exploration: BTreeMap::new(),
        vector: BTreeMap::new(),
        ..Seeds::default()
    };
    let support = Support::new(&graph, &seeds);
    let kinds = |id| -> TestResult<Vec<Evidence>> { Ok(support.of(n(id)?).into_iter().collect()) };
    assert_eq!(
        kinds(1)?,
        [Evidence::Lexical, Evidence::Structural, Evidence::CoChange]
    );
    assert_eq!(
        kinds(3)?,
        [Evidence::Lexical, Evidence::Structural, Evidence::CoAccess]
    );
    assert_eq!(kinds(5)?, [Evidence::Lexical]);
    assert_eq!(kinds(6)?, [Evidence::CoChange, Evidence::Similar]);
    assert_eq!(kinds(7)?, [Evidence::CoAccess]);
    Ok(())
}

/// A link to a file that no seed points at is no evidence: `A` calls into `B` at full confidence,
/// changes with it and was used with it to the most a link can weigh, but only `A` is a seed file.
#[test]
fn a_link_to_no_seed_file_is_no_evidence() -> TestResult {
    let nodes = [
        plain(1, NodeKind::File, None),
        plain(2, NodeKind::File, None),
    ];
    let links = [
        link(1, 2, EdgeType::Structural, 1.0),
        link(1, 2, EdgeType::CoChange, 0.9),
        link(1, 2, EdgeType::CoAccess, 5.0),
    ];
    let graph = graph(&nodes, &links)?;
    let seeds = Seeds {
        session: SessionMatch {
            seeds: BTreeMap::from([(at(&graph, 1)?, 1.0)]),
            ..SessionMatch::default()
        },
        ..Seeds::default()
    };
    let support = Support::new(&graph, &seeds);
    assert!(support.of(at(&graph, 1)?).is_empty());
    Ok(())
}

/// The uncommitted diff is no evidence, since the work tree holds it whatever the prompt is about:
/// `A` (1) calls into `B` (2) at full confidence, changes with it and was used with it to the most
/// a link can weigh, and `A` is uncommitted, yet `B` has no evidence while no other source points
/// at `A`; once the session edited `A`, `B` has all three kinds of link.
#[test]
fn the_uncommitted_diff_is_no_evidence() -> TestResult {
    let nodes = [
        plain(1, NodeKind::File, None),
        plain(2, NodeKind::File, None),
    ];
    let links = [
        link(1, 2, EdgeType::Structural, 1.0),
        link(1, 2, EdgeType::CoChange, 0.9),
        link(1, 2, EdgeType::CoAccess, 5.0),
    ];
    let graph = graph(&nodes, &links)?;
    let a = at(&graph, 1)?;
    let uncommitted = UncommittedMatch {
        seeds: BTreeMap::from([(a, 0.8)]),
        files: BTreeSet::from([a]),
    };
    let only_uncommitted = Seeds {
        uncommitted: uncommitted.clone(),
        ..Seeds::default()
    };
    assert_eq!(evidence(&graph, &only_uncommitted, 2)?, []);
    let edited = Seeds {
        session: SessionMatch {
            seeds: BTreeMap::from([(a, 1.0)]),
            ..SessionMatch::default()
        },
        uncommitted,
        ..Seeds::default()
    };
    assert_eq!(
        evidence(&graph, &edited, 2)?,
        [Evidence::Structural, Evidence::CoChange, Evidence::CoAccess]
    );
    Ok(())
}

/// Each link is held to its floor on its own: `A` (1) refers to `B` (2) at 0.5 and `B` to `A` at
/// 0.5, which add up to 1.0 between them, yet no reference reaches 0.8, so neither is evidence.
#[test]
fn weak_links_do_not_add_up_to_evidence() -> TestResult {
    let nodes = [
        plain(1, NodeKind::File, None),
        plain(2, NodeKind::File, None),
    ];
    let links = [
        link(1, 2, EdgeType::Structural, 0.5),
        link(2, 1, EdgeType::Structural, 0.5),
    ];
    let graph = graph(&nodes, &links)?;
    let seeds = seeds_at(&[at(&graph, 1)?], &[at(&graph, 2)?], &[]);
    assert_eq!(evidence(&graph, &seeds, 1)?, []);
    assert_eq!(evidence(&graph, &seeds, 2)?, []);
    Ok(())
}

/// The floors are held in the weights the graph was weighed by, worked out by hand with references
/// weighing 0.6 and co-access 0.4. `A` (1) refers to `B` (2) at exactly 0.8, kept as 0.6 x 0.8, a
/// 32-bit 0.4799999893, which the floor 0.6 x 0.8, narrowed alike, lets through; `A` refers to `C`
/// (3) at 0.79, which it does not. `A` and `D` (4) were used together to 0.055, kept as 0.4 x 0.055
/// / 5 = 0.0044, above the floor 0.4 x 0.05 / 5 = 0.004 (a floor of 0.05 / 5 alone would refuse
/// it); `A` and `E` (5) to 0.045, below it.
#[test]
fn the_floors_are_held_in_the_graph_s_weights() -> TestResult {
    let nodes: Vec<NodeInput<i64>> = (1..=5).map(|id| plain(id, NodeKind::File, None)).collect();
    let links = [
        link(1, 2, EdgeType::Structural, 0.8),
        link(1, 3, EdgeType::Structural, 0.79),
        link(1, 4, EdgeType::CoAccess, 0.055),
        link(1, 5, EdgeType::CoAccess, 0.045),
    ];
    let mut weighing = defaults();
    weighing.type_weights.structural = 0.6;
    weighing.type_weights.coaccess = 0.4;
    let graph = Graph::build(&nodes, &links, &weighing)?;
    let n = |id| at(&graph, id);
    let seeds = seeds_at(&[n(1)?], &[n(2)?, n(3)?, n(4)?, n(5)?], &[]);
    assert_eq!(evidence(&graph, &seeds, 2)?, [Evidence::Structural]);
    assert_eq!(evidence(&graph, &seeds, 3)?, []);
    assert_eq!(evidence(&graph, &seeds, 4)?, [Evidence::CoAccess]);
    assert_eq!(evidence(&graph, &seeds, 5)?, []);
    Ok(())
}

/// A co-access link is held to its floor as it has faded, worked out by hand ten tasks on at
/// 0.995 a task (0.995^10 = 0.9511101): `A` (1) and `B` (2) were used together to 0.053 at task
/// 0, 0.0504 now, enough; `A` and `C` (3) to 0.052, 0.0495 now, not.
#[test]
fn a_co_access_link_counts_as_it_has_faded() -> TestResult {
    let nodes: Vec<NodeInput<i64>> = (1..=3).map(|id| plain(id, NodeKind::File, None)).collect();
    let links = [
        link(1, 2, EdgeType::CoAccess, 0.053),
        link(1, 3, EdgeType::CoAccess, 0.052),
    ];
    let mut weighing = defaults();
    weighing.latest_seq = 10;
    let graph = Graph::build(&nodes, &links, &weighing)?;
    let n = |id| at(&graph, id);
    let seeds = seeds_at(&[n(1)?], &[n(2)?, n(3)?], &[]);
    assert_eq!(evidence(&graph, &seeds, 2)?, [Evidence::CoAccess]);
    assert_eq!(evidence(&graph, &seeds, 3)?, []);
    Ok(())
}

/// Evidence reaches a file through its symbols and only from seeds of the words, the session and
/// earlier work. `a1` (2) in `A` (1) and `b1` (4) in `B` (3) were used together to 0.25, so `A`
/// and `B` each have co-access. `A` refers to `X` (5) at full confidence, but `X` is only an
/// exploration seed, so that is no evidence for `A`; `X` itself is joined to `A`, a seed of the
/// words, and has the reference. `A` and `D` (6) change together, the miner having written both
/// directions, which is one kind of evidence for each.
#[test]
fn evidence_reaches_a_file_through_its_symbols_from_real_seeds_only() -> TestResult {
    let nodes = [
        plain(1, NodeKind::File, None),
        plain(2, NodeKind::Symbol, Some(1)),
        plain(3, NodeKind::File, None),
        plain(4, NodeKind::Symbol, Some(3)),
        plain(5, NodeKind::File, None),
        plain(6, NodeKind::File, None),
    ];
    let links = [
        link(2, 4, EdgeType::CoAccess, 0.25),
        link(1, 5, EdgeType::Structural, 1.0),
        link(1, 6, EdgeType::CoChange, 0.5),
        link(6, 1, EdgeType::CoChange, 0.4),
    ];
    let graph = graph(&nodes, &links)?;
    let n = |id| at(&graph, id);
    let seeds = seeds_at(&[n(2)?], &[n(3)?, n(6)?], &[n(5)?]);
    assert_eq!(
        evidence(&graph, &seeds, 1)?,
        [Evidence::CoChange, Evidence::CoAccess]
    );
    assert_eq!(evidence(&graph, &seeds, 3)?, [Evidence::CoAccess]);
    assert_eq!(evidence(&graph, &seeds, 5)?, [Evidence::Structural]);
    assert_eq!(evidence(&graph, &seeds, 6)?, [Evidence::CoChange]);
    Ok(())
}

/// Files that score alike come by node, and so do symbols that score alike: `A` (1, holding `a1`
/// 2 and `a2` 3) scores 0.25 + 0.125 + 0.125 = 0.5 and `B` (4) 0.5, `a1` and `a2` 0.125 each, all
/// exact in binary. Nothing kept is nothing scored.
#[test]
fn ties_are_broken_by_node() -> TestResult {
    let nodes = [
        plain(1, NodeKind::File, None),
        plain(2, NodeKind::Symbol, Some(1)),
        plain(3, NodeKind::Symbol, Some(1)),
        plain(4, NodeKind::File, None),
    ];
    let graph = graph(&nodes, &[])?;
    let n = |id| at(&graph, id);
    let activation = |kept: Vec<(u32, f64)>| Activation {
        kept,
        pushes: 0,
        left: 0.0,
        is_cut_short: false,
        is_out_of_time: false,
    };
    let kept = vec![(n(1)?, 0.25), (n(2)?, 0.125), (n(3)?, 0.125), (n(4)?, 0.5)];
    let scored = scored_files(&graph, &activation(kept));
    let files: Vec<u32> = scored.iter().map(|file| file.file).collect();
    assert_eq!(files, [n(1)?, n(4)?]);
    let symbols: Vec<u32> = scored
        .first()
        .ok_or("a file")?
        .symbols
        .iter()
        .map(|symbol| symbol.0)
        .collect();
    assert_eq!(symbols, [n(2)?, n(3)?]);
    assert!(scored_files(&graph, &activation(Vec::new())).is_empty());
    Ok(())
}
