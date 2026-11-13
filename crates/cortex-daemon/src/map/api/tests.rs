//! The map's API read through the site on a scripted repository (see [`script`]): whole bodies
//! for the small cases, with every number counted by hand in the comments, the same store read
//! twice, and each endpoint's refusals.
//!
//! The scripted repository, in short. Files by path, which are their ids: 0 `docs/guide.md`,
//! 1 `src/app.rs`, 2 `src/orders.rs` (defining `place_order` on line 10 and `OrderBook` on line
//! 40), 3 `src/orders_test.rs`, 4 `src/util.rs` (vendored, so its commits are not counted). The
//! weights bring interest and links of use forward by 0.5 for each task since they were written;
//! the second of the two tasks is the latest.

mod health;
mod history;
mod script;
mod symbols;

use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::Request;
use cortex_core::Config;
use cortex_retrieve::GraphSettings;

use self::script::{cold, script};
use super::super::MapSettings;
use super::super::assets::Asset;
use super::super::site::Site;
use super::super::source::Source;
use super::super::token::Token;
use crate::asked::AtWork;
use crate::graph::Snapshot;
use crate::history::Jobs;
use crate::inbox::{Inbox, Work};
use crate::test_store::{TestResult, add_file, writer};

/// The port the site pretends to listen on.
const PORT: u16 = 8123;

/// The time the summary's window is measured from: seven days and 2,200 ms after the epoch, so
/// that the window starts at 2,200 ms.
const NOW_MS: i64 = 7 * 86_400_000 + 2_200;

/// What an answer gave: its status and its body.
type Given = (u16, String);

/// A site over a store, and the address part its requests carry.
struct Fixture {
    /// The site.
    site: Arc<Site>,
    /// Its token.
    token: String,
    /// What tells the site the store changed, as the daemon's writer does.
    snapshot: Arc<Snapshot>,
    /// The folder the store is in, kept alive for the test.
    root: tempfile::TempDir,
}

/// A site over `root`'s store, reading commit subjects from `texts`.
fn site_over(root: tempfile::TempDir, texts: Arc<Jobs>) -> TestResult<Fixture> {
    site_told(root, texts, Arc::default())
}

/// A site like [`site_over`]'s whose daemon's inbox is `inbox`, where its writes are asked.
fn site_told(root: tempfile::TempDir, texts: Arc<Jobs>, inbox: Arc<Inbox>) -> TestResult<Fixture> {
    site_reporting(root, (texts, inbox), None)
}

/// A site like [`site_told`]'s whose daemon's status report comes from `status`, when one does.
fn site_reporting(
    root: tempfile::TempDir,
    (texts, inbox): (Arc<Jobs>, Arc<Inbox>),
    status: Option<std::sync::Weak<dyn crate::map::Status>>,
) -> TestResult<Fixture> {
    let weights = GraphSettings {
        link_decay: 0.5,
        interest_decay: 0.5,
        ..GraphSettings::default()
    };
    let snapshot = Arc::new(Snapshot::default());
    let source = Source::new(
        (root.path().join(".cortex"), 0),
        weights,
        (texts, Arc::clone(&snapshot), inbox),
        MapSettings::of(&Config::default(), root.path()),
    )
    .with_clock(|| NOW_MS);
    let source = match status {
        Some(status) => source.with_status(status),
        None => source,
    };
    let token = Token::generate()?;
    let token_text = token.as_str().to_owned();
    let assets: &'static [Asset] = &[];
    Ok(Fixture {
        site: Arc::new(Site::new(PORT, token, assets, Arc::new(source))?),
        token: token_text,
        snapshot,
        root,
    })
}

/// The scripted repository's site.
fn scripted() -> TestResult<Fixture> {
    let root = tempfile::tempdir()?;
    let texts = script(root.path())?;
    site_over(root, texts)
}

/// What the site answers a GET of `path` under its token.
fn get(fixture: &Fixture, path: &str) -> TestResult<Given> {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/{}/{path}", fixture.token))
        .header("host", format!("127.0.0.1:{PORT}"))
        .body(())?;
    let (mut parts, ()) = request.into_parts();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let response = fixture.site.answer(&mut parts).await;
        let status = response.status().as_u16();
        let body = to_bytes(response.into_body(), 1 << 24).await?;
        Ok((status, String::from_utf8(body.to_vec())?))
    })
}

/// The whole snapshot. Interest: `src/app.rs` stored 2.0 at the latest task, `src/orders.rs` 1.0
/// at the one before, which is 0.5 now; `src/util.rs`'s 0.001 is below the floor and counts as
/// none; over the highest, 2.0, that is 1.0 and 0.25. Tokens, each response counted for the file
/// its session last read or edited: session s1 read `src/app.rs` at 2,000 and 2,100, read
/// `src/orders.rs` at 2,200, edited it at 2,300 (and the watcher's edit of `src/util.rs` at 2,400
/// is the user's, not an agent's), so its responses at 2,050 (100 + 50 = 150) are the app's, at
/// 2,250 (10 + 20 + 30 + 40 = 100) and 2,350 (100 + 100 = 200) the orders' 300; session s2 read
/// `docs/guide.md` at 3,000 and its response at 3,100 took 300 + 100 = 400. Reads: the app 2,
/// the orders file and the guide 1. Facts: the orders file has three (active, a candidate on its
/// symbol, one suspect), the app one (its retired one does not count). Commits: 9, 8 and 4 as
/// stored. Links: co-change, the larger confidence of each pair, 0.8 of 6 commits between files 1
/// and 2 (the other direction says 0.5), 0.9 of 4 commits between 2 and 3 (the other says 0.6);
/// used together, files 1 and 4 at 2.5 written a task ago, 1.25 now, over the most a link can
/// weigh, 5: 0.25; work orders, two named files 1 and 2 together, one named 1 and 3 and one 2 and
/// 3 (the third work order names a file alone): over 2, 1.0, 0.5 and 0.5. Each link's count is
/// what its weight was made from: the co-change links' 6 and 4 commits, the one task that linked
/// files 1 and 4, and the work orders' 2, 1 and 1.
#[test]
fn the_graph_is_the_store_as_counted_by_hand() -> TestResult {
    let fixture = scripted()?;
    let files = [
        r#"{"id":0,"path":"docs/guide.md","lines":10,"interest":0.0,"tokens":400,"touched_ms":3000,"facts":0,"suspect":0,"reads":1,"commits":0,"commits_counted":true}"#,
        r#"{"id":1,"path":"src/app.rs","lines":120,"interest":1.0,"tokens":150,"touched_ms":2100,"facts":1,"suspect":0,"reads":2,"commits":9,"commits_counted":true}"#,
        r#"{"id":2,"path":"src/orders.rs","lines":80,"interest":0.25,"tokens":300,"touched_ms":2300,"facts":3,"suspect":1,"reads":1,"commits":8,"commits_counted":true}"#,
        r#"{"id":3,"path":"src/orders_test.rs","lines":30,"interest":0.0,"tokens":0,"touched_ms":null,"facts":0,"suspect":0,"reads":0,"commits":4,"commits_counted":true}"#,
        r#"{"id":4,"path":"src/util.rs","lines":5,"interest":0.0,"tokens":0,"touched_ms":null,"facts":0,"suspect":0,"reads":0,"commits":0,"commits_counted":false}"#,
    ];
    let links = [
        r#"{"from":1,"to":2,"kind":"co_change","weight":0.8,"commits":6,"count":6}"#,
        r#"{"from":2,"to":3,"kind":"co_change","weight":0.9,"commits":4,"count":4}"#,
        r#"{"from":1,"to":4,"kind":"used_together","weight":0.25,"commits":0,"count":1}"#,
        r#"{"from":1,"to":2,"kind":"work_order","weight":1.0,"commits":0,"count":2}"#,
        r#"{"from":1,"to":3,"kind":"work_order","weight":0.5,"commits":0,"count":1}"#,
        r#"{"from":2,"to":3,"kind":"work_order","weight":0.5,"commits":0,"count":1}"#,
    ];
    let expected = format!(
        r#"{{"version":1,"files":[{}],"links":[{}]}}"#,
        files.join(","),
        links.join(",")
    );
    assert_eq!(get(&fixture, "api/graph")?, (200, expected));
    Ok(())
}

/// A repository where nothing was learned has its files and no links, and says so in the summary.
#[test]
fn a_cold_repository_shows_its_structure_alone() -> TestResult {
    let root = tempfile::tempdir()?;
    cold(root.path())?;
    let fixture = site_over(root, Arc::default())?;
    let file = |id, path, lines| {
        format!(
            r#"{{"id":{id},"path":"{path}","lines":{lines},"interest":0.0,"tokens":0,"touched_ms":null,"facts":0,"suspect":0,"reads":0,"commits":0,"commits_counted":true}}"#
        )
    };
    let expected = format!(
        r#"{{"version":1,"files":[{},{}],"links":[]}}"#,
        file(0, "README.md", 3),
        file(1, "src/main.rs", 9)
    );
    assert_eq!(get(&fixture, "api/graph")?, (200, expected));
    let summary = r#"{"version":1,"files":2,"folders":1,"explored_files":0,"learned":false,"strongest_pair":null,"busiest_folder":null,"window_days":7,"history":{"max_commits":20000,"max_age_days":730,"max_files_per_commit":30}}"#;
    assert_eq!(get(&fixture, "api/summary")?, (200, summary.to_owned()));
    Ok(())
}

/// The first screen's numbers: 5 files in the 2 folders `docs` and `src`; 3 files read (the
/// guide, the app and the orders file); something is learned; the strongest pair of files that
/// changed together is 1 and 2, ranked as the graph ranks its links, by confidence times
/// ln(1 + commits): 0.8 ln 7 = 1.557 over 2 and 3's 0.9 ln 5 = 1.448, so the pair named is always
/// one the graph draws; of the tokens since the window
/// began at 2,200 ms, `src` took 100 + 200 = 300 (the app's response at 2,050 is before it) and
/// `docs` took 400, so `docs` is the busiest. The history the commits count is the default
/// configuration's: the newest 20,000 commits within 730 days, of at most 30 files each.
#[test]
fn the_summary_is_the_first_screen_as_counted_by_hand() -> TestResult {
    let fixture = scripted()?;
    let expected = r#"{"version":1,"files":5,"folders":2,"explored_files":3,"learned":true,"strongest_pair":{"from":1,"to":2,"commits":6},"busiest_folder":{"path":"docs","tokens":400},"window_days":7,"history":{"max_commits":20000,"max_age_days":730,"max_files_per_commit":30}}"#;
    assert_eq!(get(&fixture, "api/summary")?, (200, expected.to_owned()));
    Ok(())
}

/// The receipts of the last 7 days: the window starts at 2,200 ms, so session `s2` (started at
/// 2,900) is shown and `s1` (started at 100) is not. Both tasks were decided treated, none held
/// out, so the arms are 0 and 2 against the 30 each needs, with no comparison. With no user price
/// file the shipped table prices them, which the answer says. The rest is the meter's report of
/// the same store, read the same way.
#[test]
fn the_receipts_are_the_last_week_of_the_report() -> TestResult {
    let fixture = scripted()?;
    let (status, body) = get(&fixture, "api/receipts")?;
    assert_eq!(status, 200);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(answer["version"], 1);
    assert_eq!(answer["window_days"], 7);
    assert_eq!(answer["prices"]["state"], "shipped");
    let holdout = &answer["recorded"]["holdout"];
    assert_eq!(holdout["held_out"], 0);
    assert_eq!(holdout["treated"], 2);
    assert_eq!(holdout["needed_per_arm"], 30);
    assert_eq!(holdout["comparison"], serde_json::Value::Null);
    let sessions = answer["recorded"]["sessions"]
        .as_array()
        .ok_or("no sessions")?;
    let ids: Vec<&str> = sessions
        .iter()
        .filter_map(|session| session["session_id"].as_str())
        .collect();
    assert_eq!(ids, ["s2"]);
    let reader = cortex_store::Reader::open(&fixture.root.path().join(".cortex"), 0)?;
    let table = cortex_meter::PriceTable::parse(cortex_meter::SHIPPED_PRICES)?;
    let report = cortex_meter::recorded(
        reader.queries(),
        &cortex_meter::Selection::StartedSince(2_200),
        Some(&table),
    )?;
    assert_eq!(answer["recorded"], serde_json::to_value(&report)?);
    assert_eq!(get(&fixture, "api/receipts/1")?.0, 404);
    Ok(())
}

/// What the site answers a POST of `path` under its token from the map's own page.
fn post(fixture: &Fixture, path: &str) -> TestResult<Given> {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/{}/{path}", fixture.token))
        .header("host", format!("127.0.0.1:{PORT}"))
        .header("origin", format!("http://127.0.0.1:{PORT}"))
        .body(())?;
    let (mut parts, ()) = request.into_parts();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let response = fixture.site.answer(&mut parts).await;
        let status = response.status().as_u16();
        let body = to_bytes(response.into_body(), 1 << 24).await?;
        Ok((status, String::from_utf8(body.to_vec())?))
    })
}

/// The person's word on a fact, sent from the map's page, is carried out by the daemon's writer
/// (here a thread taking the inbox's requests as the daemon's does) and answered once written:
/// fact 2, a candidate about `src/orders.rs`, confirmed is active; fact 1, active, retired is gone
/// from the file's panel; fact 99 does not exist, and the writer's refusal says so with 409. The
/// orders file's panel then lists fact 2 active and fact 3 still suspect, fact 1 no more.
#[test]
fn a_fact_s_word_from_the_page_is_written_by_the_daemon_s_writer() -> TestResult {
    let root = tempfile::tempdir()?;
    let texts = script(root.path())?;
    let inbox = Arc::new(Inbox::default());
    let fixture = site_told(root, texts, Arc::clone(&inbox))?;
    let mut store = writer(fixture.root.path())?;
    let taking = Arc::clone(&inbox);
    let worker = std::thread::spawn(move || -> Result<usize, String> {
        let timing = cortex_coord::Timing::of(&cortex_core::CoordinationConfig::default());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut done = 0;
        while done < 3 && std::time::Instant::now() < until {
            if let Work::Ask(job) = taking.wait(Some(until)) {
                job.carry_out(&mut store, (NOW_MS, &AtWork::default(), &timing))
                    .map_err(|error| error.to_string())?;
                done += 1;
            }
        }
        Ok(done)
    });
    let confirmed = post(&fixture, "api/facts/2/confirm")?;
    let retired = post(&fixture, "api/facts/1/retire")?;
    let missing = post(&fixture, "api/facts/99/confirm")?;
    let taken = worker.join().map_err(|_| "the writer thread panicked")??;
    assert_eq!(taken, 3);
    assert_eq!(
        confirmed,
        (200, r#"{"version":1,"id":2,"status":"active"}"#.to_owned())
    );
    assert_eq!(
        retired,
        (200, r#"{"version":1,"id":1,"status":"retired"}"#.to_owned())
    );
    assert_eq!(missing.0, 409);
    assert!(missing.1.contains("there is no fact 99"), "{}", missing.1);
    let (_, panel) = get(&fixture, "api/file/2")?;
    assert!(
        panel.contains(r#""facts":[{"id":2,"kind":"convention","status":"active","#),
        "{panel}"
    );
    assert!(
        panel.contains(r#"{"id":3,"kind":"convention","status":"suspect","#),
        "{panel}"
    );
    assert!(!panel.contains(r#"{"id":1,"#), "{panel}");
    assert_eq!(post(&fixture, "api/facts/2/edit")?.0, 404);
    // A GET of a write's path writes nothing: it is no endpoint, and the writer is asked nothing.
    assert_eq!(get(&fixture, "api/facts/3/retire")?.0, 404);
    let soon = std::time::Instant::now() + std::time::Duration::from_millis(200);
    assert!(
        !matches!(inbox.wait(Some(soon)), Work::Ask(_)),
        "a GET asked the writer"
    );
    Ok(())
}

/// The activity feed: the agents' events, newest first as stored. Session `s2` read the guide
/// (id 0) at 3,000; session `s1` edited the orders file (id 2) at 2,300, read it at 2,200 and read
/// the app (id 1) at 2,100 and 2,000. The watcher's edit of `src/util.rs` at 2,400 is no agent's
/// and is left out. Both sessions are Claude Code's.
#[test]
fn the_activity_is_the_agents_events_newest_first() -> TestResult {
    let fixture = scripted()?;
    let event = |at: i64, kind: &str, session: &str, file: usize| {
        format!(
            r#"{{"at_ms":{at},"kind":"{kind}","session":"{session}","agent":"claude","title":"Claude Code","file":{file}}}"#
        )
    };
    let events = [
        event(3_000, "read", "s2", 0),
        event(2_300, "edit", "s1", 2),
        event(2_200, "read", "s1", 2),
        event(2_100, "read", "s1", 1),
        event(2_000, "read", "s1", 1),
    ];
    let expected = format!(r#"{{"version":1,"events":[{}]}}"#, events.join(","));
    assert_eq!(get(&fixture, "api/activity")?, (200, expected));
    assert_eq!(get(&fixture, "api/activity/1")?.0, 404);
    Ok(())
}

/// The warnings, newest first: at 2,320 an edit of the orders file (id 2) was warned of its test
/// (id 3), which was not then read or edited; at 2,310 an edit of the app (id 1) was warned of the
/// orders file, which usually changes with it, and its task has not ended, so it is open. Both
/// were written with strength 0.5.
#[test]
fn the_warnings_are_listed_newest_first_with_what_became_of_them() -> TestResult {
    let fixture = scripted()?;
    let expected = concat!(
        r#"{"version":1,"warnings":["#,
        r#"{"at_ms":2320,"kind":"test","source":2,"target":3,"strength":0.5,"state":"not_followed"},"#,
        r#"{"at_ms":2310,"kind":"cochange","source":1,"target":2,"strength":0.5,"state":"open"}"#,
        "]}"
    );
    assert_eq!(get(&fixture, "api/warnings")?, (200, expected.to_owned()));
    Ok(())
}

/// A search finds files by path and symbols by name, ASCII case ignored, exact names first, then
/// names that start with the text, then the rest, each by path and line. For `order`: the file
/// `orders.rs` (a name that starts with it) and the symbol `OrderBook` of the same path (line 40),
/// then `orders_test.rs`, then `place_order`, whose name only holds it. `ORDERBOOK` finds its
/// exact name alone, and the query is given back as typed.
#[test]
fn a_search_ranks_exact_names_then_starts_then_the_rest() -> TestResult {
    let fixture = scripted()?;
    let order = r#"{"version":1,"query":"order","results":[{"kind":"file","file":2,"name":"orders.rs","line":null},{"kind":"symbol","file":2,"name":"OrderBook","line":40},{"kind":"file","file":3,"name":"orders_test.rs","line":null},{"kind":"symbol","file":2,"name":"place_order","line":10}]}"#;
    assert_eq!(
        get(&fixture, "api/search?q=order")?,
        (200, order.to_owned())
    );
    let exact = r#"{"version":1,"query":"ORDERBOOK","results":[{"kind":"symbol","file":2,"name":"OrderBook","line":40}]}"#;
    assert_eq!(
        get(&fixture, "api/search?q=ORDERBOOK&other=1")?,
        (200, exact.to_owned())
    );
    let none = r#"{"version":1,"query":"xyzzy","results":[]}"#;
    assert_eq!(get(&fixture, "api/search?q=xyzzy")?, (200, none.to_owned()));
    Ok(())
}

/// A search with no text, an empty text or more than 100 characters is refused with 400 and no
/// body; a text of exactly 100 characters is searched; a path under `search` is 404.
#[test]
fn a_search_refuses_what_is_not_one_to_100_characters() -> TestResult {
    let fixture = scripted()?;
    let hundred = "a".repeat(100);
    let long = "a".repeat(101);
    for (path, status) in [
        ("api/search".to_owned(), 400),
        ("api/search?q=".to_owned(), 400),
        ("api/search?other=order".to_owned(), 400),
        (format!("api/search?q={long}"), 400),
        (format!("api/search?q={hundred}"), 200),
        ("api/search/x?q=order".to_owned(), 404),
    ] {
        let (found, body) = get(&fixture, &path)?;
        assert_eq!(found, status, "{path}");
        if status != 200 {
            assert_eq!(body, "", "{path}");
        }
    }
    Ok(())
}

/// `answer`'s body with its editor link checked and written as `EDITOR`: the default address,
/// `vscode://file`, then the repository's root as a URL path (which differs from run to run, so
/// only its start is checked here; `settings.rs` checks the whole form) and the file's `path` at
/// line 1.
fn with_editor(answer: (u16, String), path: &str) -> TestResult<(u16, String)> {
    let (status, body) = answer;
    let value: serde_json::Value = serde_json::from_str(&body)?;
    let link = value
        .get("editor_url")
        .and_then(serde_json::Value::as_str)
        .ok_or("the panel has no editor link")?;
    assert!(link.starts_with("vscode://file/"), "{link}");
    assert!(link.ends_with(&format!("/{path}:1")), "{link}");
    let written = format!(r#""editor_url":"{link}""#);
    assert_eq!(body.matches(&written).count(), 1, "{body}");
    Ok((status, body.replace(&written, r#""editor_url":"EDITOR""#)))
}

/// The orders file's panel: its editor link, then its facts, by number: 1 active, 2 a candidate,
/// 3 suspect, as stored. History: the blame names commits 1 (a pull request, 482, and issue
/// `#12`), 2 (neither) and a third whose subject the cache does not hold, which is left out;
/// newest first, so 1 then 2, two in all; the worker never started, so its reader is off. Told: two
/// work orders that named the file (at 1,500 and 1,600), whose task read and edited it, so used,
/// and a warning at 2,310 whose task has not ended, so not known; latest first.
#[test]
fn a_file_s_panel_is_its_facts_history_and_what_it_was_told() -> TestResult {
    let fixture = scripted()?;
    let facts = [
        r#"{"id":1,"kind":"convention","status":"active","text":"Orders are charged in integer cents."}"#,
        r#"{"id":2,"kind":"convention","status":"candidate","text":"Orders may be cancelled once."}"#,
        r#"{"id":3,"kind":"convention","status":"suspect","text":"Totals never round."}"#,
    ];
    let history = [
        r##"{"subject":"Charge in integer cents","refs":"#482 #12","date_s":1762300000}"##,
        r#"{"subject":"Round tax once","refs":"","date_s":1762000000}"#,
    ];
    let symbols = [
        r#"{"name":"place_order","kind":"function","start_line":10,"end_line":11,"is_nested":false,"selected":0}"#,
        r#"{"name":"OrderBook","kind":"class","start_line":40,"end_line":41,"is_nested":false,"selected":0}"#,
    ];
    let told = [
        r#"{"kind":"warning","at_ms":2310,"used":null}"#,
        r#"{"kind":"work_order","at_ms":1600,"used":true}"#,
        r#"{"kind":"work_order","at_ms":1500,"used":true}"#,
    ];
    let expected = format!(
        r#"{{"version":1,"id":2,"path":"src/orders.rs","editor_url":"EDITOR","facts":[{}],"history":[{}],"history_total":2,"history_reader":"off","symbols":[{}],"symbols_total":2,"selected":[{}],"told":[{}]}}"#,
        facts.join(","),
        history.join(","),
        symbols.join(","),
        r#"{"start_line":1,"end_line":2,"orders":2}"#,
        told.join(",")
    );
    assert_eq!(
        with_editor(get(&fixture, "api/file/2")?, "src/orders.rs")?,
        (200, expected)
    );
    Ok(())
}

/// The test file's panel: no facts or history; told, latest first, by a work order of the second
/// task (at 3,050, whose task has not ended: not known), a warning at 2,320 that was resolved as
/// not followed, and a work order at 1,600 whose task never read or edited the file: not used.
/// A file nothing is known of has empty lists.
#[test]
fn a_file_the_agent_did_not_follow_up_is_marked_unused() -> TestResult {
    let fixture = scripted()?;
    let told = [
        r#"{"kind":"work_order","at_ms":3050,"used":null}"#,
        r#"{"kind":"warning","at_ms":2320,"used":false}"#,
        r#"{"kind":"work_order","at_ms":1600,"used":false}"#,
    ];
    let expected = format!(
        r#"{{"version":1,"id":3,"path":"src/orders_test.rs","editor_url":"EDITOR","facts":[],"history":[],"history_total":0,"history_reader":"off","symbols":[],"symbols_total":0,"selected":[{}],"told":[{}]}}"#,
        r#"{"start_line":1,"end_line":2,"orders":2}"#,
        told.join(",")
    );
    assert_eq!(
        with_editor(get(&fixture, "api/file/3")?, "src/orders_test.rs")?,
        (200, expected)
    );
    let bare = r#"{"version":1,"id":0,"path":"docs/guide.md","editor_url":"EDITOR","facts":[],"history":[],"history_total":0,"history_reader":"off","symbols":[],"symbols_total":0,"selected":[],"told":[]}"#;
    assert_eq!(
        with_editor(get(&fixture, "api/file/0")?, "docs/guide.md")?,
        (200, bare.to_owned())
    );
    Ok(())
}

/// An id that names no file is 404 with an empty body: past the last, not a number, signed, empty,
/// followed by more path, or too long to be an index.
#[test]
fn a_file_id_that_is_not_a_file_is_not_found() -> TestResult {
    let fixture = scripted()?;
    for path in [
        "api/file/5",
        "api/file/99999999999",
        "api/file/abc",
        "api/file/-1",
        "api/file/+1",
        "api/file/",
        "api/file",
        "api/file/1/x",
        "api/file/01x",
    ] {
        assert_eq!(get(&fixture, path)?, (404, String::new()), "{path}");
    }
    Ok(())
}

/// The same store read twice gives the same bytes, each endpoint; so does a second site over it.
#[test]
fn the_same_store_gives_the_same_bytes() -> TestResult {
    let fixture = scripted()?;
    for path in [
        "api/graph",
        "api/summary",
        "api/search?q=order",
        "api/file/2",
        "api/file/3",
    ] {
        let first = get(&fixture, path)?;
        assert_eq!(first.0, 200, "{path}");
        assert_eq!(get(&fixture, path)?, first, "{path}");
    }
    Ok(())
}

/// What is not an endpoint, or names more than one, is 404; a store that cannot be read gives 500
/// with an empty body, and the health, which reads none, still answers.
#[test]
fn unknown_paths_are_not_found_and_an_unreadable_store_is_a_500() -> TestResult {
    let fixture = scripted()?;
    for path in [
        "api/nothing",
        "api/graph/x",
        "api/summary/",
        "api/health/x",
        "api/",
    ] {
        assert_eq!(get(&fixture, path)?, (404, String::new()), "{path}");
    }
    let without = site_over(tempfile::tempdir()?, Arc::default())?;
    for path in ["api/graph", "api/summary", "api/search?q=a", "api/file/0"] {
        assert_eq!(get(&without, path)?, (500, String::new()), "{path}");
    }
    assert_eq!(get(&without, "api/health")?.0, 200);
    Ok(())
}

/// The files are read once and kept until the daemon notes a change. After one build, five
/// requests across the four endpoints build nothing more. A file added to the store without the
/// daemon noting it is not seen: the graph is the bytes it was, which also shows that nothing
/// reread the files. Once the daemon notes a change, the next request builds a second listing and
/// shows the new file (6 files, the new one last by path); the ones after it build no third.
#[test]
fn the_listing_is_built_once_and_again_only_after_a_change() -> TestResult {
    let fixture = scripted()?;
    let first = get(&fixture, "api/graph")?;
    assert_eq!(fixture.site.listings_built(), 1);
    for path in [
        "api/summary",
        "api/search?q=order",
        "api/file/2",
        "api/file/3",
    ] {
        assert_eq!(get(&fixture, path)?.0, 200, "{path}");
    }
    assert_eq!(fixture.site.listings_built(), 1);
    add_file(&mut writer(fixture.root.path())?, "src/zzz.rs", 1)?;
    assert_eq!(get(&fixture, "api/graph")?, first);
    assert_eq!(fixture.site.listings_built(), 1);
    fixture.snapshot.changed();
    let (status, changed) = get(&fixture, "api/graph")?;
    assert_eq!(status, 200);
    assert!(
        changed.contains(r#"{"id":5,"path":"src/zzz.rs","#),
        "{changed}"
    );
    assert_eq!(fixture.site.listings_built(), 2);
    assert_eq!(get(&fixture, "api/graph")?, (200, changed));
    assert_eq!(fixture.site.listings_built(), 2);
    Ok(())
}

/// Requests that arrive together at a cold or out-of-date listing share one build: eight threads
/// ask at once and the files are read once.
#[test]
fn requests_that_arrive_together_share_one_build() -> TestResult {
    let fixture = scripted()?;
    let sizes: Vec<usize> = std::thread::scope(|scope| {
        let asking: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| fixture.site.listing().map(|listing| listing.files().len())))
            .collect();
        asking
            .into_iter()
            .map(|asked| {
                asked
                    .join()
                    .map_err(|_| "a thread panicked")?
                    .map_err(|e| e.to_string().into())
            })
            .collect::<Result<_, Box<dyn std::error::Error>>>()
    })?;
    assert_eq!(sizes, [5; 8]);
    assert_eq!(fixture.site.listings_built(), 1);
    Ok(())
}
