//! Warnings after an edit, from what the store and the task say: each rule's text typed out by
//! hand, the strongest first, the caps held, a pair never told twice in a task, and in tiny mode
//! only the call sites.

mod common;

use common::{Repo, TestResult};
use cortex_core::{FileFlags, RepoPath, ScaleMode, WarningKind, WarningsConfig, estimate_tokens};
use cortex_retrieve::{
    CallSite, Drift, EditFacts, Linked, MAX_WARNING_TOKENS, NamedFile, Partner, TaskSoFar,
    TestFile, Warned, warnings_after_edit,
};
use cortex_store::NodeId;
use proptest::prelude::*;

/// The files of the tests, stored so that each has a node: the edited file first.
const PATHS: [&str; 9] = [
    "src/orders/service.ts",
    "prisma/schema.prisma",
    "src/util.ts",
    "docs/orders.md",
    "src/billing.ts",
    "api/admin.ts",
    "api/orders/route.ts",
    "lib/legacy.ts",
    "src/orders/service.test.ts",
];

/// A store holding [`PATHS`], and each path with its node, in that order.
fn files() -> TestResult<(Repo, Vec<NamedFile>)> {
    let mut repo = Repo::new()?;
    let mut named = Vec::new();
    for path in PATHS {
        let nodes = repo.stored(path, "", &[], FileFlags::empty())?;
        let node = *nodes.first().ok_or("a file node")?;
        named.push(NamedFile {
            node,
            path: RepoPath::new(path)?,
        });
    }
    Ok((repo, named))
}

/// The file at `at` in [`PATHS`].
fn file(named: &[NamedFile], at: usize) -> TestResult<NamedFile> {
    Ok(named.get(at).ok_or("a stored file")?.clone())
}

/// The symbol `createOrder`'s node, which stands in for any node a drift is about: the rules only
/// compare it.
fn symbol_node(named: &[NamedFile]) -> TestResult<NodeId> {
    Ok(file(named, 0)?.node)
}

/// What the edit of `src/orders/service.ts` brings, worked by hand:
/// - history: 11 commits changed it; `prisma/schema.prisma` in 8 of them (confidence 0.73),
///   `src/util.ts` in 2 (too few, though at 0.9), `docs/orders.md` in 5 (confidence 0.4, too low);
/// - past work: `src/billing.ts` linked at weight 1.5 by 4 tasks;
/// - `createOrder` changed its signature; called from `api/admin.ts:12` (tied at 1.0),
///   `api/orders/route.ts:31` (0.8), `lib/legacy.ts:4` (0.6, too loose to state) and the edited
///   file itself at line 50;
/// - its test `src/orders/service.test.ts`, tied by import at 0.8.
fn facts(named: &[NamedFile]) -> TestResult<EditFacts> {
    let partner = |at, co_changes, confidence| -> TestResult<Partner> {
        Ok(Partner {
            file: file(named, at)?,
            co_changes,
            confidence,
            reverse_confidence: confidence,
        })
    };
    let site = |at, line, confidence| -> TestResult<CallSite> {
        Ok(CallSite {
            file: file(named, at)?,
            line,
            confidence,
        })
    };
    Ok(EditFacts {
        edited: file(named, 0)?,
        commits: 11,
        partners: vec![
            partner(1, 8, 0.73)?,
            partner(2, 2, 0.9)?,
            partner(3, 5, 0.4)?,
        ],
        linked: vec![Linked {
            file: file(named, 4)?,
            weight: 1.5,
            tasks: 4,
        }],
        drifts: vec![Drift {
            symbol: symbol_node(named)?,
            name: "createOrder".to_owned(),
            call_sites: vec![
                site(5, 12, 1.0)?,
                site(6, 31, 0.8)?,
                site(7, 4, 0.6)?,
                site(0, 50, 1.0)?,
            ],
            unchecked: 0,
        }],
        tests: vec![TestFile {
            file: file(named, 8)?,
            confidence: 0.8,
        }],
    })
}

/// The lines of `warned`'s text.
fn lines(warned: &Warned) -> Vec<&str> {
    warned
        .text
        .as_deref()
        .map_or_else(Vec::new, |text| text.lines().collect())
}

/// Every rule at once, under the defaults: the call sites first (tied at 1.0), then the tests
/// (0.8), then the history partner (0.73); the past-work partner, fourth at 1.5 / 5 = 0.3, is past
/// the cap of three. The loose call site and the edited file's own are not listed.
#[test]
fn an_edit_gets_its_strongest_three_warnings() -> TestResult {
    let (_repo, named) = files()?;
    let warned = warnings_after_edit(
        &facts(&named)?,
        &TaskSoFar::default(),
        &WarningsConfig::default(),
        ScaleMode::Medium,
    );
    assert_eq!(
        lines(&warned),
        [
            "[cortex] createOrder's signature changed; 2 call sites still use the old one: \
             api/admin.ts:12, api/orders/route.ts:31",
            "[cortex] Tests for src/orders/service.ts: src/orders/service.test.ts",
            "[cortex] Edits to src/orders/service.ts usually come with prisma/schema.prisma \
             (8 of 11 changes); not touched yet.",
        ]
    );
    let kinds: Vec<(WarningKind, Vec<NodeId>, f64)> = warned
        .given
        .iter()
        .map(|warning| (warning.kind, warning.targets.clone(), warning.strength))
        .collect();
    assert_eq!(
        kinds,
        [
            (
                WarningKind::CallSite,
                vec![file(&named, 5)?.node, file(&named, 6)?.node],
                1.0
            ),
            (WarningKind::Test, vec![file(&named, 8)?.node], 0.8),
            (WarningKind::CoChange, vec![file(&named, 1)?.node], 0.73),
        ]
    );
    Ok(())
}

/// The past-work partner, alone: its line says how many tasks drew the link. A link too light, or
/// drawn by too few tasks, gives none; and a file history names as well is named once, by history.
#[test]
fn past_work_names_a_companion() -> TestResult {
    let (_repo, named) = files()?;
    let mut facts = facts(&named)?;
    facts.partners.clear();
    facts.drifts.clear();
    facts.tests.clear();
    let warn = |facts: &EditFacts| {
        warnings_after_edit(
            facts,
            &TaskSoFar::default(),
            &WarningsConfig::default(),
            ScaleMode::Medium,
        )
    };
    assert_eq!(
        lines(&warn(&facts)),
        [
            "[cortex] Edits to src/orders/service.ts usually come with src/billing.ts (worked on \
          together in 4 earlier tasks); not touched yet."
        ]
    );
    for (weight, tasks) in [(0.99, 4), (1.5, 2)] {
        let mut weak = facts.clone();
        weak.linked = vec![Linked {
            file: file(&named, 4)?,
            weight,
            tasks,
        }];
        assert_eq!(warn(&weak), Warned::default(), "{weight} {tasks}");
    }
    facts.partners = vec![Partner {
        file: file(&named, 4)?,
        co_changes: 3,
        confidence: 0.5,
        reverse_confidence: 0.5,
    }];
    assert_eq!(
        lines(&warn(&facts)),
        [
            "[cortex] Edits to src/orders/service.ts usually come with src/billing.ts (3 of 11 \
          changes); not touched yet."
        ]
    );
    Ok(())
}

/// A history partner must hold from both sides. `prisma/schema.prisma` changed in 8 of the edited
/// file's 11 commits (0.73), but if it changed in 40 commits of its own, only 8 / 40 = 0.2 of them
/// changed the edited file: it changes with everything, and is not named. At 8 / 16 = 0.5 it is.
#[test]
fn a_partner_that_changes_with_everything_is_not_named() -> TestResult {
    let (_repo, named) = files()?;
    let mut facts = facts(&named)?;
    facts.drifts.clear();
    facts.tests.clear();
    facts.linked.clear();
    let warn = |facts: &EditFacts| {
        warnings_after_edit(
            facts,
            &TaskSoFar::default(),
            &WarningsConfig::default(),
            ScaleMode::Medium,
        )
    };
    let schema = |reverse_confidence| -> TestResult<Vec<Partner>> {
        Ok(vec![Partner {
            file: file(&named, 1)?,
            co_changes: 8,
            confidence: 0.73,
            reverse_confidence,
        }])
    };
    facts.partners = schema(8.0 / 40.0)?;
    assert_eq!(warn(&facts), Warned::default());
    facts.partners = schema(8.0 / 16.0)?;
    assert_eq!(
        lines(&warn(&facts)),
        [
            "[cortex] Edits to src/orders/service.ts usually come with prisma/schema.prisma (8 \
          of 11 changes); not touched yet."
        ]
    );
    Ok(())
}

/// A partner is named once a task, whichever edited file it is a partner of: the task was warned of
/// `prisma/schema.prisma` after an edit of `src/util.ts`, so the edit of `src/orders/service.ts`
/// does not name it again.
#[test]
fn a_partner_is_named_once_a_task() -> TestResult {
    let (_repo, named) = files()?;
    let mut facts = facts(&named)?;
    facts.drifts.clear();
    facts.tests.clear();
    facts.linked.clear();
    let mut task = TaskSoFar::default();
    task.warned.insert((
        WarningKind::CoChange,
        file(&named, 2)?.node,
        file(&named, 1)?.node,
    ));
    let warned = warnings_after_edit(&facts, &task, &WarningsConfig::default(), ScaleMode::Medium);
    assert_eq!(warned, Warned::default());
    Ok(())
}

/// Only the edited file's own tests are named. `api/admin.ts` stands here for a test file that
/// imports `src/orders/service.ts` without being named after it: with it alone there is no
/// warning; beside `src/orders/service.test.ts`, only the latter is named.
#[test]
fn only_the_edited_file_s_own_tests_are_named() -> TestResult {
    let (_repo, named) = files()?;
    let mut facts = facts(&named)?;
    facts.drifts.clear();
    facts.partners.clear();
    facts.linked.clear();
    let test = |at| -> TestResult<TestFile> {
        Ok(TestFile {
            file: file(&named, at)?,
            confidence: 0.8,
        })
    };
    let warn = |facts: &EditFacts| {
        warnings_after_edit(
            facts,
            &TaskSoFar::default(),
            &WarningsConfig::default(),
            ScaleMode::Medium,
        )
    };
    facts.tests = vec![test(5)?];
    assert_eq!(warn(&facts), Warned::default());
    facts.tests = vec![test(5)?, test(8)?];
    assert_eq!(
        lines(&warn(&facts)),
        ["[cortex] Tests for src/orders/service.ts: src/orders/service.test.ts"]
    );
    Ok(())
}

/// What the task already did: a file it read is no companion, a file it edited holds no call site
/// to tell of, a pair it was warned about is not told again, and tests are told once a task.
#[test]
fn the_task_so_far_holds_warnings_back() -> TestResult {
    let (_repo, named) = files()?;
    let node = |at| -> TestResult<NodeId> { Ok(file(&named, at)?.node) };
    let edited = node(0)?;
    let task = TaskSoFar {
        touched: [node(1)?, node(6)?].into(),
        edited: [node(6)?].into(),
        warned: [
            (WarningKind::CallSite, symbol_node(&named)?, node(5)?),
            (WarningKind::Test, node(3)?, node(2)?),
        ]
        .into(),
    };
    let warned = warnings_after_edit(
        &facts(&named)?,
        &task,
        &WarningsConfig::default(),
        ScaleMode::Medium,
    );
    assert_eq!(
        lines(&warned),
        [
            "[cortex] Edits to src/orders/service.ts usually come with src/billing.ts (worked on \
          together in 4 earlier tasks); not touched yet."
        ]
    );
    assert_eq!(
        warned.given.first().map(|warning| warning.source),
        Some(edited)
    );
    Ok(())
}

/// The settings: one warning an edit keeps the strongest; history's thresholds move which
/// partners count; none gives no text.
#[test]
fn the_settings_move_the_caps_and_thresholds() -> TestResult {
    let (_repo, named) = files()?;
    let facts = facts(&named)?;
    let warn = |config: &WarningsConfig| {
        warnings_after_edit(&facts, &TaskSoFar::default(), config, ScaleMode::Medium)
    };
    let one = warn(&WarningsConfig {
        max_per_edit: 1,
        ..WarningsConfig::default()
    });
    assert_eq!(one.given.len(), 1);
    assert_eq!(
        one.given.first().map(|warning| warning.kind),
        Some(WarningKind::CallSite)
    );
    let none = warn(&WarningsConfig {
        max_per_edit: 0,
        ..WarningsConfig::default()
    });
    assert_eq!(none, Warned::default());
    // At a count of 2 and confidence 0.4, `src/util.ts` (2 at 0.9) and `docs/orders.md` (5 at 0.4)
    // count too; ten warnings fit, so every partner shows, the strongest first.
    let loose = warn(&WarningsConfig {
        max_per_edit: 10,
        cochange_min_confidence: 0.4,
        cochange_min_count: 2,
    });
    let partners: Vec<f64> = loose
        .given
        .iter()
        .filter(|warning| warning.kind == WarningKind::CoChange)
        .map(|warning| warning.strength)
        .collect();
    assert_eq!(partners, [0.9, 0.73, 0.4, 0.3]);
    Ok(())
}

/// In tiny mode only the call sites are told.
#[test]
fn a_tiny_repository_hears_only_of_call_sites() -> TestResult {
    let (_repo, named) = files()?;
    let warned = warnings_after_edit(
        &facts(&named)?,
        &TaskSoFar::default(),
        &WarningsConfig::default(),
        ScaleMode::Tiny,
    );
    let kinds: Vec<WarningKind> = warned.given.iter().map(|warning| warning.kind).collect();
    assert_eq!(kinds, [WarningKind::CallSite]);
    Ok(())
}

/// Calls in files not checked against the disk: the count says "at least" the two told.
#[test]
fn unchecked_calls_make_the_count_at_least() -> TestResult {
    let (_repo, named) = files()?;
    let mut facts = facts(&named)?;
    facts.drifts.first_mut().ok_or("a drift")?.unchecked = 4;
    let warned = warnings_after_edit(
        &facts,
        &TaskSoFar::default(),
        &WarningsConfig::default(),
        ScaleMode::Tiny,
    );
    assert_eq!(
        lines(&warned),
        [
            "[cortex] createOrder's signature changed; at least 2 call sites still use the old one: \
             api/admin.ts:12, api/orders/route.ts:31"
        ]
    );
    Ok(())
}

/// Seven call sites: five are listed, the count says seven, and only the five listed files are
/// named, so the other two can still be told of later.
#[test]
fn many_call_sites_are_counted_and_five_listed() -> TestResult {
    let (_repo, named) = files()?;
    let mut facts = facts(&named)?;
    let sites: Vec<CallSite> = (1..=7)
        .map(|at| -> TestResult<CallSite> {
            Ok(CallSite {
                file: file(&named, at)?,
                line: u32::try_from(at * 10)?,
                confidence: 1.0,
            })
        })
        .collect::<TestResult<_>>()?;
    let drift = facts.drifts.first_mut().ok_or("a drift")?;
    drift.call_sites = sites;
    let warned = warnings_after_edit(
        &facts,
        &TaskSoFar::default(),
        &WarningsConfig::default(),
        ScaleMode::Tiny,
    );
    assert_eq!(
        lines(&warned),
        [
            "[cortex] createOrder's signature changed; 7 call sites still use the old one: \
          prisma/schema.prisma:10, src/util.ts:20, docs/orders.md:30, src/billing.ts:40, \
          api/admin.ts:50, ..."
        ]
    );
    let targets = warned.given.first().map(|warning| warning.targets.len());
    assert_eq!(targets, Some(5));
    Ok(())
}

/// Three companions of long paths are more than 250 tokens: the weakest goes. By hand, each line is
/// `[cortex] ` (9 bytes), `Edits to ` (9), the edited path (155), ` usually come with ` (19), the
/// partner's path (156), ` (` (2), `3 of 9 changes` (14), `); not touched yet.` (19) and the line
/// break (1): 384 bytes. Three are 1,152 bytes, 288 tokens at four bytes each, past the cap; two are
/// 768 bytes, 192 tokens.
#[test]
fn the_weakest_warning_goes_when_the_text_is_too_long() -> TestResult {
    let mut repo = Repo::new()?;
    let mut stored = |path: String| -> TestResult<NamedFile> {
        let node = *repo
            .stored(&path, "", &[], FileFlags::empty())?
            .first()
            .ok_or("a file node")?;
        Ok(NamedFile {
            node,
            path: RepoPath::new(&path)?,
        })
    };
    let edited = stored(format!("e/{}.ts", "x".repeat(150)))?;
    let mut partners = Vec::new();
    for (at, confidence) in [(1, 0.9), (2, 0.8), (3, 0.7)] {
        partners.push(Partner {
            file: stored(format!("p{at}/{}.ts", "y".repeat(150)))?,
            co_changes: 3,
            confidence,
            reverse_confidence: confidence,
        });
    }
    let facts = EditFacts {
        edited,
        commits: 9,
        partners,
        linked: Vec::new(),
        drifts: Vec::new(),
        tests: Vec::new(),
    };
    let warned = warnings_after_edit(
        &facts,
        &TaskSoFar::default(),
        &WarningsConfig::default(),
        ScaleMode::Medium,
    );
    let strengths: Vec<f64> = warned
        .given
        .iter()
        .map(|warning| warning.strength)
        .collect();
    assert_eq!(strengths, [0.9, 0.8]);
    let text = warned.text.unwrap_or_default();
    assert_eq!((text.len(), estimate_tokens(&text)), (768, 192));
    Ok(())
}

/// A name that is not plain text (a right-to-left override in a path, a line break in a symbol's
/// name) is never written: its warning is not given.
#[test]
fn a_crafted_name_gives_no_warning() -> TestResult {
    let (mut repo, mut named) = files()?;
    let crafted = "prisma/sch\u{202e}ema.prisma";
    let node = *repo
        .stored(crafted, "", &[], FileFlags::empty())?
        .first()
        .ok_or("a file node")?;
    *named.get_mut(1).ok_or("a stored file")? = NamedFile {
        node,
        path: RepoPath::new(crafted)?,
    };
    let mut facts = facts(&named)?;
    facts.linked.clear();
    facts.tests.clear();
    let drift = facts.drifts.first_mut().ok_or("a drift")?;
    drift.name = "create\nOrder".to_owned();
    let warned = warnings_after_edit(
        &facts,
        &TaskSoFar::default(),
        &WarningsConfig::default(),
        ScaleMode::Medium,
    );
    assert_eq!(warned, Warned::default());
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Whatever the partners, links, call sites and tests: at most `max_per_edit` warnings, within
    /// the token cap, the strongest first, each a single line of the text, and the same output for
    /// the same input.
    #[test]
    fn the_caps_hold_for_any_edit(
        partners in prop::collection::vec((1usize..9, 0u32..40, 0.0f64..=1.0), 0..12),
        linked in prop::collection::vec((1usize..9, 0.0f64..5.0, 0u32..12), 0..6),
        sites in prop::collection::vec((0usize..9, 1u32..100_000, 0.0f64..=1.0), 0..30),
        tests in prop::collection::vec((1usize..9, 0.0f64..=1.0), 0..4),
        unchecked in 0u32..3,
        max_per_edit in 0u32..=10,
        is_tiny in any::<bool>(),
    ) {
        let (_repo, named) = files().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let pick = |at: usize| named.get(at).cloned().ok_or_else(|| TestCaseError::fail("a file"));
        let mut facts = EditFacts {
            edited: pick(0)?,
            commits: 40,
            partners: Vec::new(),
            linked: Vec::new(),
            drifts: Vec::new(),
            tests: Vec::new(),
        };
        for (at, co_changes, confidence) in partners {
            facts.partners.push(Partner {
                file: pick(at)?,
                co_changes,
                confidence,
                reverse_confidence: confidence,
            });
        }
        for (at, weight, tasks) in linked {
            facts.linked.push(Linked { file: pick(at)?, weight, tasks });
        }
        let mut call_sites = Vec::new();
        for (at, line, confidence) in sites {
            call_sites.push(CallSite { file: pick(at)?, line, confidence });
        }
        facts.drifts.push(Drift { symbol: pick(0)?.node, name: "a_rather_long_function_name".to_owned(), call_sites, unchecked });
        for (at, confidence) in tests {
            facts.tests.push(TestFile { file: pick(at)?, confidence });
        }
        let config = WarningsConfig { max_per_edit, ..WarningsConfig::default() };
        let mode = if is_tiny { ScaleMode::Tiny } else { ScaleMode::Medium };
        let warned = warnings_after_edit(&facts, &TaskSoFar::default(), &config, mode);
        prop_assert!(warned.given.len() <= usize::try_from(max_per_edit).unwrap_or(usize::MAX));
        let text = warned.text.clone().unwrap_or_default();
        prop_assert!(estimate_tokens(&text) <= MAX_WARNING_TOKENS);
        prop_assert_eq!(text.lines().count(), warned.given.len());
        prop_assert_eq!(warned.text.is_none(), warned.given.is_empty());
        for pair in warned.given.windows(2) {
            if let [stronger, weaker] = pair {
                prop_assert!(stronger.strength >= weaker.strength);
            }
        }
        for warning in &warned.given {
            prop_assert!(!warning.targets.is_empty());
            prop_assert!(!warning.line.contains('\n'));
            prop_assert!(!is_tiny || warning.kind == WarningKind::CallSite);
        }
        let again = warnings_after_edit(&facts, &TaskSoFar::default(), &config, mode);
        prop_assert_eq!(&again, &warned);
    }
}
