//! History seeds on a small scripted history, every expected seed worked out by hand: the changes
//! a prompt names by number or by commit, the past changes whose words are like the prompt's
//! (scored against the most a card could score, and held to the bar), the same seeds whatever
//! order the cards were stored in, and the gate's rule that history alone never clears it.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{Repo, TestResult};
use cortex_core::{EdgeType, FileFlags, NodeKind, TypeWeights};
use cortex_retrieve::{
    Activation, Confidence, FileMatch, Found, Graph, GraphSettings, HistoryAsk, HistoryMatch,
    LexicalMatch, LinkInput, NodeInput, PromptTerms, REFERENCE_WEIGHT, Rule, ScoredFile, Seeds,
    Shut, Standing, Support, Verdict, Weighing, decide, features_of, history_seeds,
    is_history_alone, read_graph, scored_changes,
};
use cortex_store::{CardKind, ChangeCardRecord, NodeId};

/// The files of the history, each stored with a node.
const PATHS: [&str; 4] = ["src/tax.py", "src/money.py", "src/retry.py", "docs/tax.md"];

/// A card of the commit `oid`, of `kind`, that changed `files`, from pull request `pull_request`.
fn card(
    oid: &str,
    kind: CardKind,
    files: Vec<NodeId>,
    pull_request: Option<u64>,
) -> ChangeCardRecord {
    ChangeCardRecord {
        oid: oid.to_owned(),
        first_parent: None,
        committed_at_s: 1_000,
        kind,
        pull_request,
        issues: Vec::new(),
        has_reason: true,
        files,
        symbols: Vec::new(),
        tests: Vec::new(),
        carded_at_ms: 1,
    }
}

/// The scripted history, ten cards of four words each, so that every card is as long as the
/// average and BM25 weighs a word a card holds once at exactly its inverse document frequency:
/// - `1111111...`, normal, pull request #482: `quokka wombat numbat bilby`; changed `src/tax.py`
///   and `src/money.py`;
/// - `a1b2c3d4...`, normal, fixing #471: `dingo emu koala possum`; changed `src/retry.py`;
/// - `a1b2c3e5...`, normal: `dingo emu koala possum` too; changed nothing the index holds;
/// - `3333333...`, a formatting change: `quokka wombat dingo emu`; changed `docs/tax.md`;
/// - six normal fillers, `4444444...` to `9999999...`: `alpha beta gamma delta`, changing nothing.
fn cards(nodes: &[NodeId]) -> TestResult<Vec<(ChangeCardRecord, Vec<String>)>> {
    let node = |at: usize| -> TestResult<NodeId> { Ok(*nodes.get(at).ok_or("a node")?) };
    let words = |text: &str| text.split(' ').map(str::to_owned).collect::<Vec<String>>();
    let mut fixed = card(
        &format!("a1b2c3d4{}", "0".repeat(32)),
        CardKind::Normal,
        vec![node(2)?],
        None,
    );
    fixed.issues = vec!["#471".to_owned()];
    let mut all = vec![
        (
            card(
                &"1".repeat(40),
                CardKind::Normal,
                vec![node(0)?, node(1)?],
                Some(482),
            ),
            words("quokka wombat numbat bilby"),
        ),
        (fixed, words("dingo emu koala possum")),
        (
            card(
                &format!("a1b2c3e5{}", "0".repeat(32)),
                CardKind::Normal,
                Vec::new(),
                None,
            ),
            words("dingo emu koala possum"),
        ),
        (
            card(&"3".repeat(40), CardKind::Trivial, vec![node(3)?], None),
            words("quokka wombat dingo emu"),
        ),
    ];
    for digit in '4'..='9' {
        all.push((
            card(
                &digit.to_string().repeat(40),
                CardKind::Normal,
                Vec::new(),
                None,
            ),
            words("alpha beta gamma delta"),
        ));
    }
    Ok(all)
}

/// A store holding [`PATHS`] and the scripted history, its cards stored in their order or, when
/// `is_reversed`, the other way round; and the files' nodes.
fn history(is_reversed: bool) -> TestResult<(Repo, Vec<NodeId>)> {
    let mut repo = Repo::new()?;
    let mut nodes = Vec::new();
    for path in PATHS {
        nodes.push(
            *repo
                .file(path, "x = 1\n", &[], FileFlags::empty())?
                .first()
                .ok_or("a node")?,
        );
    }
    let mut all = cards(&nodes)?;
    if is_reversed {
        all.reverse();
    }
    let mut transaction = repo.writer.transaction()?;
    for (card, words) in &all {
        transaction.put_change_card(card, words)?;
    }
    transaction.commit()?;
    Ok((repo, nodes))
}

/// The seeds `prompt` gets from `repo`'s history at a seed weight of 0.4, by path.
fn seeded(repo: &Repo, prompt: &str) -> TestResult<BTreeMap<String, f64>> {
    seeded_at(repo, prompt, 0.4)
}

/// The seeds `prompt` gets from `repo`'s history at a seed weight of `seed_weight`, by path.
fn seeded_at(repo: &Repo, prompt: &str, seed_weight: f64) -> TestResult<BTreeMap<String, f64>> {
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let terms = PromptTerms::of(prompt);
    let found = history_seeds(
        reader.queries(),
        &graph,
        &HistoryAsk {
            prompt,
            terms: &terms,
            seed_weight,
        },
    )?;
    let mut by_path = BTreeMap::new();
    for (index, weight) in found.seeds {
        let node = graph.id(index).ok_or("a node of the graph")?;
        let (_, path) = reader.queries().node_file(node)?.ok_or("a file")?;
        by_path.insert(path.as_str().to_owned(), weight);
    }
    Ok(by_path)
}

/// Whether `found` holds exactly `expected`, each weight within a billionth.
fn holds(found: &BTreeMap<String, f64>, expected: &[(&str, f64)]) -> bool {
    found.len() == expected.len()
        && expected.iter().all(|(path, weight)| {
            found
                .get(*path)
                .is_some_and(|found| (found - weight).abs() < 1e-9)
        })
}

/// A change named by its pull request (`#482`) or a merge request's mark (`!482`) seeds the two
/// files it changed at 0.8 spread over them, 0.4 each; one named by the issue it fixes (`#471`),
/// or by the start of its id in any case (`a1b2c3d`, `A1B2C3D4`), seeds its one file at 0.8. `a1b2c3e` names the card that changed no file the
/// index holds, which seeds nothing; `a1b2c3`, six digits and the start both ids share, names
/// nothing; nor does a number no card has.
#[test]
fn a_named_change_seeds_the_files_it_changed() -> TestResult {
    let (repo, _) = history(false)?;
    let both = [
        ("src/money.py", REFERENCE_WEIGHT / 2.0),
        ("src/tax.py", REFERENCE_WEIGHT / 2.0),
    ];
    assert!(holds(&seeded(&repo, "Do again what #482 did")?, &both));
    assert!(holds(&seeded(&repo, "Do again what !482 did")?, &both));
    let retry = [("src/retry.py", 0.8)];
    assert!(holds(&seeded(&repo, "Undo #471")?, &retry));
    assert!(holds(&seeded(&repo, "Undo a1b2c3d")?, &retry));
    assert!(holds(&seeded(&repo, "Undo A1B2C3D4")?, &retry));
    assert_eq!(seeded(&repo, "Undo a1b2c3e")?, BTreeMap::new());
    assert_eq!(seeded(&repo, "Undo a1b2c3")?, BTreeMap::new());
    assert_eq!(seeded(&repo, "Undo #999")?, BTreeMap::new());
    // A number past what the store keeps names nothing, and fails nothing.
    assert_eq!(seeded(&repo, "Undo #9223372036854775808")?, BTreeMap::new());
    Ok(())
}

/// A start two cards' ids share names neither: with a card `a1b2c3d9...` added, `a1b2c3d` starts
/// two ids and seeds nothing, while `a1b2c3d4` still names one.
#[test]
fn a_start_two_cards_share_names_neither() -> TestResult {
    let (mut repo, nodes) = history(false)?;
    let mut transaction = repo.writer.transaction()?;
    let twin = card(
        &format!("a1b2c3d9{}", "0".repeat(32)),
        CardKind::Normal,
        vec![*nodes.first().ok_or("a node")?],
        None,
    );
    transaction.put_change_card(&twin, &[])?;
    transaction.commit()?;
    assert_eq!(seeded(&repo, "Undo a1b2c3d")?, BTreeMap::new());
    assert!(holds(
        &seeded(&repo, "Undo a1b2c3d4")?,
        &[("src/retry.py", 0.8)]
    ));
    Ok(())
}

/// Worked by hand over the ten cards: `quokka` and `wombat` are held by 2 cards each, weighing
/// ln(8.5 / 2.5) = ln 3.4 = 1.2238; `numbat` and `bilby` by 1 each, ln(9.5 / 1.5) = 1.8458. The
/// least the whole is taken as is twice the weight of a word one card holds, 3.6917.
/// - "quokka wombat numbat bilby": the whole is 2 x 1.2238 + 2 x 1.8458 = 6.1392; card `1111111`
///   holds all four once and scores as much, 1.0, so its two files share 0.4 x 1.0, 0.2 each;
///   the formatting card holds two of the words but is no normal change.
/// - "quokka numbat": the whole is 1.2238 + 1.8458 = 3.0696, taken as 3.6917; card `1111111`
///   scores 3.0696 / 3.6917 = 0.8315, at the bar or above: 0.4 x 0.8315 = 0.3326, 0.1663 each.
/// - "quokka": 1.2238 / 3.6917 = 0.3315, below the bar of 0.6: nothing.
/// - "quokka wombat dingo emu": `dingo` and `emu` are held by 3 cards each, ln(7.5 / 3.5) =
///   0.7621; the whole is 2 x 1.2238 + 2 x 0.7621 = 3.9718. The formatting card holds all four and
///   would score 1.0, but is no normal change; card `1111111` holds two, 2.4476 / 3.9718 = 0.6162:
///   0.4 x 0.6162 = 0.2465, 0.1232 each; the two cards of `dingo emu` score 0.3838, below the bar.
///
/// At a seed weight of 0 the first prompt seeds nothing, not even at no weight.
#[test]
fn a_past_change_like_the_prompt_seeds_its_files_by_its_score() -> TestResult {
    let (repo, _) = history(false)?;
    let paths = ["src/money.py", "src/tax.py"];
    let at = |weight: f64| paths.map(|path| (path, weight / 2.0));
    assert!(holds(
        &seeded(&repo, "quokka wombat numbat bilby")?,
        &at(0.4)
    ));
    let score = (3.4_f64.ln() + (9.5_f64 / 1.5).ln()) / (2.0 * (9.5_f64 / 1.5).ln());
    assert!((score - 0.8315).abs() < 1e-4);
    let found = seeded(&repo, "quokka numbat")?;
    assert!(holds(&found, &at(0.4 * score)), "{found:?}");
    assert_eq!(seeded(&repo, "quokka")?, BTreeMap::new());
    assert_eq!(seeded(&repo, "nothing like it")?, BTreeMap::new());
    let score = 2.0 * 3.4_f64.ln() / (2.0 * 3.4_f64.ln() + 2.0 * (7.5_f64 / 3.5).ln());
    assert!((score - 0.6162).abs() < 1e-4);
    let found = seeded(&repo, "quokka wombat dingo emu")?;
    assert!(holds(&found, &at(0.4 * score)), "{found:?}");
    assert_eq!(
        seeded_at(&repo, "quokka wombat numbat bilby", 0.0)?,
        BTreeMap::new()
    );
    Ok(())
}

/// A store of files `f1.py` to `f4.py` and these cards, every one of four words, so that they
/// are all as long as the average:
/// - `aaaaaaa...`, a formatting change: `kiwi kiwi kiwi kiwi`, changing `f1.py`;
/// - `bbbbbbb...` to `eeeeeee...`, normal changes of pull request #500, committed at 1 to 4 s:
///   `kiwi plum pear fig`, each changing one file, `f1.py` to `f4.py` in that order;
/// - eight normal fillers, `1111111...` to `8888888...`: `alpha beta gamma delta`.
fn busy_history() -> TestResult<Repo> {
    let mut repo = Repo::new()?;
    let mut nodes = Vec::new();
    for path in ["f1.py", "f2.py", "f3.py", "f4.py"] {
        nodes.push(
            *repo
                .file(path, "x = 1\n", &[], FileFlags::empty())?
                .first()
                .ok_or("a node")?,
        );
    }
    let words = |text: &str| text.split(' ').map(str::to_owned).collect::<Vec<String>>();
    let mut transaction = repo.writer.transaction()?;
    let first = *nodes.first().ok_or("a node")?;
    let format = card(&"a".repeat(40), CardKind::Trivial, vec![first], None);
    transaction.put_change_card(&format, &words("kiwi kiwi kiwi kiwi"))?;
    for ((digit, node), at_s) in ['b', 'c', 'd', 'e'].into_iter().zip(&nodes).zip(1..) {
        let mut change = card(
            &digit.to_string().repeat(40),
            CardKind::Normal,
            vec![*node],
            Some(500),
        );
        change.committed_at_s = at_s;
        transaction.put_change_card(&change, &words("kiwi plum pear fig"))?;
    }
    for digit in '1'..='8' {
        let filler = card(
            &digit.to_string().repeat(40),
            CardKind::Normal,
            Vec::new(),
            None,
        );
        transaction.put_change_card(&filler, &words("alpha beta gamma delta"))?;
    }
    transaction.commit()?;
    Ok(repo)
}

/// The best three normal changes are taken after the other kinds are passed over: `kiwi` ranks
/// the formatting card first (it says the word four times), then the four normal cards alike,
/// in commit order. The three are `bbbbbbb`, `ccccccc` and `ddddddd`, not two of them.
#[test]
fn three_normal_changes_are_taken_past_another_kind() -> TestResult {
    let repo = busy_history()?;
    let reader = repo.reader()?;
    let found = scored_changes(reader.queries(), &PromptTerms::of("kiwi"))?;
    let firsts: String = found
        .iter()
        .filter_map(|(card, _)| card.oid.chars().next())
        .collect();
    assert_eq!(firsts, "bcd");
    Ok(())
}

/// A number four changes answer to seeds the files of its newest three only: #500 seeds
/// `f4.py`, `f3.py` and `f2.py` (committed at 4, 3 and 2 s), each change's 0.8 on its one file,
/// and not `f1.py`.
#[test]
fn a_number_seeds_its_newest_three_changes() -> TestResult {
    let repo = busy_history()?;
    let found = seeded(&repo, "Redo #500")?;
    assert!(
        holds(&found, &[("f2.py", 0.8), ("f3.py", 0.8), ("f4.py", 0.8)]),
        "{found:?}"
    );
    Ok(())
}

/// The same history stored in the other order seeds the same files at the same weights, bit for
/// bit, for every prompt above.
#[test]
fn the_seeds_do_not_hang_on_the_order_the_cards_came_in() -> TestResult {
    let (forward, _) = history(false)?;
    let (backward, _) = history(true)?;
    for prompt in [
        "quokka wombat numbat bilby",
        "quokka numbat",
        "dingo emu",
        "Undo a1b2c3d and #482",
    ] {
        let (one, other) = (seeded(&forward, prompt)?, seeded(&backward, prompt)?);
        let bits = |found: &BTreeMap<String, f64>| -> Vec<(String, u64)> {
            found
                .iter()
                .map(|(path, weight)| (path.clone(), weight.to_bits()))
                .collect()
        };
        assert_eq!(bits(&one), bits(&other), "{prompt}");
    }
    Ok(())
}

/// Files `A` (1), `B` (2) and `C` (3), `A` and `B` changing together, used together and `A` calling
/// into `B`: the links a past change that touched both would leave.
fn linked() -> TestResult<Graph<i64>> {
    let file = |id| NodeInput {
        id,
        kind: NodeKind::File,
        file: None,
        standing: Standing::default(),
    };
    let link = |src, dst, kind, weight| LinkInput {
        src,
        dst,
        kind,
        weight,
        last_seq: 0,
    };
    let links = [
        link(1, 2, EdgeType::CoChange, 0.5),
        link(2, 1, EdgeType::CoChange, 0.5),
        link(1, 2, EdgeType::CoAccess, 0.25),
        link(1, 2, EdgeType::Structural, 1.0),
    ];
    let weighing = Weighing {
        type_weights: TypeWeights::default(),
        latest_seq: 0,
        link_decay: 0.995,
    };
    Ok(Graph::build(
        &[file(1), file(2), file(3)],
        &links,
        &weighing,
    )?)
}

/// What the gate says of `seeds` over `graph`, the files scored `A`, `B`, `C`, with no raise and
/// no model; and the kinds of evidence behind `A`.
fn verdict(
    graph: &Graph<i64>,
    seeds: &Seeds,
    model: Option<Confidence>,
) -> TestResult<(Verdict, BTreeSet<cortex_retrieve::Evidence>)> {
    let n = |id| graph.index_of(id).ok_or("a node of the graph");
    let scored = |file, activation| ScoredFile {
        file,
        score: activation,
        activation,
        symbols: Vec::new(),
    };
    let files = [scored(n(1)?, 0.5), scored(n(2)?, 0.3), scored(n(3)?, 0.2)];
    let activation = Activation {
        kept: files
            .iter()
            .map(|file| (file.file, file.activation))
            .collect(),
        pushes: 0,
        left: 0.0,
        is_cut_short: false,
        is_out_of_time: false,
    };
    let support = Support::new(graph, seeds);
    let found = Found {
        seeds,
        activation: &activation,
        scored: &files,
        support: &support,
    };
    let features = features_of(&found, None);
    let said = decide(&features, is_history_alone(&found), 0.0, model);
    Ok((said, support.of(n(1)?)))
}

/// History is one kind and never clears the gate alone. A past change seeded `A` and `B`, whose
/// links to each other count for nothing, since neither is a seed of another source:
/// - with nothing else, `A` has history alone, one kind: silent, too few kinds, and so with a kept
///   model that puts the chance at 0.99, refused before the model is asked;
/// - with the prompt's words matching `C` at 0.2, `C` has the words alone and `A` history alone:
///   silent (were the change's files seeds for the links, `A` would have four kinds, and three with
///   a match of 0.1 clear);
/// - with the words matching `A` at 0.6, history and the words are two kinds agreeing on `A` with
///   a strong match: history counts alongside another kind, and a kept model is then asked.
#[test]
fn history_alone_never_clears_the_gate() -> TestResult {
    use cortex_retrieve::Evidence;
    let graph = linked()?;
    let n = |id| graph.index_of(id).ok_or("a node of the graph");
    let history = HistoryMatch {
        seeds: BTreeMap::from([(n(1)?, 0.4), (n(2)?, 0.4)]),
    };
    let alone = Seeds {
        history,
        ..Seeds::default()
    };
    let (said, kinds) = verdict(&graph, &alone, None)?;
    assert_eq!(kinds, BTreeSet::from([Evidence::History]));
    assert_eq!(said, Verdict::Silent(Shut::TooFewKinds));
    let sure = Some(Confidence {
        probability: 0.99,
        bar: 0.7,
    });
    assert_eq!(
        verdict(&graph, &alone, sure)?.0,
        Verdict::Silent(Shut::TooFewKinds)
    );
    let matched = |seeds: &Seeds, file: u32, coverage: f64| -> Seeds {
        let mut seeds = seeds.clone();
        seeds.lexical = LexicalMatch {
            seeds: BTreeMap::from([(file, 1.0)]),
            files: BTreeMap::from([(
                file,
                FileMatch {
                    coverage,
                    words: vec!["tax".to_owned()],
                    is_named: false,
                },
            )]),
            top_lexical: coverage,
        };
        seeds
    };
    let (said, kinds) = verdict(&graph, &matched(&alone, n(3)?, 0.2), None)?;
    assert_eq!(kinds, BTreeSet::from([Evidence::History]));
    assert_eq!(said, Verdict::Silent(Shut::TooFewKinds));
    let (said, kinds) = verdict(&graph, &matched(&alone, n(1)?, 0.6), None)?;
    assert_eq!(
        kinds,
        BTreeSet::from([Evidence::Lexical, Evidence::History])
    );
    assert_eq!(said, Verdict::Inject(Rule::Agreement));
    assert_eq!(
        verdict(&graph, &matched(&alone, n(1)?, 0.6), sure)?.0,
        Verdict::Inject(Rule::Calibrated)
    );
    Ok(())
}

/// A retrieval's seeds take history only while `history.seeds` is on: with a seed weight, the
/// change `#482` names seeds `src/tax.py` and `src/money.py` (0.4 each, its 0.8 spread over them),
/// which the prompt's words do not find,
/// and both are in the vector activation starts from, at equal shares; with none, nothing is
/// seeded from history and, the words finding nothing, there is no vector.
#[test]
fn the_seeds_take_history_only_while_it_is_on() -> TestResult {
    use cortex_retrieve::{SeedQuery, seeds};
    let (repo, nodes) = history(false)?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let prompt = "Do again what #482 did";
    let terms = PromptTerms::of(prompt);
    let query = |history_seed_weight| SeedQuery {
        prompt,
        terms: &terms,
        session: None,
        uncommitted: &[],
        failure: &[],
        history_seed_weight,
    };
    let index = |at: usize| -> TestResult<u32> {
        let node = *nodes.get(at).ok_or("a node")?;
        Ok(graph.index_of(node).ok_or("a node of the graph")?)
    };
    let (tax, money) = (index(0)?, index(1)?);
    let on = seeds(reader.queries(), &graph, &query(Some(0.4)))?;
    assert_eq!(
        on.history.seeds,
        BTreeMap::from([
            (tax, REFERENCE_WEIGHT / 2.0),
            (money, REFERENCE_WEIGHT / 2.0)
        ])
    );
    assert!(on.lexical.seeds.is_empty());
    let shares: Vec<(u32, f64)> = on
        .vector
        .iter()
        .map(|(node, share)| (*node, *share))
        .collect();
    assert_eq!(shares.len(), 2, "{shares:?}");
    assert!(
        shares
            .iter()
            .all(|(node, share)| [tax, money].contains(node) && (share - 0.5).abs() < 1e-12)
    );
    let off = seeds(reader.queries(), &graph, &query(None))?;
    assert_eq!(off.history, HistoryMatch::default());
    assert!(off.vector.is_empty());
    Ok(())
}
