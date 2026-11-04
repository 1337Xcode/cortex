//! The clusters a repository's task records fall into: which tasks belong together, which files
//! and words each cluster is known by, and the bounds on what is kept.

mod common;

use common::{Store, TestResult};
use cortex_core::{Agent, Identifier, TaskOutcome, Ulid};
use cortex_learn::{Commands, Recorder, archetypes_of};
use cortex_store::{ChangedFile, EpisodeRecord, NodeId, Pitfalls, SessionRecord, TaskRecord};

/// The files of a repository of six, two under `shop/` and `tax.py` and `mail.py` beside them,
/// with `calc/ops.py` and `calc/util.py` apart from the rest.
const PATHS: [&str; 6] = [
    "shop/orders.py",
    "shop/lines.py",
    "tax.py",
    "mail.py",
    "calc/ops.py",
    "calc/util.py",
];

/// What one cluster says: the words it is found by, its files, and how many tasks it covers.
type Said = (Vec<String>, Vec<NodeId>, u32);

/// What one cluster covers: its files, and how many tasks.
type Shape = (Vec<NodeId>, u32);

/// What each cluster says.
fn said(store: &Store, (limit, at_ms): (usize, i64)) -> TestResult<Vec<Said>> {
    Ok(archetypes_of(store.writer.queries(), (limit, at_ms))?
        .into_iter()
        .map(|cluster| {
            (
                cluster
                    .seed_terms
                    .iter()
                    .map(|term| term.as_str().to_owned())
                    .collect(),
                cluster.files,
                cluster.episode_count,
            )
        })
        .collect())
}

/// What each cluster covers.
fn shapes(store: &Store, limit: usize) -> TestResult<Vec<Shape>> {
    Ok(said(store, (limit, 9_000))?
        .into_iter()
        .map(|(_, files, tasks)| (files, tasks))
        .collect())
}

/// `terms` as owned words.
fn words(terms: &[&str]) -> Vec<String> {
    terms.iter().map(|term| (*term).to_owned()).collect()
}

/// Tasks that changed much the same files fall into one cluster, worked out by hand. Of the six
/// files, taking `f0` to `f5` in the order above, eight tasks change:
///
/// - `f0`, `f1`, `f2`, which starts a cluster whose files the rest are compared against;
/// - `f0`, `f1`, `f3`: both changed `f0` and `f1`, and between them four files, which is half, so
///   it joins;
/// - `f4`, `f5`: nothing in common with the first cluster, so it starts one of its own;
/// - `f0`, `f2`: both changed two of the three files between them, two thirds, so it joins the
///   first;
/// - nothing at all, which belongs to no cluster;
/// - `f4`: both changed one of the two files between them, half, so it joins the second cluster;
/// - `f0`, `f3`, `f4`: one of five with the first cluster and one of four with the second, neither
///   of them half, so it starts a third;
/// - `f2`, `f3`: one of four with the first cluster, which is what the first task of it changed,
///   and one of four with the third, so it starts a fourth. The three tasks of the first cluster
///   changed `f3` between them, so a cluster that took in every file of every task that joined it
///   would have taken this one in.
///
/// So the first cluster covers three tasks and the second two, and the last two one each, the
/// fourth first as it holds the newer work. The first's files are those its tasks changed, the
/// ones most of them changed first and ties going to the smaller node: `f0` for three tasks, then
/// `f1` and `f2` for two, then `f3` for one.
///
/// No file here has a symbol, so every word is a word of a path. Of the seven tasks clustered,
/// `py` comes from all seven, `shop` and `orders` from four, `tax`, `mail`, `calc` and `ops` from
/// three, `lines` from two and `util` from one. A word scores how many of a cluster's tasks gave
/// it, times that count over how many of the seven did: in the first cluster `shop` and `orders`
/// score 3 x 3 / 4 = 2.25, `lines` 2 x 2 / 2 = 2, `tax` 2 x 2 / 3, about 1.33, `py` 3 x 3 / 7,
/// about 1.29, and `mail` 1 / 3, so `py` falls behind words fewer tasks gave. In the second, `calc`
/// and `ops` score 2 x 2 / 3, `util` 1 and `py` 4 / 7. In the fourth, `tax` and `mail` score a
/// third each, the one met first first, and `py` a seventh; in the third, `mail`, `calc` and `ops`
/// a third, then `shop` and `orders` a quarter, then `py`.
#[test]
fn tasks_that_changed_the_same_files_fall_into_one_cluster() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let nodes = store.index(&PATHS)?;
    let node = |at: usize| nodes.get(at).copied().ok_or("no such file");
    let (f0, f1, f2, f3, f4, f5) = (node(0)?, node(1)?, node(2)?, node(3)?, node(4)?, node(5)?);
    let changed = [
        vec!["shop/orders.py", "shop/lines.py", "tax.py"],
        vec!["shop/orders.py", "shop/lines.py", "mail.py"],
        vec!["calc/ops.py", "calc/util.py"],
        vec!["shop/orders.py", "tax.py"],
        vec![],
        vec!["calc/ops.py"],
        vec!["shop/orders.py", "mail.py", "calc/ops.py"],
        vec!["tax.py", "mail.py"],
    ];
    for (at, paths) in changed.iter().enumerate() {
        let at_ms = 1_000 + i64::try_from(at)? * 1_000;
        store.task_that_changed(&mut recorder, ("s1", at_ms), paths)?;
    }
    let clusters = said(&store, (100, 9_000))?;
    assert_eq!(
        clusters,
        [
            (
                words(&["shop", "orders", "lines", "tax", "py", "mail"]),
                vec![f0, f1, f2, f3],
                3,
            ),
            (words(&["calc", "ops", "util", "py"]), vec![f4, f5], 2),
            (words(&["tax", "mail", "py"]), vec![f2, f3], 1),
            (
                words(&["mail", "calc", "ops", "shop", "orders", "py"]),
                vec![f0, f3, f4],
                1,
            ),
        ]
    );
    assert_eq!(said(&store, (100, 9_000))?, clusters, "the same again");
    Ok(())
}

/// Only the newest records the clustering is asked for are clustered: of three tasks, two that
/// changed `shop/orders.py` and then one that changed `tax.py`, reading one record gives the
/// cluster of the last task alone, and reading all three gives the first two together and the
/// last apart.
#[test]
fn the_clustering_reads_only_the_newest_records_it_is_asked_for() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let nodes = store.index(&PATHS)?;
    let (f0, f2) = (
        nodes.first().copied().ok_or("no file")?,
        nodes.get(2).copied().ok_or("no file")?,
    );
    let changed = ["shop/orders.py", "shop/orders.py", "tax.py"];
    for (at, path) in changed.iter().enumerate() {
        let at_ms = 1_000 + i64::try_from(at)? * 1_000;
        store.task_that_changed(&mut recorder, ("s1", at_ms), &[path])?;
    }
    assert_eq!(shapes(&store, 1)?, [(vec![f2], 1)]);
    assert_eq!(shapes(&store, 3)?, [(vec![f0], 2), (vec![f2], 1)]);
    Ok(())
}

/// Only tasks whose work stood are clustered: five tasks change `tax.py` and end as a success, a
/// failure, put back, left before they ended, and with nothing to vouch for them either way. The
/// first and last make one cluster of two tasks; the other three are in none, and `tax` and `py`
/// are given by two tasks of the two clustered.
#[test]
fn only_tasks_whose_work_stood_are_clustered() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let nodes = store.index(&PATHS)?;
    let f2 = nodes.get(2).copied().ok_or("no file")?;
    let outcomes = [
        TaskOutcome::Success,
        TaskOutcome::Failure,
        TaskOutcome::Reverted,
        TaskOutcome::Abandoned,
        TaskOutcome::Unknown,
    ];
    for (at, outcome) in outcomes.into_iter().enumerate() {
        let at_ms = 1_000 + i64::try_from(at)? * 1_000;
        store.task_ended_as(&mut recorder, ("s1", at_ms), &["tax.py"], outcome)?;
    }
    assert_eq!(
        said(&store, (100, 9_000))?,
        [(words(&["tax", "py"]), vec![f2], 2)]
    );
    Ok(())
}

/// A task joins the cluster it is most like, the older on a tie. Of nine files `a.py` to `i.py`,
/// six tasks change:
///
/// - `a`, `b`, `c`, `d`, which starts the first cluster;
/// - `c`, `d`, `e`: two of the five files between them with the first, less than half, so it
///   starts the second;
/// - `a`, `c`, `d`, `e`: three of five with the first, three of four with the second, so it joins
///   the second, which it is more like, though the first would have taken it too;
/// - `f`, `g`, which shares no file with either and starts the third;
/// - `h`, `i`, which starts the fourth;
/// - `f`, `g`, `h`, `i`: two of four with the third and two of four with the fourth, a tie, so it
///   joins the third, the older.
///
/// The second and third clusters cover two tasks each and come first, the third first as it holds
/// the newest task, then the fourth and the first with one each, in the same way. The second's
/// files are `c`, `d` and `e`, changed by both of its tasks, then `a`; the third's are `f` and
/// `g`, then `h` and `i`.
#[test]
fn a_task_joins_the_cluster_it_is_most_like() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let paths = [
        "a.py", "b.py", "c.py", "d.py", "e.py", "f.py", "g.py", "h.py", "i.py",
    ];
    let nodes = store.index(&paths)?;
    let node = |at: usize| nodes.get(at).copied().ok_or("no such file");
    let (a, b, c, d, e) = (node(0)?, node(1)?, node(2)?, node(3)?, node(4)?);
    let (f, g, h, i) = (node(5)?, node(6)?, node(7)?, node(8)?);
    let changed = [
        vec!["a.py", "b.py", "c.py", "d.py"],
        vec!["c.py", "d.py", "e.py"],
        vec!["a.py", "c.py", "d.py", "e.py"],
        vec!["f.py", "g.py"],
        vec!["h.py", "i.py"],
        vec!["f.py", "g.py", "h.py", "i.py"],
    ];
    for (at, paths) in changed.iter().enumerate() {
        let at_ms = 1_000 + i64::try_from(at)? * 1_000;
        store.task_that_changed(&mut recorder, ("s1", at_ms), paths)?;
    }
    assert_eq!(
        shapes(&store, 100)?,
        [
            (vec![f, g, h, i], 2),
            (vec![c, d, e, a], 2),
            (vec![h, i], 1),
            (vec![a, b, c, d], 1),
        ]
    );
    Ok(())
}

/// Ties are broken by newer work and by more tasks. Four tasks change, in order, `lone.py` and
/// `shared/a.py`; `shared/b.py`; `shared/b.py`; and `shared/a.py`. The first and last make one
/// cluster, half alike, and the middle two another, so the clusters cover two tasks each and the
/// first comes first, as its newest task is the newest of all, though it was made before the
/// other. Its words: `a`, from both its tasks and no other, scores 2 x 2 / 2 = 2; `py` and
/// `shared`, from both its tasks and all four, 2 x 2 / 4 = 1; `lone`, from one task and no other,
/// 1 x 1 / 1 = 1. Of the three that score 1, `py` and `shared` come first as two tasks gave them,
/// in the order they were met, then `lone`, though it was met first of all. The other cluster's
/// words are `b`, scoring 2, then `shared` and `py`, scoring 1, in the order they were met.
#[test]
fn ties_go_to_newer_work_and_to_words_more_tasks_gave() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let nodes = store.index(&["lone.py", "shared/a.py", "shared/b.py"])?;
    let node = |at: usize| nodes.get(at).copied().ok_or("no such file");
    let (lone, a, b) = (node(0)?, node(1)?, node(2)?);
    let changed = [
        vec!["lone.py", "shared/a.py"],
        vec!["shared/b.py"],
        vec!["shared/b.py"],
        vec!["shared/a.py"],
    ];
    for (at, paths) in changed.iter().enumerate() {
        let at_ms = 1_000 + i64::try_from(at)? * 1_000;
        store.task_that_changed(&mut recorder, ("s1", at_ms), paths)?;
    }
    assert_eq!(
        said(&store, (100, 9_000))?,
        [
            (words(&["a", "py", "shared", "lone"]), vec![a, lone], 2),
            (words(&["b", "shared", "py"]), vec![b], 2),
        ]
    );
    Ok(())
}

/// A cluster is found by the words of the files and the symbols its tasks changed, as the search
/// index holds them. `billing/refund.py` holds a function `issueRefund` on lines 1 to 3, which a
/// change to its first line touches; `notify.py` holds nothing. One task changes `notify.py` and
/// then `billing/refund.py`, which gives, in that order, `notify` and `py`, then `billing` and
/// `refund`, then `issue` from the function; a second task changes `billing/refund.py` alone, one
/// of the two files the first changed, half, so it joins, and gives `billing`, `refund`, `py` and
/// `issue`. A word counts once for each task that gives it, however many of the task's files and
/// symbols hold it, so `refund`, in a file's path and a function's name, counts two and not four.
/// With one cluster every word's tasks are all the cluster's own, so a word scores how many of
/// them gave it: `py`, `billing`, `refund` and `issue` two, in the order they were first met, then
/// `notify` one, though it was met first of all.
#[test]
fn a_cluster_is_found_by_the_words_most_of_its_tasks_gave() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let notify = store.index(&["notify.py"])?;
    let refund = store.index_symbols("billing/refund.py", &[("issueRefund", 1, 3)])?;
    let (notify, refund) = (
        notify.first().copied().ok_or("no file")?,
        refund.first().copied().ok_or("no file")?,
    );
    store.task_that_changed(
        &mut recorder,
        ("s1", 1_000),
        &["notify.py", "billing/refund.py"],
    )?;
    store.task_that_changed(&mut recorder, ("s1", 2_000), &["billing/refund.py"])?;
    assert_eq!(
        said(&store, (100, 9_000))?,
        [(
            words(&["py", "billing", "refund", "issue", "notify"]),
            vec![refund, notify],
            2,
        )]
    );
    Ok(())
}

/// A word too long to be an identifier is not a word a cluster is found by: a file whose name is
/// 300 letters gives its cluster `py` alone.
#[test]
fn a_word_too_long_to_name_anything_is_left_out() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let path = format!("{}.py", "w".repeat(300));
    let nodes = store.index(&[path.as_str()])?;
    store.task_that_changed(&mut recorder, ("s1", 1_000), &[path.as_str()])?;
    assert_eq!(said(&store, (100, 9_000))?, [(words(&["py"]), nodes, 1)]);
    Ok(())
}

/// A cluster keeps at most 50 files and at most 50 words: one task changes 60 files whose names
/// share no word but `py`, so its cluster holds the 50 with the smallest nodes, which are the 50
/// changed first. Every word comes from the one task, so every word scores one and they keep the
/// order they were met in: `d00`, `n00` and `py` from the first file, then two from each of the
/// next, which puts the 50th word, `d24`, in the 25th file.
#[test]
fn a_cluster_keeps_only_so_many_files_and_words() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let paths: Vec<String> = (0..60).map(|at| format!("d{at:02}/n{at:02}.py")).collect();
    let listed: Vec<&str> = paths.iter().map(String::as_str).collect();
    let nodes = store.index(&listed)?;
    store.task_that_changed(&mut recorder, ("s1", 1_000), &listed)?;
    let mut expected = words(&["d00", "n00", "py"]);
    for at in 1..60 {
        expected.push(format!("d{at:02}"));
        expected.push(format!("n{at:02}"));
    }
    expected.truncate(50);
    let clusters = said(&store, (100, 9_000))?;
    let (terms, files, tasks) = clusters.first().cloned().ok_or("no cluster")?;
    assert_eq!(tasks, 1);
    assert_eq!(files, nodes[..50], "the first 50 by node");
    assert_eq!(terms, expected, "the first 50 words met");
    Ok(())
}

/// The clustering keeps at most 256 clusters, those covering the most tasks first and, of those
/// covering as many, the ones holding the newest work: 256 tasks that change a file each, sharing
/// nothing, make 256 clusters of one task, and then two tasks that change one more file make a
/// cluster of two, the 257th made. That one is kept and comes first, then the newest 255 of the
/// others, newest first; the oldest is the one left out.
#[test]
fn the_clustering_keeps_only_so_many_clusters() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let mut paths: Vec<String> = (0..256).map(|at| format!("d{at:03}/n{at:03}.py")).collect();
    paths.push("pair/both.py".to_owned());
    let listed: Vec<&str> = paths.iter().map(String::as_str).collect();
    let nodes = store.index(&listed)?;
    let pair = nodes.last().copied().ok_or("no file")?;
    let mut changed: Vec<&str> = listed.get(..256).ok_or("too few files")?.to_vec();
    changed.extend(["pair/both.py", "pair/both.py"]);
    for (at, path) in changed.iter().enumerate() {
        let at_ms = 1_000 + i64::try_from(at)? * 1_000;
        store.task_that_changed(&mut recorder, ("s1", at_ms), &[path])?;
    }
    let mut expected: Vec<Shape> = vec![(vec![pair], 2)];
    expected.extend(
        nodes
            .get(1..256)
            .ok_or("too few files")?
            .iter()
            .rev()
            .map(|node| (vec![*node], 1)),
    );
    assert_eq!(shapes(&store, 300)?, expected);
    Ok(())
}

/// A pass reads at most the newest 2,000 records: of 2,001 tasks, the oldest changes `a.py` alone
/// and the others `b.py`, so the oldest is not read and the one cluster covers 2,000 tasks, however
/// many records the caller asks for. The records are written straight to the store, as the
/// recorder would leave them, to keep the test quick.
#[test]
fn a_pass_reads_at_most_so_many_records() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index(&["a.py", "b.py"])?;
    let (a, b) = (
        nodes.first().copied().ok_or("no file")?,
        nodes.get(1).copied().ok_or("no file")?,
    );
    let session = Identifier::new("s1")?;
    let mut transaction = store.writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: session.clone(),
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 500,
    })?;
    for at in 0..2_001_u64 {
        let task_id = Ulid::from_seed(1_000 + at, b"task");
        let started_at_ms = 1_000 + i64::try_from(at)?;
        transaction.start_task(&TaskRecord {
            task_id,
            session_id: session.clone(),
            started_at_ms,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: None,
        })?;
        transaction.replace_episode(&EpisodeRecord {
            task_id,
            localize: Vec::new(),
            change: vec![ChangedFile {
                node: if at == 0 { a } else { b },
                lines: 1,
                symbols: Vec::new(),
            }],
            verify: Vec::new(),
            pitfalls: Pitfalls::default(),
            outcome: TaskOutcome::Success,
            created_at_ms: started_at_ms,
        })?;
    }
    transaction.commit()?;
    assert_eq!(shapes(&store, usize::MAX)?, [(vec![b], 2_000)]);
    Ok(())
}

/// A repository with no records has no clusters, and the time each cluster carries is the one it
/// was written at.
#[test]
fn a_repository_with_no_records_has_no_clusters() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    assert_eq!(said(&store, (100, 9_000))?, []);
    store.index(&PATHS)?;
    store.task_that_changed(&mut recorder, ("s1", 1_000), &["tax.py"])?;
    let written = archetypes_of(store.writer.queries(), (100, 7_777))?;
    assert_eq!(
        written
            .iter()
            .map(|cluster| cluster.updated_at_ms)
            .collect::<Vec<i64>>(),
        [7_777]
    );
    Ok(())
}
