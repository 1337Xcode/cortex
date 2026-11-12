//! One retrieval end to end on a repository on disk: what the prompt and the session point at, the
//! gate, and the work order written from the files as they are.

mod common;

use common::{Def, Repo, TestResult, event, function, record, tool};
use cortex_core::{
    EdgeType, FileFlags, HookKind, Identifier, RepoPath, RetrievalConfig, ScaleMode, SymbolKind,
    ToolKind,
};
use cortex_learn::{ConfidenceModel, MODEL_INPUTS};
use cortex_retrieve::{
    GraphSettings, Location, Outcome, Reason, Request, Rule, Shut, Silence, Verdict, locate,
    read_graph, retrieve,
};

/// `src/orders/service.py`: one function, `create_order`, on lines 1 to 4.
const SERVICE: &str =
    "def create_order(cart):\n    total = sum(cart)\n    check(total)\n    return total\n";

/// `src/orders/model.py`: one class, `Order`, on lines 1 to 3.
const MODEL: &str = "class Order:\n    def __init__(self):\n        self.lines = []\n";

/// `src/billing/tax.py`: one function, `rate`, on lines 1 to 2.
const TAX: &str = "def rate():\n    return 0.2\n";

/// `src/payments/checkout.py`: one function, `pay`, on lines 1 to 2.
const CHECKOUT: &str = "def pay(order):\n    return order\n";

/// `src/lib/checks.py`: one function, `check`, on lines 1 to 2.
const CHECKS: &str = "def check(total):\n    return total > 0\n";

/// A repository of five files on disk and in the store. The service and the model change together
/// in 9 commits, of the service's 12 and the model's 10: a link each way, 9/12 and 9/10. The
/// service and the checkout change together in 5, of the checkout's 60: a link from the service,
/// 5/12, and none back, 5/60 being under the miner's least confidence of 0.1. The service's
/// `create_order` calls `check`, a reference resolved at 0.5 only, under the 0.8 a statement needs.
fn repository() -> TestResult<Repo> {
    let mut repo = Repo::new()?;
    let service = repo.file(
        "src/orders/service.py",
        SERVICE,
        &[function("create_order", 1, 4)],
        FileFlags::empty(),
    )?;
    let model = repo.file(
        "src/orders/model.py",
        MODEL,
        &[Def {
            name: "Order",
            kind: SymbolKind::Class,
            start: 1,
            end: 3,
            parent: None,
        }],
        FileFlags::empty(),
    )?;
    repo.file(
        "src/billing/tax.py",
        TAX,
        &[function("rate", 1, 2)],
        FileFlags::empty(),
    )?;
    let checkout = repo.file(
        "src/payments/checkout.py",
        CHECKOUT,
        &[function("pay", 1, 2)],
        FileFlags::empty(),
    )?;
    let checks = repo.file(
        "src/lib/checks.py",
        CHECKS,
        &[function("check", 1, 2)],
        FileFlags::empty(),
    )?;
    let node =
        |nodes: &[cortex_store::NodeId], at: usize| nodes.get(at).copied().ok_or("a stored node");
    let (create_order, check) = (node(&service, 1)?, node(&checks, 1)?);
    let (service, model, checkout) = (node(&service, 0)?, node(&model, 0)?, node(&checkout, 0)?);
    // Each link weighs the share of its file's commits that changed the other too.
    repo.link((service, model), EdgeType::CoChange, (9.0 / 12.0, 9, 0))?;
    repo.link((model, service), EdgeType::CoChange, (9.0 / 10.0, 9, 0))?;
    repo.link((service, checkout), EdgeType::CoChange, (5.0 / 12.0, 5, 0))?;
    repo.link((create_order, check), EdgeType::Structural, (0.5, 1, 0))?;
    repo.cochange(
        &[
            ("src/orders/service.py", 12),
            ("src/orders/model.py", 10),
            ("src/payments/checkout.py", 60),
        ],
        &[
            (("src/orders/service.py", "src/orders/model.py"), 9),
            (("src/orders/service.py", "src/payments/checkout.py"), 5),
        ],
    )?;
    repo.mode(ScaleMode::Medium)?;
    Ok(repo)
}

/// What `prompt` in session `s1` gives in `repo`, in `mode`.
fn asked(repo: &Repo, prompt: &str, mode: ScaleMode) -> TestResult<cortex_retrieve::Retrieval> {
    asked_with(repo, prompt, mode, &RetrievalConfig::default())
}

/// What `prompt` in session `s1` gives in `repo`, in `mode`, with the `retrieval.*` `settings`.
fn asked_with(
    repo: &Repo,
    prompt: &str,
    mode: ScaleMode,
    settings: &RetrievalConfig,
) -> TestResult<cortex_retrieve::Retrieval> {
    asked_while_changed(repo, prompt, (mode, settings), (&[], None))
}

/// What `prompt` in session `s1` gives in `repo`, in `mode`, with the `retrieval.*` `settings`,
/// while the work tree holds the files `changed` changed and not committed, for a task that began
/// at `experience_before_ms` when the experience hint is asked for.
fn asked_while_changed(
    repo: &Repo,
    prompt: &str,
    (mode, settings): (ScaleMode, &RetrievalConfig),
    (changed, experience_before_ms): (&[&str], Option<i64>),
) -> TestResult<cortex_retrieve::Retrieval> {
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let mut uncommitted = Vec::new();
    for path in changed {
        let node = reader.queries().file_node_at(&RepoPath::new(path)?)?;
        uncommitted.push(node.ok_or("a stored file")?);
    }
    let session = Identifier::new("s1")?;
    let request = Request {
        prompt,
        session: Some(&session),
        uncommitted: &uncommitted,
        failure: &[],
        mode,
        settings,
        max_file_bytes: 1_048_576,
        raise: 0.0,
        recent_precision: None,
        model: None,
        deadline: None,
        root: repo.root.path(),
        why_lines: None,
        history_seed_weight: None,
        experience_before_ms,
    };
    Ok(retrieve(reader.queries(), &graph, &request)?)
}

/// What the engine finds without the gate for `question`, asked in `session` or outside any, in
/// `repo` in `mode`, at most `limit` files.
fn located(
    repo: &Repo,
    question: &str,
    (mode, session): (ScaleMode, Option<&str>),
    limit: usize,
) -> TestResult<Location> {
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let session = session.map(Identifier::new).transpose()?;
    let settings = RetrievalConfig::default();
    let request = Request {
        prompt: question,
        session: session.as_ref(),
        uncommitted: &[],
        failure: &[],
        mode,
        settings: &settings,
        max_file_bytes: 1_048_576,
        raise: 0.0,
        recent_precision: None,
        model: None,
        deadline: None,
        root: repo.root.path(),
        why_lines: None,
        history_seed_weight: None,
        experience_before_ms: None,
    };
    Ok(locate(reader.queries(), &graph, &request, limit)?)
}

/// The paths `location` lists, the best first.
fn paths_of(location: &Location) -> Vec<&str> {
    location
        .listed
        .iter()
        .map(|entry| entry.item.path.as_str())
        .collect()
}

/// The prompt names `create_order` exactly, so its file matches the prompt fully, and the commit
/// history joins that file to the model, which the prompt's word "orders" also finds: two kinds
/// agree on the service and the work order goes through. Typed out by hand: the service, named,
/// and the model, whose class and path hold "order" and "orders", are where to start, each with
/// its code; the checkout, which no word finds, is also likely for changing with the service in 5
/// of its 60 commits, a link that runs from the service only; the tax file, which nothing points
/// at, is not listed, nor is the checks file, which the service's call reaches but which nothing
/// true can be said of.
#[test]
fn a_prompt_in_a_familiar_area_gets_its_work_order() -> TestResult {
    let repo = repository()?;
    let retrieval = asked(
        &repo,
        "fix create_order in the orders service",
        ScaleMode::Medium,
    )?;
    let Outcome::WorkOrder(rule, rendered) = &retrieval.outcome else {
        return Err(format!("{:?}", retrieval.outcome).into());
    };
    assert_eq!(*rule, Rule::Agreement);
    let expected = "\
[cortex] Work order · the prompt's words and the commit history agree
Start here:
  1. src/orders/service.py:1-4  create_order  (named in the prompt)
  2. src/orders/model.py:1-3  Order  (matches order, orders)
Also likely: src/payments/checkout.py:1-2 (changes with service.py in 5 of 60 commits)
Current code:
  src/orders/service.py:1-4
  1 | def create_order(cart):
  2 |     total = sum(cart)
  3 |     check(total)
  4 |     return total
  src/orders/model.py:1-3
  1 | class Order:
  2 |     def __init__(self):
  3 |         self.lines = []
";
    assert_eq!(rendered.text, expected);
    let features = retrieval.features.ok_or("the gate's features")?;
    assert!((features.top_lexical - 1.0).abs() < 1e-12);
    assert!(features.agreeing_kinds >= 2);
    Ok(())
}

/// A vague prompt points at nothing and gets nothing.
#[test]
fn a_vague_prompt_gets_nothing() -> TestResult {
    let repo = repository()?;
    for prompt in ["make it better", "fix the bug", "what does this do?"] {
        let retrieval = asked(&repo, prompt, ScaleMode::Medium)?;
        assert_eq!(
            retrieval.outcome,
            Outcome::Silent(Silence::NothingFound),
            "{prompt}"
        );
        assert!(retrieval.listed.is_empty());
    }
    Ok(())
}

/// A spreading the deadline cut short gives nothing, since what it ranked would hang on how fast the
/// machine was. The prompt that gets its work order above, asked with a deadline already passed,
/// stops before its first push, in medium mode and in huge mode's two stages alike: the gate stays
/// shut for that reason, and its features are kept for the record.
#[test]
fn a_spreading_out_of_time_gives_nothing() -> TestResult {
    let repo = repository()?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let session = Identifier::new("s1")?;
    let settings = RetrievalConfig::default();
    for mode in [ScaleMode::Medium, ScaleMode::Huge] {
        let request = Request {
            prompt: "fix create_order in the orders service",
            session: Some(&session),
            uncommitted: &[],
            failure: &[],
            mode,
            settings: &settings,
            max_file_bytes: 1_048_576,
            raise: 0.0,
            recent_precision: None,
            model: None,
            deadline: Some(std::time::Instant::now()),
            root: repo.root.path(),
            why_lines: None,
            history_seed_weight: None,
            experience_before_ms: None,
        };
        let retrieval = retrieve(reader.queries(), &graph, &request)?;
        assert_eq!(
            retrieval.outcome,
            Outcome::Silent(Silence::Gate(Shut::OutOfTime)),
            "{mode:?}"
        );
        assert!(retrieval.features.is_some(), "{mode:?}");
    }
    Ok(())
}

/// The engine asked directly names no file when its spreading runs out of time, in either mode:
/// which files it had reached by then is the clock's doing. Its answer says only that, and what the
/// gate would have said is that it ran out of time. With no deadline the same question names files.
#[test]
fn a_question_out_of_time_names_no_file() -> TestResult {
    let repo = repository()?;
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let settings = RetrievalConfig::default();
    for mode in [ScaleMode::Medium, ScaleMode::Huge] {
        let mut request = Request {
            prompt: "fix create_order in the orders service",
            session: None,
            uncommitted: &[],
            failure: &[],
            mode,
            settings: &settings,
            max_file_bytes: 1_048_576,
            raise: 0.0,
            recent_precision: None,
            model: None,
            deadline: None,
            root: repo.root.path(),
            why_lines: None,
            history_seed_weight: None,
            experience_before_ms: None,
        };
        let in_time = locate(reader.queries(), &graph, &request, 5)?;
        assert!(!in_time.listed.is_empty(), "{mode:?}");
        request.deadline = Some(std::time::Instant::now());
        let location = locate(reader.queries(), &graph, &request, 5)?;
        assert!(location.listed.is_empty(), "{mode:?}");
        assert_eq!(
            location.judged.map(|(_, verdict)| verdict),
            Some(Verdict::Silent(Shut::OutOfTime)),
            "{mode:?}"
        );
        assert_eq!(
            location.rendered.map(|rendered| rendered.text).as_deref(),
            Some(
                "[cortex] Where · the gate would stay silent: the spreading ran out of time, so no \
                 file is named\n"
            ),
            "{mode:?}"
        );
    }
    Ok(())
}

/// A prompt whose words find one file, with nothing else to back it, stays below the gate: the word
/// "rate" matches the tax file's function, but the file has no second kind of evidence.
#[test]
fn one_kind_of_evidence_is_not_enough() -> TestResult {
    let repo = repository()?;
    let retrieval = asked(&repo, "change rate", ScaleMode::Medium)?;
    assert_eq!(
        retrieval.outcome,
        Outcome::Silent(Silence::Gate(Shut::TooFewKinds))
    );
    Ok(())
}

/// What `prompt` in session `s1` gives in `repo` in medium mode once the repository keeps
/// `model`, a confidence model.
fn asked_by_model(
    repo: &Repo,
    prompt: &str,
    model: &ConfidenceModel,
) -> TestResult<cortex_retrieve::Retrieval> {
    asked_by_model_at(repo, prompt, model, (ScaleMode::Medium, 0.70, 0.0))
}

/// What `prompt` in session `s1` gives in `repo` once the repository keeps `model`, in `mode`, with
/// `retrieval.min_confidence` set to `least` and the self-check's raise at `raise`.
fn asked_by_model_at(
    repo: &Repo,
    prompt: &str,
    model: &ConfidenceModel,
    (mode, least, raise): (ScaleMode, f64, f64),
) -> TestResult<cortex_retrieve::Retrieval> {
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let session = Identifier::new("s1")?;
    let settings = RetrievalConfig {
        min_confidence: least,
        ..RetrievalConfig::default()
    };
    let request = Request {
        prompt,
        session: Some(&session),
        uncommitted: &[],
        failure: &[],
        mode,
        settings: &settings,
        max_file_bytes: 1_048_576,
        raise,
        recent_precision: None,
        model: Some(model),
        deadline: None,
        root: repo.root.path(),
        why_lines: None,
        history_seed_weight: None,
        experience_before_ms: None,
    };
    Ok(retrieve(reader.queries(), &graph, &request)?)
}

/// What the engine finds without the gate for `question` in `repo` in medium mode, at most 5
/// files, once the repository keeps `model`.
fn located_by_model(repo: &Repo, question: &str, model: &ConfidenceModel) -> TestResult<Location> {
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let settings = RetrievalConfig::default();
    let request = Request {
        prompt: question,
        session: None,
        uncommitted: &[],
        failure: &[],
        mode: ScaleMode::Medium,
        settings: &settings,
        max_file_bytes: 1_048_576,
        raise: 0.0,
        recent_precision: None,
        model: Some(model),
        deadline: None,
        root: repo.root.path(),
        why_lines: None,
        history_seed_weight: None,
        experience_before_ms: None,
    };
    Ok(locate(reader.queries(), &graph, &request, 5)?)
}

/// A model of intercept `log_odds` and no weights: it gives every retrieval the same chance,
/// 1 / (1 + e^-log_odds).
fn constant_model(log_odds: f64) -> ConfidenceModel {
    ConfidenceModel {
        intercept: log_odds,
        weights: [0.0; MODEL_INPUTS],
    }
}

/// Once a model is kept it alone decides. The prompt the cold-start rule lets through (two kinds
/// agree on the service) is kept back by a model giving every retrieval a chance of
/// 1 / (1 + e^20), about 2e-9, under the bar of 0.70, its features and that chance recorded. The
/// prompt the rule keeps back ("change rate": the tax file, one kind) goes through a model giving
/// 1 / (1 + e^-20), above 0.999, under the header that says the model chose it. Typed by hand from
/// the fixture: the tax file's `rate`, lines 1 to 2, which the plain word "rate" matches without
/// naming it (as "orders" matches `Order` above), and its code.
#[test]
fn a_kept_model_decides_in_place_of_the_cold_start_rule() -> TestResult {
    let repo = repository()?;
    let kept_back = asked_by_model(
        &repo,
        "fix create_order in the orders service",
        &constant_model(-20.0),
    )?;
    assert_eq!(
        kept_back.outcome,
        Outcome::Silent(Silence::Gate(Shut::Unlikely))
    );
    assert!(kept_back.features.is_some());
    let chance = kept_back.confidence.ok_or("the model's chance")?;
    assert!(chance > 0.0 && chance < 1e-8, "{chance}");
    let given = asked_by_model(&repo, "change rate", &constant_model(20.0))?;
    let Outcome::WorkOrder(rule, rendered) = &given.outcome else {
        return Err(format!("{:?}", given.outcome).into());
    };
    assert_eq!(*rule, Rule::Calibrated);
    let expected = "\
[cortex] Work order · learned from earlier work here
Start here:
  1. src/billing/tax.py:1-2  rate  (matches rate)
Current code:
  src/billing/tax.py:1-2
  1 | def rate():
  2 |     return 0.2
";
    assert_eq!(rendered.text, expected);
    assert!(given.confidence.is_some_and(|chance| chance > 0.999));
    Ok(())
}

/// The bar the pipeline asks of a kept model follows the setting, the mode and the self-check's
/// raise. A model of log-odds ln 3 gives every retrieval a chance of 1 / (1 + 1/3) = 0.75, between
/// the bars: the familiar prompt goes through at the default bar of 0.70 in medium mode, and is kept
/// back when the raise of 0.1 lifts the bar to 0.80, in small mode (at least 0.85), and with the
/// setting at 0.80; each time with the same chance recorded.
#[test]
fn the_bar_follows_the_setting_the_mode_and_the_raise() -> TestResult {
    let repo = repository()?;
    let three_to_one = constant_model(3.0_f64.ln());
    let prompt = "fix create_order in the orders service";
    let cases = [
        ((ScaleMode::Medium, 0.70, 0.0), true),
        ((ScaleMode::Medium, 0.70, 0.1), false),
        ((ScaleMode::Small, 0.70, 0.0), false),
        ((ScaleMode::Medium, 0.80, 0.0), false),
    ];
    for (bar, is_given) in cases {
        let retrieval = asked_by_model_at(&repo, prompt, &three_to_one, bar)?;
        if is_given {
            assert!(
                matches!(retrieval.outcome, Outcome::WorkOrder(Rule::Calibrated, _)),
                "{bar:?}: {:?}",
                retrieval.outcome
            );
        } else {
            assert_eq!(
                retrieval.outcome,
                Outcome::Silent(Silence::Gate(Shut::Unlikely)),
                "{bar:?}"
            );
        }
        let chance = retrieval.confidence.ok_or("the model's chance")?;
        assert!((chance - 0.75).abs() < 1e-12, "{bar:?}: {chance}");
    }
    Ok(())
}

/// Asked directly, the engine says what a kept model would do: for the familiar question, which the
/// prompt's words and the commit history agree on, "the gate would give it" under a model giving
/// above 0.999, and that the chance is below the bar under one giving about 2e-9.
#[test]
fn a_question_says_what_a_kept_model_would_do() -> TestResult {
    let repo = repository()?;
    let question = "fix create_order in the orders service";
    let heading = |location: &Location| -> TestResult<String> {
        let rendered = location.rendered.as_ref().ok_or("an answer")?;
        Ok(rendered.text.lines().next().unwrap_or_default().to_owned())
    };
    let given = located_by_model(&repo, question, &constant_model(20.0))?;
    assert_eq!(
        heading(&given)?,
        "[cortex] Where · the prompt's words and the commit history agree · the gate would give it"
    );
    let kept_back = located_by_model(&repo, question, &constant_model(-20.0))?;
    assert_eq!(
        heading(&kept_back)?,
        "[cortex] Where · the prompt's words and the commit history agree · the gate would stay \
         silent: the confidence model puts the chance these files are used below its bar"
    );
    assert_eq!(
        kept_back.judged.map(|(_, verdict)| verdict),
        Some(Verdict::Silent(Shut::Unlikely))
    );
    Ok(())
}

/// In tiny mode nothing is retrieved, whatever the prompt.
#[test]
fn tiny_mode_retrieves_nothing() -> TestResult {
    let repo = repository()?;
    let retrieval = asked(
        &repo,
        "fix create_order in the orders service",
        ScaleMode::Tiny,
    )?;
    assert_eq!(retrieval.outcome, Outcome::Silent(Silence::Tiny));
    assert_eq!(retrieval.features, None);
    Ok(())
}

/// A session that edited the tax file and is asked about its rate again is continuing its own
/// work: the work order goes through on that alone, the edit the reason the file is listed.
#[test]
fn a_session_continuing_its_own_work_gets_its_work_order() -> TestResult {
    let mut repo = repository()?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(
                ("s1", 2_100),
                ToolKind::Edit,
                ("src/billing/tax.py", &[(2, 2)]),
            )?,
        ],
    )?;
    let retrieval = asked(&repo, "change rate", ScaleMode::Medium)?;
    let Outcome::WorkOrder(rule, rendered) = &retrieval.outcome else {
        return Err(format!("{:?}", retrieval.outcome).into());
    };
    assert_eq!(*rule, Rule::Continuity);
    assert!(
        rendered.text.starts_with(
            "[cortex] Work order · continuing the work under way\nStart here:\n  1. src/billing/tax.py:1-2  rate  (edited earlier in this session)\n"
        ),
        "{}",
        rendered.text
    );
    Ok(())
}

/// The same prompt, about the tax file's rate, while the work tree holds the tax file changed and
/// not committed and the session has edited nothing: the work under way goes on, and the work order
/// goes through on that alone, the words the reason the file is listed.
#[test]
fn a_prompt_about_an_uncommitted_change_gets_its_work_order() -> TestResult {
    let repo = repository()?;
    let settings = RetrievalConfig::default();
    let retrieval = asked_while_changed(
        &repo,
        "change rate",
        (ScaleMode::Medium, &settings),
        (&["src/billing/tax.py"], None),
    )?;
    let Outcome::WorkOrder(rule, rendered) = &retrieval.outcome else {
        return Err(format!("{:?}", retrieval.outcome).into());
    };
    assert_eq!(*rule, Rule::Continuity);
    assert!(
        rendered.text.starts_with(
            "[cortex] Work order · continuing the work under way\nStart here:\n  1. src/billing/tax.py:1-2  rate  (matches rate)\n"
        ),
        "{}",
        rendered.text
    );
    Ok(())
}

/// A file that changed on disk since it was indexed lends no lines: the service is listed by path
/// alone, and only the model's code is shown.
#[test]
fn a_file_changed_since_indexing_loses_its_lines() -> TestResult {
    let repo = repository()?;
    std::fs::write(
        repo.root.path().join("src/orders/service.py"),
        "def create_order(cart):\n    return 0\n",
    )?;
    let retrieval = asked(
        &repo,
        "fix create_order in the orders service",
        ScaleMode::Medium,
    )?;
    let Outcome::WorkOrder(_, rendered) = &retrieval.outcome else {
        return Err(format!("{:?}", retrieval.outcome).into());
    };
    assert!(
        !rendered.text.contains("service.py:1-4"),
        "{}",
        rendered.text
    );
    assert!(
        rendered
            .text
            .contains("src/orders/service.py (named in the prompt)"),
        "{}",
        rendered.text
    );
    assert!(!rendered.text.contains("return 0"), "{}", rendered.text);
    assert!(
        rendered.text.contains("  1 | class Order:\n"),
        "{}",
        rendered.text
    );
    Ok(())
}

/// Days, in milliseconds.
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// The prompt the experience hint is asked for with.
const EXPERIENCED: &str = "fix create_order in the orders service";

/// [`repository`] with earlier work like [`EXPERIENCED`]: a cluster whose words are the prompt's
/// own and whose files are the service and the model, and task 1, which changed both and ended at
/// day 100.
fn experienced_repository() -> TestResult<Repo> {
    let mut repo = repository()?;
    let reader = repo.reader()?;
    let node = |path: &str| -> TestResult<cortex_store::NodeId> {
        Ok(reader
            .queries()
            .file_node_at(&RepoPath::new(path)?)?
            .ok_or("a stored file")?)
    };
    let (service, model) = (node("src/orders/service.py")?, node("src/orders/model.py")?);
    drop(reader);
    let words = cortex_retrieve::PromptTerms::of(EXPERIENCED).words;
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    repo.clusters(&[(words.as_slice(), &[service, model], 1)])?;
    repo.tasks(1)?;
    let mut transaction = repo.writer.transaction()?;
    transaction.replace_episode(&cortex_store::EpisodeRecord {
        task_id: cortex_core::Ulid::from_seed(1, b"task"),
        localize: Vec::new(),
        change: [service, model]
            .map(|node| cortex_store::ChangedFile {
                node,
                lines: 1,
                symbols: Vec::new(),
            })
            .to_vec(),
        verify: Vec::new(),
        pitfalls: cortex_store::Pitfalls::default(),
        outcome: cortex_core::TaskOutcome::Success,
        created_at_ms: 100 * DAY_MS,
    })?;
    transaction.commit()?;
    Ok(repo)
}

/// The work orders [`EXPERIENCED`] gets in `repo` not asking for the hint and asking for it at day
/// 114, with the second's retrieval.
fn without_and_with(
    repo: &Repo,
) -> TestResult<(cortex_retrieve::Rendered, cortex_retrieve::Retrieval)> {
    let settings = RetrievalConfig::default();
    let at = |experience| {
        asked_while_changed(
            repo,
            EXPERIENCED,
            (ScaleMode::Medium, &settings),
            (&[], experience),
        )
    };
    let Outcome::WorkOrder(_, without) = at(None)?.outcome else {
        return Err("a work order not asked for the hint".into());
    };
    Ok((without, at(Some(114 * DAY_MS))?))
}

/// A work order asked for the experience hint carries it last, and is otherwise the work order
/// asked without it, from [`experienced_repository`]: task 1 ended 14 days before the task asking
/// began. The hint line is typed by hand; a work order not asked for it has none.
#[test]
fn a_work_order_carries_the_experience_hint_last() -> TestResult {
    let repo = experienced_repository()?;
    let (without, retrieval) = without_and_with(&repo)?;
    let Outcome::WorkOrder(_, with) = retrieval.outcome else {
        return Err("a work order asked for the hint".into());
    };
    assert!(!without.is_hinted);
    assert!(with.is_hinted);
    assert_eq!(retrieval.hint_failure, None);
    assert_eq!(
        with.text,
        format!(
            "{}[cortex] A similar task (2 weeks ago) changed src/orders/service.py and \
             src/orders/model.py.\n",
            without.text
        )
    );
    Ok(())
}

/// A record the store cannot read back costs the hint, never the work order: with task 1's changes
/// overwritten by text that is not JSON, the work order asked for the hint is the one not asked for
/// it, and the retrieval says why the hint failed, naming the column and no content.
#[test]
fn a_corrupt_record_costs_the_hint_alone() -> TestResult {
    let repo = experienced_repository()?;
    let database = rusqlite::Connection::open(repo.root.path().join(".cortex").join("cortex.db"))?;
    let changed = database.execute("UPDATE episodes SET change = 'not json'", [])?;
    assert_eq!(changed, 1);
    drop(database);
    let (without, retrieval) = without_and_with(&repo)?;
    let Outcome::WorkOrder(_, with) = retrieval.outcome else {
        return Err("a work order asked for the hint".into());
    };
    assert!(!with.is_hinted);
    assert_eq!(with.text, without.text);
    let failure = retrieval.hint_failure.ok_or("the hint's failure")?;
    assert!(failure.contains("episodes.change"), "{failure}");
    assert!(!failure.contains("not json"), "{failure}");
    Ok(())
}

/// The same request on the same store gives the same bytes.
#[test]
fn the_same_request_gives_the_same_bytes() -> TestResult {
    let repo = repository()?;
    let prompt = "fix create_order in the orders service";
    let first = asked(&repo, prompt, ScaleMode::Medium)?;
    let second = asked(&repo, prompt, ScaleMode::Medium)?;
    assert_eq!(first, second);
    Ok(())
}

/// What the session read of a file since its context was last compacted, while the file still
/// holds it, is not shown again: the service's one span is in the session's context, so the
/// service is not listed at all, though it still seeds and its links still count. Typed out by
/// hand: the model, then the checkout, each with its code, the two code spans the work order
/// shows, so nothing is left for "Also likely".
#[test]
fn what_the_session_holds_is_not_shown_again() -> TestResult {
    let mut repo = repository()?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(
                ("s1", 2_100),
                ToolKind::Read,
                ("src/orders/service.py", &[(1, 4)]),
            )?,
        ],
    )?;
    let retrieval = asked(
        &repo,
        "fix create_order in the orders service",
        ScaleMode::Medium,
    )?;
    let Outcome::WorkOrder(_, rendered) = &retrieval.outcome else {
        return Err(format!("{:?}", retrieval.outcome).into());
    };
    let expected = "\
[cortex] Work order · the prompt's words and the commit history agree
Start here:
  1. src/orders/model.py:1-3  Order  (matches order, orders)
  2. src/payments/checkout.py:1-2  pay  (changes with service.py in 5 of 60 commits)
Current code:
  src/orders/model.py:1-3
  1 | class Order:
  2 |     def __init__(self):
  3 |         self.lines = []
  src/payments/checkout.py:1-2
  1 | def pay(order):
  2 |     return order
";
    assert_eq!(rendered.text, expected);
    let paths: Vec<&str> = retrieval
        .listed
        .iter()
        .map(|entry| entry.item.path.as_str())
        .collect();
    assert_eq!(paths, ["src/orders/model.py", "src/payments/checkout.py"]);
    Ok(())
}

/// Small mode holds a work order to 400 tokens, whatever `retrieval.max_tokens` allows. The
/// service's `create_order` is one line of 24 bytes and 40 of 59, 2,384 bytes, 596 tokens of code
/// at four bytes a token before the work order numbers its lines, so medium mode, at 900, shows its
/// last line; small mode lists the same files within 400 tokens, and its code does not reach that
/// line.
#[test]
fn small_mode_holds_the_work_order_to_400_tokens() -> TestResult {
    let mut repo = Repo::new()?;
    let mut body = String::from("def create_order(cart):\n");
    for line in 2..=41 {
        body.push_str(&format!(
            "    total = total + cart.item_{line:02}.price * quantity_of(cart)\n"
        ));
    }
    let service = repo.file(
        "src/orders/service.py",
        &body,
        &[function("create_order", 1, 41)],
        FileFlags::empty(),
    )?;
    let model = repo.file(
        "src/orders/model.py",
        MODEL,
        &[Def {
            name: "Order",
            kind: SymbolKind::Class,
            start: 1,
            end: 3,
            parent: None,
        }],
        FileFlags::empty(),
    )?;
    let node = |nodes: &[cortex_store::NodeId]| nodes.first().copied().ok_or("a file's node");
    let (service, model) = (node(&service)?, node(&model)?);
    repo.link((service, model), EdgeType::CoChange, (9.0 / 12.0, 9, 0))?;
    repo.link((model, service), EdgeType::CoChange, (9.0 / 10.0, 9, 0))?;
    let prompt = "fix create_order in the orders service";
    let text_in = |mode| -> TestResult<(String, u32)> {
        match asked(&repo, prompt, mode)?.outcome {
            Outcome::WorkOrder(_, rendered) => Ok((rendered.text, rendered.tokens)),
            Outcome::Silent(silence) => Err(format!("{silence:?}").into()),
        }
    };
    let (medium, medium_tokens) = text_in(ScaleMode::Medium)?;
    let (small, small_tokens) = text_in(ScaleMode::Small)?;
    let last = "  41 |     total = total + cart.item_41.price * quantity_of(cart)\n";
    assert!(medium_tokens > 400, "{medium_tokens}");
    assert!(medium.contains(last), "{medium}");
    assert!(small_tokens <= 400, "{small_tokens}");
    assert!(!small.contains(last), "{small}");
    for path in ["src/orders/service.py", "src/orders/model.py"] {
        assert!(small.contains(path), "{small}");
    }
    Ok(())
}

/// Without the gate, a question the gate would stay silent on is answered: the word "rate", written
/// as a plain word and not as code, matches the tax file's function, so the file is found with its
/// one span and the reason that it matches "rate", though nothing backs it; the gate, had it been
/// asked, would have stayed shut for too few kinds of evidence.
#[test]
fn the_engine_without_the_gate_answers_what_the_gate_would_not() -> TestResult {
    let repo = repository()?;
    let location = located(&repo, "change rate", (ScaleMode::Medium, None), 5)?;
    assert_eq!(paths_of(&location), ["src/billing/tax.py"]);
    let entry = location.listed.first().ok_or("one file found")?;
    assert_eq!(entry.item.reason, Reason::Matches(vec!["rate".to_owned()]));
    let lines: Vec<(u32, u32)> = entry
        .item
        .spans
        .iter()
        .map(|span| (span.lines.start_line(), span.lines.end_line()))
        .collect();
    assert_eq!(lines, [(1, 2)]);
    let (_, verdict) = location.judged.ok_or("the gate's view")?;
    assert_eq!(verdict, Verdict::Silent(Shut::TooFewKinds));
    Ok(())
}

/// A span inside a class carries the class's first line, which the work order shows above it: in
/// `cart.py`, `Cart` runs from line 1 to line 72, longer than a span may be (60 lines), and its
/// method `subtotal` from line 70 to line 72; whatever the span the question about `subtotal`
/// finds, it ends at 72, starts after line 1 and carries line 1 as its scope, written above its
/// code with a line marking the lines between.
#[test]
fn a_span_inside_a_class_carries_the_class_s_line() -> TestResult {
    let mut repo = Repo::new()?;
    let mut text = String::from("class Cart:\n");
    for line in 2..70 {
        text.push_str(&format!("    x_{line} = {line}\n"));
    }
    text.push_str(
        "    def subtotal(self):\n        total = sum(self.lines)\n        return total\n",
    );
    repo.file(
        "cart.py",
        &text,
        &[
            Def {
                name: "Cart",
                kind: SymbolKind::Class,
                start: 1,
                end: 72,
                parent: None,
            },
            Def {
                name: "subtotal",
                kind: SymbolKind::Method,
                start: 70,
                end: 72,
                parent: Some(0),
            },
        ],
        FileFlags::empty(),
    )?;
    repo.mode(ScaleMode::Medium)?;
    let location = located(&repo, "fix subtotal", (ScaleMode::Medium, None), 5)?;
    let entry = location.listed.first().ok_or("one file found")?;
    let span = entry.item.spans.first().ok_or("a span")?;
    assert!(
        span.lines.start_line() > 1 && span.lines.end_line() == 72,
        "{span:?}"
    );
    assert_eq!(span.scope, Some(1));
    let text = location
        .rendered
        .as_ref()
        .map(|rendered| rendered.text.as_str())
        .ok_or("a rendered answer")?;
    assert!(
        text.contains("   1 | class Cart:\n     | ...\n")
            && text.contains("  70 |     def subtotal"),
        "{text}"
    );
    Ok(())
}

/// The engine without the gate finds at most the files asked for, the best first, and says that
/// the gate would have let this one through on agreement.
#[test]
fn the_engine_without_the_gate_keeps_to_its_limit() -> TestResult {
    let repo = repository()?;
    let question = "fix create_order in the orders service";
    let location = located(&repo, question, (ScaleMode::Medium, None), 1)?;
    assert_eq!(paths_of(&location), ["src/orders/service.py"]);
    let (_, verdict) = location.judged.ok_or("the gate's view")?;
    assert_eq!(verdict, Verdict::Inject(Rule::Agreement));
    let all = located(&repo, question, (ScaleMode::Medium, None), 5)?;
    assert_eq!(
        paths_of(&all),
        [
            "src/orders/service.py",
            "src/orders/model.py",
            "src/payments/checkout.py"
        ]
    );
    Ok(())
}

/// The engine without the gate answers in tiny mode too, where nothing is injected: a question
/// asked directly is answered whatever the repository's size.
#[test]
fn the_engine_without_the_gate_answers_in_tiny_mode() -> TestResult {
    let repo = repository()?;
    let question = "fix create_order in the orders service";
    let location = located(&repo, question, (ScaleMode::Tiny, None), 5)?;
    assert_eq!(paths_of(&location).first(), Some(&"src/orders/service.py"));
    Ok(())
}

/// A question asked outside any session sees nothing of one: the tax file, edited in session `s1`,
/// is found for matching "rate", and the gate would stay shut; asked in `s1`, the file is found for
/// its edit, and the gate would let it through as the work under way going on.
#[test]
fn a_question_outside_a_session_sees_nothing_of_one() -> TestResult {
    let mut repo = repository()?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(
                ("s1", 2_100),
                ToolKind::Edit,
                ("src/billing/tax.py", &[(2, 2)]),
            )?,
        ],
    )?;
    let outside = located(&repo, "change rate", (ScaleMode::Medium, None), 5)?;
    let entry = outside.listed.first().ok_or("one file found")?;
    assert_eq!(entry.item.reason, Reason::Matches(vec!["rate".to_owned()]));
    let (_, verdict) = outside.judged.ok_or("the gate's view")?;
    assert_eq!(verdict, Verdict::Silent(Shut::TooFewKinds));
    let inside = located(&repo, "change rate", (ScaleMode::Medium, Some("s1")), 5)?;
    let entry = inside.listed.first().ok_or("one file found")?;
    assert_eq!(entry.item.reason, Reason::EditedHere);
    let (_, verdict) = inside.judged.ok_or("the gate's view")?;
    assert_eq!(verdict, Verdict::Inject(Rule::Continuity));
    Ok(())
}

/// A question that points at nothing finds nothing, and the gate is never asked.
#[test]
fn a_question_that_points_at_nothing_finds_nothing() -> TestResult {
    let repo = repository()?;
    let location = located(&repo, "make it better", (ScaleMode::Medium, None), 5)?;
    assert_eq!(
        location,
        Location {
            judged: None,
            listed: Vec::new(),
            rendered: None,
        }
    );
    Ok(())
}

/// A work order lists only what it wrote. With room for 50 tokens, the work order sheds both
/// bodies of code and then the checkout: the three items alone take 256 bytes, 64 tokens; the
/// first two, 181 bytes, 46 tokens, fit. Typed out by hand; the retrieval lists those two, each with
/// the span it wrote.
#[test]
fn a_work_order_lists_only_what_it_wrote() -> TestResult {
    let repo = repository()?;
    let settings = RetrievalConfig {
        max_tokens: 50,
        ..RetrievalConfig::default()
    };
    let retrieval = asked_with(
        &repo,
        "fix create_order in the orders service",
        ScaleMode::Medium,
        &settings,
    )?;
    let Outcome::WorkOrder(_, rendered) = &retrieval.outcome else {
        return Err(format!("{:?}", retrieval.outcome).into());
    };
    let expected = "\
[cortex] Work order · the prompt's words and the commit history agree
Also likely: src/orders/service.py:1-4 (named in the prompt), src/orders/model.py:1-3 (matches order, orders)
";
    assert_eq!(rendered.text, expected);
    let listed: Vec<(&str, Vec<(u32, u32)>)> = retrieval
        .listed
        .iter()
        .map(|entry| {
            let lines = entry
                .spans
                .iter()
                .map(|span| (span.lines.start_line(), span.lines.end_line()))
                .collect();
            (entry.item.path.as_str(), lines)
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("src/orders/service.py", vec![(1, 4)]),
            ("src/orders/model.py", vec![(1, 3)])
        ]
    );
    Ok(())
}

/// Each reason a file can be listed for comes from its own evidence. "fix create_order in the
/// orders service" names `create_order`, so the service seeds, and no other file holds a word of
/// the prompt. `submit` in the api calls `create_order`, a reference tied at 1.0: the api refers to
/// service.py. `create_order` calls `save` in the store at 0.9: the store is referred to by
/// service.py. `create_order` calls `bill` in the invoice too, but at 0.5 only, which carries
/// activation and says nothing; the invoice changed with the service in 4 of its 8 commits, a link
/// from the invoice only, the service's 4 of 50 being under the miner's least confidence of 0.1,
/// so the invoice changes with service.py. The views were worked on with the service, a co-access
/// link of 0.1, what one successful task gives. Earlier tasks whose first words are the prompt's four and "cart", 4 of 5 alike,
/// changed the history file. The session read the first of the form's three lines, so its one
/// span, the whole file, is not all in context. Typed out by hand, without the gate.
#[test]
fn each_reason_comes_from_its_own_evidence() -> TestResult {
    let mut repo = Repo::new()?;
    let service = repo.file(
        "src/orders/service.py",
        SERVICE,
        &[function("create_order", 1, 4)],
        FileFlags::empty(),
    )?;
    let api = repo.file(
        "src/web/api.py",
        "def submit(cart):\n    return create_order(cart)\n",
        &[function("submit", 1, 2)],
        FileFlags::empty(),
    )?;
    let store = repo.file(
        "src/db/store.py",
        "def save(total):\n    return total\n",
        &[function("save", 1, 2)],
        FileFlags::empty(),
    )?;
    let invoice = repo.file(
        "src/billing/invoice.py",
        "def bill(total):\n    return total\n",
        &[function("bill", 1, 2)],
        FileFlags::empty(),
    )?;
    let views = repo.file(
        "src/web/views.py",
        "def show(page):\n    return page\n",
        &[function("show", 1, 2)],
        FileFlags::empty(),
    )?;
    let history = repo.file(
        "src/jobs/history.py",
        "def replay(log):\n    return log\n",
        &[function("replay", 1, 2)],
        FileFlags::empty(),
    )?;
    repo.file(
        "src/web/form.py",
        "x = 1\ny = 2\nz = 3\n",
        &[],
        FileFlags::empty(),
    )?;
    let node =
        |nodes: &[cortex_store::NodeId], at: usize| nodes.get(at).copied().ok_or("a stored node");
    repo.link(
        (node(&api, 1)?, node(&service, 1)?),
        EdgeType::Structural,
        (1.0, 1, 0),
    )?;
    repo.link(
        (node(&service, 1)?, node(&store, 1)?),
        EdgeType::Structural,
        (0.9, 1, 0),
    )?;
    repo.link(
        (node(&service, 1)?, node(&invoice, 1)?),
        EdgeType::Structural,
        (0.5, 1, 0),
    )?;
    repo.link(
        (node(&invoice, 0)?, node(&service, 0)?),
        EdgeType::CoChange,
        (4.0 / 8.0, 4, 0),
    )?;
    repo.cochange(
        &[("src/orders/service.py", 50), ("src/billing/invoice.py", 8)],
        &[(("src/orders/service.py", "src/billing/invoice.py"), 4)],
    )?;
    repo.link(
        (node(&service, 0)?, node(&views, 0)?),
        EdgeType::CoAccess,
        (0.1, 1, 0),
    )?;
    repo.clusters(&[(
        &["create", "order", "orders", "service", "cart"],
        &[node(&history, 0)?],
        2,
    )])?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(
                ("s1", 2_100),
                ToolKind::Read,
                ("src/web/form.py", &[(1, 1)]),
            )?,
        ],
    )?;
    let location = located(
        &repo,
        "fix create_order in the orders service",
        (ScaleMode::Medium, Some("s1")),
        10,
    )?;
    let reasons: std::collections::BTreeMap<&str, &Reason> = location
        .listed
        .iter()
        .map(|entry| (entry.item.path.as_str(), &entry.item.reason))
        .collect();
    let service_py = "service.py".to_owned();
    let changes_with = Reason::ChangesWith {
        other: service_py.clone(),
        together: 4,
        of: 8,
    };
    let refers_to = Reason::RefersTo(service_py.clone());
    let referred_to_by = Reason::ReferredToBy(service_py.clone());
    let worked_on_with = Reason::WorkedOnWith(service_py);
    let expected: std::collections::BTreeMap<&str, &Reason> = [
        ("src/billing/invoice.py", &changes_with),
        ("src/db/store.py", &referred_to_by),
        ("src/jobs/history.py", &Reason::LikeEarlierWork),
        ("src/orders/service.py", &Reason::Named),
        ("src/web/api.py", &refers_to),
        ("src/web/form.py", &Reason::ReadHere),
        ("src/web/views.py", &worked_on_with),
    ]
    .into_iter()
    .collect();
    assert_eq!(reasons, expected);
    assert_eq!(location.listed.len(), 7);
    Ok(())
}

/// A work order that would show none of the three files the gate weighed is not given. The session
/// holds the whole of the service, the model and the checkout, each read and so each seeded at 0.4
/// besides what else points at it: those three are the best files. The checks file, reached only
/// through the 0.5 call and as a dark neighbour at 0.05, changed with the service in 3 of its 6
/// commits, so it has a true reason, and the engine without the gate lists it alone; but the gate
/// weighed the other three, so the work order stays silent.
#[test]
fn a_work_order_showing_none_of_the_files_weighed_is_not_given() -> TestResult {
    let mut repo = repository()?;
    let counted = repo.reader()?.queries().cochange_files()?;
    let node = |path: &str| {
        counted
            .iter()
            .find(|file| file.path.as_str() == path)
            .map(|file| file.node)
            .ok_or_else(|| format!("{path} is stored"))
    };
    let (checks, service) = (node("src/lib/checks.py")?, node("src/orders/service.py")?);
    repo.link((checks, service), EdgeType::CoChange, (3.0 / 6.0, 3, 0))?;
    repo.cochange(
        &[("src/lib/checks.py", 6)],
        &[(("src/lib/checks.py", "src/orders/service.py"), 3)],
    )?;
    record(
        &mut repo,
        &[
            event("s1", 1_000, HookKind::SessionStart)?,
            event("s1", 2_000, HookKind::Prompt)?,
            tool(
                ("s1", 2_100),
                ToolKind::Read,
                ("src/orders/service.py", &[(1, 4)]),
            )?,
            tool(
                ("s1", 2_200),
                ToolKind::Read,
                ("src/orders/model.py", &[(1, 3)]),
            )?,
            tool(
                ("s1", 2_300),
                ToolKind::Read,
                ("src/payments/checkout.py", &[(1, 2)]),
            )?,
        ],
    )?;
    let prompt = "fix create_order in the orders service";
    let location = located(&repo, prompt, (ScaleMode::Medium, Some("s1")), 5)?;
    assert_eq!(paths_of(&location), ["src/lib/checks.py"]);
    let retrieval = asked(&repo, prompt, ScaleMode::Medium)?;
    assert_eq!(retrieval.outcome, Outcome::Silent(Silence::NothingToShow));
    assert!(retrieval.features.is_some());
    assert!(retrieval.listed.is_empty());
    Ok(())
}

/// A huge repository spreads over files first, then over the symbols of the best files. Here it
/// gives the work order medium mode gives, typed out by hand above: each file listed has one
/// symbol, so the spans are the same, and the evidence is read from the whole graph either way. The
/// gate weighs the files-first activation, which holds nothing on a symbol, so its activation mass
/// is not the one a single spreading over symbols gives.
#[test]
fn a_huge_repository_spreads_over_files_then_symbols() -> TestResult {
    let repo = repository()?;
    let prompt = "fix create_order in the orders service";
    let medium = asked(&repo, prompt, ScaleMode::Medium)?;
    let huge = asked(&repo, prompt, ScaleMode::Huge)?;
    assert_eq!(huge.outcome, medium.outcome);
    let (medium, huge) = (
        medium.features.ok_or("the gate's features")?,
        huge.features.ok_or("the gate's features")?,
    );
    assert!(
        (medium.top_mass - huge.top_mass).abs() > 1e-9,
        "{} {}",
        medium.top_mass,
        huge.top_mass
    );
    Ok(())
}

/// The engine without the gate writes its answer under a header of its own, saying what found the
/// files and what the gate would have said. Typed out by hand: "rate" finds the tax file by the
/// prompt's words alone, so the gate would stay silent for too few kinds of evidence; the file is
/// listed with its code.
#[test]
fn the_engine_without_the_gate_writes_its_own_answer() -> TestResult {
    let repo = repository()?;
    let location = located(&repo, "change rate", (ScaleMode::Medium, None), 5)?;
    let rendered = location.rendered.ok_or("an answer")?;
    let expected = "\
[cortex] Where · the prompt's words alone · the gate would stay silent: too few kinds of evidence agree
Start here:
  1. src/billing/tax.py:1-2  rate  (matches rate)
Current code:
  src/billing/tax.py:1-2
  1 | def rate():
  2 |     return 0.2
";
    assert_eq!(rendered.text, expected);
    let question = "fix create_order in the orders service";
    let agreed = located(&repo, question, (ScaleMode::Medium, None), 1)?;
    let header = agreed
        .rendered
        .as_ref()
        .and_then(|rendered| rendered.text.lines().next())
        .ok_or("a header")?;
    assert_eq!(
        header,
        "[cortex] Where · the prompt's words and the commit history agree · the gate would give it"
    );
    Ok(())
}

/// A retrieval the gate keeps back keeps the files it weighed: for "change rate", the tax file
/// alone, the one file reached; one that finds nothing weighed nothing.
#[test]
fn a_retrieval_keeps_the_files_the_gate_weighed() -> TestResult {
    let repo = repository()?;
    let retrieval = asked(&repo, "change rate", ScaleMode::Medium)?;
    assert_eq!(
        retrieval.outcome,
        Outcome::Silent(Silence::Gate(Shut::TooFewKinds))
    );
    let tax = repo
        .reader()?
        .queries()
        .cochange_files()?
        .into_iter()
        .find(|file| file.path.as_str() == "src/billing/tax.py")
        .map(|file| file.node)
        .ok_or("the tax file is stored")?;
    assert_eq!(retrieval.weighed, [tax]);
    let vague = asked(&repo, "make it better", ScaleMode::Medium)?;
    assert!(vague.weighed.is_empty());
    Ok(())
}
