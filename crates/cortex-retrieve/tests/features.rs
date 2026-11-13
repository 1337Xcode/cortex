//! The gate's features from one retrieval, worked out by hand on a graph built by hand.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{EdgeType, NodeKind, TypeWeights};
use cortex_retrieve::{
    Activation, FileMatch, Found, Graph, LexicalMatch, LinkInput, NodeInput, ScoredFile, Seeds,
    SessionMatch, SimilarMatch, Standing, Support, UncommittedMatch, Weighing, features_of,
};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Files `A` to `D` (1 to 4) and a folder (5). `A` and `B` were used together to 0.25, `A` calls
/// into `C` at full confidence, and `A` changes with `D`.
fn graph() -> TestResult<Graph<i64>> {
    let file = |id| NodeInput {
        id,
        kind: NodeKind::File,
        file: None,
        standing: Standing::default(),
    };
    let nodes = [
        file(1),
        file(2),
        file(3),
        file(4),
        NodeInput {
            kind: NodeKind::Dir,
            ..file(5)
        },
    ];
    let link = |src, dst, kind, weight| LinkInput {
        src,
        dst,
        kind,
        weight,
        last_seq: 0,
    };
    let links = [
        link(1, 2, EdgeType::CoAccess, 0.25),
        link(1, 3, EdgeType::Structural, 1.0),
        link(1, 4, EdgeType::CoChange, 0.5),
    ];
    let weighing = Weighing {
        type_weights: TypeWeights::default(),
        latest_seq: 0,
        link_decay: 0.995,
    };
    Ok(Graph::build(nodes.to_vec(), links.to_vec(), &weighing)?)
}

/// A scored file `file` holding `activation` of the activation, scoring as much.
fn scored(file: u32, activation: f64) -> ScoredFile {
    ScoredFile {
        file,
        score: activation,
        activation,
        symbols: Vec::new(),
    }
}

/// The prompt's words found `A` (top lexical 0.55); the session edited `B`; earlier work changed
/// `C` (best overlap 0.4). Worked by hand: `A` has the words, a reference to `C` and co-access with
/// `B`, three kinds (its co-change with `D` says nothing, no seed pointing at `D`); `B` co-access
/// with `A`, one; `C` a reference from `A` and earlier work, two; `D`, fourth, is not counted,
/// though `A`'s co-change would give it a fifth kind. So three kinds agree on one file, and four
/// kinds stand behind the top three in all. Of the 1.0 of activation kept, the folder holding 0.15, the top five files
/// hold 0.4 + 0.3 + 0.1 + 0.05 = 0.85. The session edited `B`, which the words did not find, so it
/// is not continuing; once the words find `B` too, it is. With no session edit, the work tree's
/// uncommitted change to `C`, which the words also find, is continuing work too; one to `D`, fourth
/// and so past the three the gate weighs, is not.
#[test]
fn the_features_are_worked_out_from_what_was_found() -> TestResult {
    let graph = graph()?;
    let n = |id| graph.index_of(id).ok_or("a node of the graph");
    let matched = FileMatch {
        coverage: 0.55,
        words: vec!["order".to_owned()],
        is_named: false,
    };
    let mut seeds = Seeds {
        lexical: LexicalMatch {
            seeds: BTreeMap::from([(n(1)?, 1.0)]),
            files: BTreeMap::from([(n(1)?, matched.clone())]),
            top_lexical: 0.55,
        },
        session: SessionMatch {
            seeds: BTreeMap::from([(n(2)?, 1.0)]),
            edited_files: BTreeSet::from([n(2)?]),
            in_context: Vec::new(),
            shown: Vec::new(),
        },
        similar: SimilarMatch {
            seeds: BTreeMap::from([(n(3)?, 0.24)]),
            files: BTreeMap::from([(n(3)?, (0.4, 2))]),
            best: 0.4,
        },
        ..Seeds::default()
    };
    let activation = Activation {
        kept: vec![
            (n(1)?, 0.4),
            (n(2)?, 0.3),
            (n(3)?, 0.1),
            (n(4)?, 0.05),
            (n(5)?, 0.15),
        ],
        pushes: 0,
        left: 0.0,
        is_cut_short: false,
        is_out_of_time: false,
    };
    let files = [
        scored(n(1)?, 0.4),
        scored(n(2)?, 0.3),
        scored(n(3)?, 0.1),
        scored(n(4)?, 0.05),
    ];
    let features = {
        let support = Support::new(&graph, &seeds);
        let found = Found {
            seeds: &seeds,
            activation: &activation,
            scored: &files,
            support: &support,
        };
        features_of(&found, Some(0.8))
    };
    assert!((features.top_lexical - 0.55).abs() < 1e-12);
    assert!((features.top_mass - 0.85).abs() < 1e-12);
    assert_eq!(features.agreeing_kinds, 3);
    assert_eq!(features.supporting_kinds, 4);
    assert!((features.similar_task - 0.4).abs() < 1e-12);
    assert_eq!(features.recent_precision, Some(0.8));
    assert!(!features.is_continuing);
    seeds.lexical.files.insert(n(2)?, matched.clone());
    let support = Support::new(&graph, &seeds);
    let found = Found {
        seeds: &seeds,
        activation: &activation,
        scored: &files,
        support: &support,
    };
    assert!(features_of(&found, None).is_continuing);
    let continuing = |uncommitted: u32| -> TestResult<bool> {
        let mut seeds = seeds.clone();
        seeds.session = SessionMatch::default();
        seeds.uncommitted = UncommittedMatch {
            seeds: BTreeMap::from([(uncommitted, 0.8)]),
            files: BTreeSet::from([uncommitted]),
        };
        seeds.lexical.files.insert(uncommitted, matched.clone());
        let support = Support::new(&graph, &seeds);
        let found = Found {
            seeds: &seeds,
            activation: &activation,
            scored: &files,
            support: &support,
        };
        Ok(features_of(&found, None).is_continuing)
    };
    assert!(continuing(n(3)?)?);
    assert!(!continuing(n(4)?)?);
    Ok(())
}

/// Nothing found is nothing in every feature: no kinds, no mass, no continuity.
#[test]
fn nothing_found_has_no_features() -> TestResult {
    let graph = graph()?;
    let seeds = Seeds::default();
    let activation = Activation {
        kept: Vec::new(),
        pushes: 0,
        left: 0.0,
        is_cut_short: false,
        is_out_of_time: false,
    };
    let support = Support::new(&graph, &seeds);
    let found = Found {
        seeds: &seeds,
        activation: &activation,
        scored: &[],
        support: &support,
    };
    let features = features_of(&found, None);
    assert_eq!((features.agreeing_kinds, features.supporting_kinds), (0, 0));
    assert!(features.top_mass.abs() < 1e-12);
    assert!(features.top_lexical.abs() < 1e-12);
    assert!(!features.is_continuing);
    Ok(())
}
