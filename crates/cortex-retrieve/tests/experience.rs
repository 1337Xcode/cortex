//! The experience hint on a small scripted store: a past task like the prompt told with its facts,
//! the temporal guard, a file gone from disk, nothing like the prompt, the files a task put back, a
//! past change when no task is like the prompt, the same bytes from the same store, and the place
//! the hint takes in the work order's shed order. Every expected line is typed by hand.

mod common;

use std::time::Instant;

use common::{Repo, TestResult};
use cortex_core::{
    CommandKind, ContentHash, FileFlags, Identifier, LineRange, RepoPath, TaskOutcome, Ulid,
    estimate_tokens,
};
use cortex_retrieve::{
    Basis, ExperienceAsk, GraphSettings, Item, ItemSpan, Limits, PromptTerms, Reason, WorkOrder,
    experience_hint, read_graph, render, similar,
};
use cortex_store::{
    CardKind, ChangeCardRecord, ChangedFile, EpisodeRecord, NodeId, PersistedFailure, Pitfalls,
    RanCheck,
};

/// A day, in milliseconds.
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// When the earliest past task ended, in UTC milliseconds.
const PAST_MS: i64 = 1_000 * DAY_MS;

/// When the task asking began: two weeks after the earliest past task ended.
const START_MS: i64 = PAST_MS + 14 * DAY_MS;

/// The prompt, whose words are `orders`, `service` and `validate`.
const PROMPT: &str = "orders service validate";

/// The files of the store, in this order: the service, its validation, their tests, and a file of
/// another part.
const PATHS: [&str; 4] = [
    "src/orders/service.py",
    "src/orders/validate.py",
    "tests/test_orders.py",
    "src/billing/tax.py",
];

/// A store holding [`PATHS`] on disk, one cluster of past tasks whose first words are the prompt's
/// three and two more (a Jaccard score of 3/5, above the bar of 0.3) and whose files are the first
/// three, and four tasks for records to belong to; and the files' nodes.
fn store() -> TestResult<(Repo, [NodeId; 4])> {
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
    let nodes: [NodeId; 4] = nodes.try_into().map_err(|_| "four nodes")?;
    repo.clusters(&[(
        &["orders", "service", "validate", "create", "order"],
        &[nodes[0], nodes[1], nodes[2]],
        3,
    )])?;
    repo.tasks(4)?;
    Ok((repo, nodes))
}

/// The record of task number `at` (as [`Repo::tasks`] makes it), which changed `changed`, ended as
/// `outcome` at `created_at_ms`, with `pitfalls`.
fn episode(
    at: u64,
    changed: &[NodeId],
    (outcome, created_at_ms): (TaskOutcome, i64),
    pitfalls: Pitfalls,
) -> TestResult<EpisodeRecord> {
    Ok(EpisodeRecord {
        task_id: Ulid::from_seed(at, b"task"),
        localize: Vec::new(),
        change: changed
            .iter()
            .map(|node| ChangedFile {
                node: *node,
                lines: 1,
                symbols: Vec::new(),
            })
            .collect(),
        verify: vec![RanCheck {
            runner: Identifier::new("pytest")?,
            kind: CommandKind::Test,
            code: Some(0),
        }],
        pitfalls,
        outcome,
        created_at_ms,
    })
}

/// The pitfalls of a task whose pytest tests stayed failing while it had changed `edited`, until
/// it changed `until`.
fn persisted(edited: &[NodeId], until: &[NodeId]) -> TestResult<Pitfalls> {
    Ok(Pitfalls {
        persisted: vec![PersistedFailure {
            runner: Identifier::new("pytest")?,
            edited: edited.to_vec(),
            until: until.to_vec(),
        }],
        ..Pitfalls::default()
    })
}

/// Writes `episodes` to `repo`'s store.
fn keep(repo: &mut Repo, episodes: &[EpisodeRecord]) -> TestResult {
    let mut transaction = repo.writer.transaction()?;
    for record in episodes {
        transaction.replace_episode(record)?;
    }
    transaction.commit()?;
    Ok(())
}

/// The hint `prompt` gets in `repo` for a task that began at `before_ms`.
fn hint(repo: &Repo, prompt: &str, before_ms: i64) -> TestResult<Option<String>> {
    hint_by(repo, prompt, (before_ms, None))
}

/// The hint `prompt` gets in `repo` for a task that began at `before_ms`, the search stopped at
/// `deadline`.
fn hint_by(
    repo: &Repo,
    prompt: &str,
    (before_ms, deadline): (i64, Option<Instant>),
) -> TestResult<Option<String>> {
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let terms = PromptTerms::of(prompt);
    let found = similar(reader.queries(), &graph, &terms)?;
    Ok(experience_hint(
        reader.queries(),
        &graph,
        &ExperienceAsk {
            before_ms,
            similar: &found,
            terms: &terms,
            root: repo.root.path(),
            deadline,
        },
    )?)
}

/// The worked example. Task 1 ended two weeks before the task asking began; it changed the service
/// and the tests, both among the cluster's files (two of two, at least half), so it is like the
/// prompt; its pytest tests stayed failing while it had changed the service, until it changed the
/// validation.
const TOLD: &str = "[cortex] A similar task (2 weeks ago) changed src/orders/service.py and \
                    tests/test_orders.py; editing src/orders/service.py alone left the same \
                    pytest tests failing until src/orders/validate.py changed too.\n";

/// A past task like the prompt is told with its facts, as [`TOLD`] types them out: the prompt's
/// words are as the cluster's, the task changed two files both among the cluster's, it ended two
/// weeks before, and its failure stayed until the validation changed.
#[test]
fn a_similar_task_is_told_with_its_facts() -> TestResult {
    let (mut repo, nodes) = store()?;
    assert_eq!(
        PromptTerms::of(PROMPT).words,
        ["orders", "service", "validate"]
    );
    keep(
        &mut repo,
        &[episode(
            1,
            &[nodes[0], nodes[2]],
            (TaskOutcome::Success, PAST_MS),
            persisted(&[nodes[0]], &[nodes[1]])?,
        )?],
    )?;
    let told = hint(&repo, PROMPT, START_MS)?;
    assert_eq!(told.as_deref(), Some(TOLD));
    assert!(
        told.is_some_and(|line| estimate_tokens(&line) <= cortex_retrieve::MAX_EXPERIENCE_TOKENS)
    );
    Ok(())
}

/// The temporal guard: task 2 ended at the very moment the task asking began, and task 3 a
/// millisecond later; both are like the prompt and newer than task 1, and would be told first,
/// with another line (they changed the validation alone). Neither is ever told: task 1 is. With
/// task 1 gone, nothing is.
#[test]
fn work_that_ended_once_the_task_began_is_never_told() -> TestResult {
    let (mut repo, nodes) = store()?;
    let later = |at, created_at_ms| {
        episode(
            at,
            &[nodes[1]],
            (TaskOutcome::Success, created_at_ms),
            Pitfalls::default(),
        )
    };
    keep(&mut repo, &[later(2, START_MS)?, later(3, START_MS + 1)?])?;
    assert_eq!(hint(&repo, PROMPT, START_MS)?, None);
    // Asked a moment after task 3 ended, the newest is told: the guard is the only thing that kept
    // them out.
    assert_eq!(
        hint(&repo, PROMPT, START_MS + 2)?.as_deref(),
        Some("[cortex] A similar task (less than an hour ago) changed src/orders/validate.py.\n")
    );
    keep(
        &mut repo,
        &[episode(
            1,
            &[nodes[0], nodes[2]],
            (TaskOutcome::Success, PAST_MS),
            persisted(&[nodes[0]], &[nodes[1]])?,
        )?],
    )?;
    assert_eq!(hint(&repo, PROMPT, START_MS)?.as_deref(), Some(TOLD));
    Ok(())
}

/// A file gone from disk is not named. With the validation gone, the failure that names it is not
/// told (it would say something else without it), and the files the task changed are: the line
/// ends after them. With the tests gone too, the service alone is named. With every file the task
/// changed gone, there is no line.
#[test]
fn a_file_gone_from_disk_is_not_named() -> TestResult {
    let (mut repo, nodes) = store()?;
    keep(
        &mut repo,
        &[episode(
            1,
            &[nodes[0], nodes[2]],
            (TaskOutcome::Success, PAST_MS),
            persisted(&[nodes[0]], &[nodes[1]])?,
        )?],
    )?;
    let remove = |path: &str| -> TestResult {
        std::fs::remove_file(RepoPath::new(path)?.to_native(repo.root.path()))?;
        Ok(())
    };
    remove("src/orders/validate.py")?;
    assert_eq!(
        hint(&repo, PROMPT, START_MS)?.as_deref(),
        Some(
            "[cortex] A similar task (2 weeks ago) changed src/orders/service.py and \
             tests/test_orders.py.\n"
        )
    );
    remove("tests/test_orders.py")?;
    assert_eq!(
        hint(&repo, PROMPT, START_MS)?.as_deref(),
        Some("[cortex] A similar task (2 weeks ago) changed src/orders/service.py.\n")
    );
    remove("src/orders/service.py")?;
    assert_eq!(hint(&repo, PROMPT, START_MS)?, None);
    Ok(())
}

/// Nothing like the prompt gives nothing. A prompt sharing no word with the cluster finds no
/// similar task. A task that changed the service and two files of another part has one of three
/// files among the cluster's, below half, and is not like the prompt; a task that failed, though
/// like it, is not told either; and with no record at all there is nothing to tell.
#[test]
fn nothing_like_the_prompt_gives_nothing() -> TestResult {
    let (mut repo, nodes) = store()?;
    assert_eq!(hint(&repo, PROMPT, START_MS)?, None);
    let other = repo.file("src/billing/rates.py", "x = 1\n", &[], FileFlags::empty())?[0];
    keep(
        &mut repo,
        &[
            episode(
                1,
                &[nodes[0], nodes[3], other],
                (TaskOutcome::Success, PAST_MS),
                Pitfalls::default(),
            )?,
            episode(
                2,
                &[nodes[0], nodes[1]],
                (TaskOutcome::Failure, PAST_MS + 1),
                Pitfalls::default(),
            )?,
        ],
    )?;
    assert_eq!(hint(&repo, PROMPT, START_MS)?, None);
    keep(
        &mut repo,
        &[episode(
            3,
            &[nodes[0], nodes[2]],
            (TaskOutcome::Success, PAST_MS + 2),
            Pitfalls::default(),
        )?],
    )?;
    assert_eq!(hint(&repo, "billing tax rates", START_MS)?, None);
    Ok(())
}

/// A task that put a file back says so, and the file is not among those it changed and kept: it
/// changed the service and the validation, put the validation back, and ended five days before.
/// Unknown is an outcome whose work stood as far as anyone knows, and is told.
#[test]
fn the_files_a_task_put_back_are_told() -> TestResult {
    let (mut repo, nodes) = store()?;
    keep(
        &mut repo,
        &[episode(
            1,
            &[nodes[0], nodes[1]],
            (TaskOutcome::Unknown, START_MS - 5 * DAY_MS),
            Pitfalls {
                undone: vec![nodes[1]],
                ..Pitfalls::default()
            },
        )?],
    )?;
    assert_eq!(
        hint(&repo, PROMPT, START_MS)?.as_deref(),
        Some(
            "[cortex] A similar task (5 days ago) changed src/orders/service.py; it put \
             src/orders/validate.py back.\n"
        )
    );
    Ok(())
}

/// A card of commit `digit` x 40, normal, holding the words `words`, committed at `committed_s`,
/// that changed `files`.
fn card(digit: char, files: Vec<NodeId>, committed_s: i64) -> ChangeCardRecord {
    ChangeCardRecord {
        oid: digit.to_string().repeat(40),
        first_parent: None,
        committed_at_s: committed_s,
        kind: CardKind::Normal,
        pull_request: None,
        issues: Vec::new(),
        has_reason: false,
        files,
        symbols: Vec::new(),
        tests: Vec::new(),
        carded_at_ms: 1,
    }
}

/// With no past task like the prompt, a past change is told. Seven cards of four words each: card
/// `1111111` holds `quokka wombat numbat bilby` and changed the tax file and the tests, committed
/// three days before the task began; card `2222222` holds the same words and changed the service,
/// committed at the very moment the task began; five fillers hold other words. The prompt's four
/// words are held by two cards each, and each of the two cards holds all of them once at the
/// cards' usual length, scoring 1. Card `2222222` is newer and never told; `1111111` is. The
/// prompt `quokka` alone scores below the bar of 0.6: its one word, held by 2 of 7 cards, weighs
/// ln(5.5 / 2.5) = 0.7885 against the floor of twice a word one card holds, 2 x ln(6.5 / 1.5) =
/// 2.9329, which is 0.27, and is told nothing. With card `1111111` gone, nothing is.
#[test]
fn a_similar_change_is_told_when_no_task_is() -> TestResult {
    let (mut repo, nodes) = store()?;
    let words = |text: &str| text.split(' ').map(str::to_owned).collect::<Vec<String>>();
    let start_s = START_MS / 1_000;
    let mut transaction = repo.writer.transaction()?;
    transaction.put_change_card(
        &card('1', vec![nodes[3], nodes[2]], start_s - 3 * 24 * 60 * 60),
        &words("quokka wombat numbat bilby"),
    )?;
    transaction.put_change_card(
        &card('2', vec![nodes[0]], start_s),
        &words("quokka wombat numbat bilby"),
    )?;
    for digit in '3'..='7' {
        transaction.put_change_card(
            &card(digit, Vec::new(), 1),
            &words("alpha beta gamma delta"),
        )?;
    }
    transaction.commit()?;
    assert_eq!(
        hint(&repo, "quokka wombat numbat bilby", START_MS)?.as_deref(),
        Some(
            "[cortex] A similar change (commit 1111111, 3 days ago) changed src/billing/tax.py \
             and tests/test_orders.py.\n"
        )
    );
    assert_eq!(hint(&repo, "quokka", START_MS)?, None);
    let mut transaction = repo.writer.transaction()?;
    transaction.remove_change_cards(&["1".repeat(40)])?;
    transaction.commit()?;
    assert_eq!(hint(&repo, "quokka wombat numbat bilby", START_MS)?, None);
    // Card `2222222` was committed in the second the task began at: asked half a second into that
    // second it may have come after the start, and is not told; a second later it is.
    assert_eq!(
        hint(&repo, "quokka wombat numbat bilby", START_MS + 500)?,
        None
    );
    assert_eq!(
        hint(&repo, "quokka wombat numbat bilby", START_MS + 1_000)?.as_deref(),
        Some(
            "[cortex] A similar change (commit 2222222, less than an hour ago) changed src/orders/service.py.\n"
        )
    );
    Ok(())
}

/// A search the deadline overtakes gives no hint at all: the worked example, which with no deadline
/// is told, asked with a deadline already past, gives nothing.
#[test]
fn a_search_past_its_deadline_gives_no_hint() -> TestResult {
    let (mut repo, nodes) = store()?;
    keep(
        &mut repo,
        &[episode(
            1,
            &[nodes[0], nodes[2]],
            (TaskOutcome::Success, PAST_MS),
            persisted(&[nodes[0]], &[nodes[1]])?,
        )?],
    )?;
    assert_eq!(hint(&repo, PROMPT, START_MS)?.as_deref(), Some(TOLD));
    let past = Instant::now();
    assert_eq!(hint_by(&repo, PROMPT, (START_MS, Some(past)))?, None);
    Ok(())
}

/// The same store and prompt give the same bytes, asked again.
#[test]
fn the_same_store_gives_the_same_bytes() -> TestResult {
    let (mut repo, nodes) = store()?;
    keep(
        &mut repo,
        &[
            episode(
                1,
                &[nodes[0], nodes[2]],
                (TaskOutcome::Success, PAST_MS),
                persisted(&[nodes[0]], &[nodes[1]])?,
            )?,
            episode(
                2,
                &[nodes[1], nodes[2]],
                (TaskOutcome::Success, PAST_MS + DAY_MS),
                Pitfalls::default(),
            )?,
        ],
    )?;
    let first = hint(&repo, PROMPT, START_MS)?;
    assert!(first.is_some());
    for _ in 0..3 {
        assert_eq!(hint(&repo, PROMPT, START_MS)?, first);
    }
    Ok(())
}

/// The hint is written last and shed before code. `a.py` holds two functions; the work order
/// lists it with its first, shown. With room, the hint follows the code; with one token less than
/// that takes, the hint is shed and the code is whole, as the work order is written without one.
#[test]
fn the_hint_is_written_last_and_shed_before_code() -> TestResult {
    let text = "def f():\n    return 1\n\n\ndef g():\n    return 2\n";
    let mut repo = Repo::new()?;
    repo.file("a.py", text, &[], FileFlags::empty())?;
    let order = |hint: Option<&str>| -> TestResult<WorkOrder> {
        Ok(WorkOrder {
            basis: Basis::Agreement(vec!["the prompt's words", "the commit history"]),
            items: vec![Item {
                path: RepoPath::new("a.py")?,
                content: Some(ContentHash::of(text.as_bytes())),
                spans: vec![ItemSpan {
                    lines: LineRange::new(1, 2)?,
                    name: Some(Identifier::new("f")?),
                    scope: None,
                }],
                reason: Reason::Matches(vec!["f".to_owned()]),
            }],
            facts: Vec::new(),
            hint: hint.map(str::to_owned),
        })
    };
    let limits = |max_tokens| Limits {
        max_tokens,
        max_files: 5,
        max_code_spans: 1,
        span_max_lines: 60,
        max_file_bytes: 1_048_576,
    };
    let line = "[cortex] A similar task (1 day ago) changed a.py.\n";
    let without = "\
[cortex] Work order · the prompt's words and the commit history agree
Start here:
  1. a.py:1-2  f  (matches f)
Current code:
  a.py:1-2
  1 | def f():
  2 |     return 1
";
    let whole = render(&order(Some(line))?, repo.root.path(), &limits(900)).ok_or("whole")?;
    assert_eq!(whole.text, format!("{without}{line}"));
    assert!(whole.is_hinted);
    let tight = render(
        &order(Some(line))?,
        repo.root.path(),
        &limits(whole.tokens - 1),
    )
    .ok_or("tight")?;
    assert_eq!(tight.text, without);
    assert!(!tight.is_hinted);
    let plain = render(&order(None)?, repo.root.path(), &limits(900)).ok_or("plain")?;
    assert_eq!(plain.text, without);
    Ok(())
}
