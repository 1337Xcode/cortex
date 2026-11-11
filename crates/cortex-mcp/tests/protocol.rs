//! The server spoken to by an MCP client over an in-memory pipe, its daemon a fake that records
//! each request and answers as told: the tools and their schemas, the answer as the daemon wrote
//! it, the inputs refused, and each way the daemon can fail turned into a tool error the agent
//! reads. A client that names roots has them given to the daemon with each call, asked once a
//! session and again after it says they changed; one that never answers is waited for once.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cortex_core::ipc::{FactsAnswer, Note, NoteOutcome, RequestBody, ResponseBody};
use cortex_core::{FactKind, FactSource};
use cortex_mcp::{Daemon, Server};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

/// What a test's result is.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A daemon that answers every request with one answer and keeps the requests it was sent, and
/// the roots given with each.
struct Fake {
    /// The answer, or why there is none.
    answer: Result<ResponseBody, String>,
    /// Every request, in order.
    asked: Mutex<Vec<RequestBody>>,
    /// The roots given with each request, in order.
    roots: Mutex<Vec<Vec<PathBuf>>>,
}

impl Fake {
    /// A fake answering `answer`, asked nothing yet.
    fn answering(answer: Result<ResponseBody, String>) -> Self {
        Self {
            answer,
            asked: Mutex::new(Vec::new()),
            roots: Mutex::new(Vec::new()),
        }
    }
}

impl Daemon for Fake {
    fn ask(&self, body: RequestBody, roots: &[PathBuf]) -> Result<ResponseBody, String> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(body);
        }
        if let Ok(mut given) = self.roots.lock() {
            given.push(roots.to_vec());
        }
        self.answer.clone()
    }
}

/// A client that offers roots, names these when it answers, and counts the times it was asked.
struct WithRoots {
    /// The roots' URIs.
    uris: Vec<&'static str>,
    /// Whether it never answers.
    is_silent: bool,
    /// How many times it was asked.
    asked: Arc<AtomicUsize>,
}

impl WithRoots {
    /// A client naming `uris`, or never answering when `is_silent`.
    fn new(uris: Vec<&'static str>, is_silent: bool) -> Self {
        Self {
            uris,
            is_silent,
            asked: Arc::default(),
        }
    }
}

#[expect(deprecated, reason = "roots are what this test is about")]
impl rmcp::ClientHandler for WithRoots {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        rmcp::model::ClientInfo::new(
            rmcp::model::ClientCapabilities::builder()
                .enable_roots()
                .enable_roots_list_changed()
                .build(),
            rmcp::model::Implementation::new("test", "0"),
        )
        .with_protocol_version(rmcp::model::ProtocolVersion::V_2025_06_18)
    }

    async fn list_roots(
        &self,
        _: rmcp::service::RequestContext<rmcp::RoleClient>,
    ) -> Result<rmcp::model::ListRootsResult, rmcp::ErrorData> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.is_silent {
            std::future::pending::<()>().await;
        }
        Ok(rmcp::model::ListRootsResult::new(
            self.uris
                .iter()
                .map(|uri| rmcp::model::Root::new(*uri))
                .collect(),
        ))
    }
}

/// Runs `calls` of `cortex_where` against a server whose daemon answers `answer`, and returns,
/// for each call, the result as JSON (or the protocol error's code), with the requests the daemon
/// was sent.
async fn session(
    answer: Result<ResponseBody, String>,
    calls: Vec<Value>,
) -> TestResult<(Vec<Value>, Vec<RequestBody>)> {
    session_of("cortex_where", answer, calls).await
}

/// Runs `calls` of the tool `tool` against a server whose daemon answers `answer`, as [`session`]
/// does.
async fn session_of(
    tool: &'static str,
    answer: Result<ResponseBody, String>,
    calls: Vec<Value>,
) -> TestResult<(Vec<Value>, Vec<RequestBody>)> {
    let fake = Arc::new(Fake::answering(answer));
    let (server_side, client_side) = tokio::io::duplex(64 * 1024);
    let server = Server::new(fake.clone()).serve(server_side);
    let client = ().serve(client_side);
    let (server, client) = tokio::join!(server, client);
    let (server, client) = (server?, client?);
    let mut results = Vec::new();
    for arguments in calls {
        let Value::Object(arguments) = arguments else {
            return Err("arguments are no object".into());
        };
        let params = CallToolRequestParams::new(tool).with_arguments(arguments);
        results.push(match client.call_tool(params).await {
            Ok(result) => serde_json::to_value(result)?,
            Err(rmcp::ServiceError::McpError(error)) => json!({"code": error.code.0}),
            Err(other) => return Err(other.into()),
        });
    }
    client.cancel().await?;
    server.cancel().await?;
    let asked = fake.asked.lock().map_err(|_| "poisoned")?.clone();
    Ok((results, asked))
}

/// A tool result holding `text`, a success or an error as `is_error` says.
fn result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

/// The input schema of the tool `name` among `tools`.
fn schema_of_tool(tools: &[rmcp::model::Tool], name: &str) -> TestResult<Value> {
    let tool = tools
        .iter()
        .find(|tool| tool.name == name)
        .ok_or("no such tool")?;
    Ok(serde_json::to_value(&tool.input_schema)?)
}

/// The server offers exactly three tools: `cortex_note`, taking a rule, its anchors and an
/// optional kind; `cortex_where`, taking a query and an optional limit; and `cortex_why`, taking a
/// target.
#[tokio::test]
async fn the_tools_are_cortex_note_where_and_why() -> TestResult {
    let fake = Arc::new(Fake::answering(Err("unused".to_owned())));
    let (server_side, client_side) = tokio::io::duplex(64 * 1024);
    let (server, client) =
        tokio::join!(Server::new(fake).serve(server_side), ().serve(client_side));
    let (server, client) = (server?, client?);
    let tools = client.list_all_tools().await?;
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(names, ["cortex_note", "cortex_where", "cortex_why"]);
    let why = schema_of_tool(&tools, "cortex_why")?;
    assert_eq!(why.pointer("/required"), Some(&json!(["target"])));
    assert_eq!(
        why.pointer("/properties/target/type"),
        Some(&json!("string"))
    );
    let note = schema_of_tool(&tools, "cortex_note")?;
    assert_eq!(note.pointer("/required"), Some(&json!(["text", "anchors"])));
    assert_eq!(
        note.pointer("/properties/anchors/type"),
        Some(&json!("array"))
    );
    let schema = schema_of_tool(&tools, "cortex_where")?;
    assert_eq!(schema.pointer("/required"), Some(&json!(["query"])));
    assert_eq!(
        schema.pointer("/properties/query/type"),
        Some(&json!("string"))
    );
    assert_eq!(schema.pointer("/properties/limit/minimum"), Some(&json!(1)));
    assert_eq!(schema.pointer("/properties/limit/maximum"), Some(&json!(5)));
    let info = serde_json::to_value(client.peer_info())?;
    assert_eq!(info.pointer("/serverInfo/name"), Some(&json!("cortex")));
    client.cancel().await?;
    server.cancel().await?;
    Ok(())
}

/// A question goes to the daemon as `cortex where` sends it, the limit 3 when left out, and its
/// answer comes back as the daemon wrote it; a limit outside 1 to 5 and a blank question are tool
/// errors the model can correct, and never reach the daemon.
#[tokio::test]
async fn a_question_is_answered_as_the_daemon_wrote_it() -> TestResult {
    let answer = "calc/ops.py:12-13 divide (callers 2, confidence 0.8)\n    def divide(a, b):\n";
    let (results, asked) = session(
        Ok(ResponseBody::Reply {
            text: Some(answer.to_owned()),
            to_user: None,
        }),
        vec![
            json!({"query": "where is divide"}),
            json!({"query": "divide", "limit": 5}),
            json!({"query": "divide", "limit": 6}),
            json!({"query": "divide", "limit": 0}),
            json!({"query": "divide", "limit": 300}),
            json!({"query": "divide", "limit": -1}),
            json!({"query": "  "}),
        ],
    )
    .await?;
    let bad_limit = result("Cortex: the limit is a number from 1 to 5", true);
    assert_eq!(
        results,
        [
            result(answer, false),
            result(answer, false),
            bad_limit.clone(),
            bad_limit.clone(),
            bad_limit.clone(),
            bad_limit,
            result("Cortex: the query is empty: ask in a sentence", true),
        ]
    );
    assert_eq!(
        asked,
        [
            RequestBody::Where {
                query: "where is divide".to_owned(),
                limit: 3,
                worktree_root: None
            },
            RequestBody::Where {
                query: "divide".to_owned(),
                limit: 5,
                worktree_root: None
            },
        ]
    );
    Ok(())
}

/// Each answer without code is said in words: nothing found, the daemon's error, no daemon; the
/// last two as tool errors, which the agent reads, while the session goes on.
#[tokio::test]
async fn an_answer_without_code_is_said_in_words() -> TestResult {
    let question = || vec![json!({"query": "divide"})];
    let (nothing, _) = session(
        Ok(ResponseBody::Reply {
            text: None,
            to_user: None,
        }),
        question(),
    )
    .await?;
    assert_eq!(
        nothing,
        [result(
            "Nothing found: no file the engine can show for this question, or the daemon has \
             not built its graph yet (`cortex status` says which).",
            false
        )]
    );
    let (error, _) = session(
        Ok(ResponseBody::Error {
            message: "the index is not ready".to_owned(),
        }),
        question(),
    )
    .await?;
    assert_eq!(
        error,
        [result(
            "Cortex: the daemon could not answer: the index is not ready",
            true
        )]
    );
    let (down, _) = session(
        Err("no daemon answered within 20 s".to_owned()),
        vec![json!({"query": "divide"}), json!({"query": "add"})],
    )
    .await?;
    let failed = result("Cortex: no daemon answered within 20 s", true);
    assert_eq!(down, [failed.clone(), failed]);
    Ok(())
}

/// An answer past 1,200 tokens is cut after a whole line, with the note saying so.
#[tokio::test]
async fn a_long_answer_is_cut() -> TestResult {
    let line = "x".repeat(99);
    let long: String = (0..300).map(|_| format!("{line}\n")).collect();
    let (results, _) = session(
        Ok(ResponseBody::Reply {
            text: Some(long),
            to_user: None,
        }),
        vec![json!({"query": "divide"})],
    )
    .await?;
    let text = results
        .first()
        .and_then(|result| result.pointer("/content/0/text"))
        .and_then(Value::as_str)
        .ok_or("no text")?;
    assert_eq!(text.lines().count(), 48);
    assert!(
        text.ends_with("[cortex] cut at 1200 tokens: 253 more lines; ask with a smaller limit\n")
    );
    Ok(())
}

/// A rule goes to the daemon as an agent's note, a convention when no kind is given, its anchors
/// as given; what became of it comes back in words: a new candidate, a fact that says it already,
/// one that is now active. A kind Cortex does not know, an empty rule and no anchor are tool
/// errors the model can correct, and never reach the daemon.
#[tokio::test]
async fn a_rule_is_noted_as_an_agent_s_candidate() -> TestResult {
    let calls = || {
        vec![
            json!({"text": "Money is kept in cents.", "anchors": ["calc/money.py:Money"]}),
            json!({"text": "Tax rounds up.", "anchors": ["calc/tax.py"], "kind": "pitfall"}),
            json!({"text": "Tax rounds up.", "anchors": ["calc/tax.py"], "kind": "rule"}),
            json!({"text": " ", "anchors": ["calc/tax.py"]}),
            json!({"text": "Tax rounds up.", "anchors": []}),
        ]
    };
    let noted = |outcome| Ok(ResponseBody::Facts(FactsAnswer::Noted { id: 7, outcome }));
    let (results, asked) = session_of("cortex_note", noted(NoteOutcome::Added), calls()).await?;
    let added = result(
        "Noted as fact 7, a candidate: given to agents once the user confirms it or it is noted \
         again in another task.",
        false,
    );
    assert_eq!(
        results,
        [
            added.clone(),
            added,
            result(
                "Cortex: the kind is convention, decision, invariant, pitfall or ownership",
                true
            ),
            result("Cortex: the rule is empty: say it in a sentence", true),
            result(
                "Cortex: name what the rule is about: a file, `path:symbol`, or `.` for the \
                 repository",
                true
            ),
        ]
    );
    assert_eq!(
        asked,
        [
            RequestBody::Note(Note {
                text: "Money is kept in cents.".to_owned(),
                kind: FactKind::Convention,
                source: FactSource::Agent,
                at: vec!["calc/money.py:Money".to_owned()],
            }),
            RequestBody::Note(Note {
                text: "Tax rounds up.".to_owned(),
                kind: FactKind::Pitfall,
                source: FactSource::Agent,
                at: vec!["calc/tax.py".to_owned()],
            }),
        ]
    );
    let one = || vec![json!({"text": "Tax rounds up.", "anchors": ["calc/tax.py"]})];
    let (merged, _) = session_of("cortex_note", noted(NoteOutcome::Merged), one()).await?;
    assert_eq!(merged, [result("Fact 7 says this already.", false)]);
    let (activated, _) = session_of("cortex_note", noted(NoteOutcome::Activated), one()).await?;
    assert_eq!(
        activated,
        [result(
            "Fact 7 says this already, and is now active.",
            false
        )]
    );
    let refused = Ok(ResponseBody::Error {
        message: "calc/tax.py is not a file Cortex indexes".to_owned(),
    });
    let (failed, _) = session_of("cortex_note", refused, one()).await?;
    assert_eq!(
        failed,
        [result(
            "Cortex: the rule was not noted: calc/tax.py is not a file Cortex indexes",
            true
        )]
    );
    Ok(())
}

/// A target goes to the daemon as it is given; the daemon's answer comes back as it wrote it, and
/// its refusal as a tool error the model can correct, in the daemon's own words. An empty target
/// is refused before the daemon is asked.
#[tokio::test]
async fn a_why_is_answered_as_the_daemon_wrote_it() -> TestResult {
    let answer = "[cortex] Why · calc/ops.py\nLinked by code:\n  - calc/tax.py (0.90)\n";
    let reply = Ok(ResponseBody::Reply {
        text: Some(answer.to_owned()),
        to_user: None,
    });
    let calls = vec![json!({"target": "calc/ops.py"}), json!({"target": " "})];
    let (results, asked) = session_of("cortex_why", reply, calls).await?;
    assert_eq!(
        results,
        [
            result(answer, false),
            result(
                "Cortex: name a file, `path:line`, `path:symbol` or a symbol's name",
                true
            ),
        ]
    );
    assert_eq!(
        asked,
        [RequestBody::Why {
            target: "calc/ops.py".to_owned(),
            worktree_root: None
        }]
    );
    let refused = Ok(ResponseBody::Error {
        message: "refund names no file or symbol Cortex indexes".to_owned(),
    });
    let one = vec![json!({"target": "refund"})];
    let (failed, _) = session_of("cortex_why", refused, one).await?;
    assert_eq!(
        failed,
        [result(
            "Cortex: refund names no file or symbol Cortex indexes",
            true
        )]
    );
    Ok(())
}

/// How long a call may take in these tests: a client that never names its roots delays one call
/// by the server's 2 s at most.
const CALL_WITHIN: Duration = Duration::from_secs(30);

/// The roots each call gave the daemon, in a session of `calls` `cortex_where` calls made by
/// `client`, each within [`CALL_WITHIN`]; after the first, the client says its roots changed when
/// `is_changed`.
async fn roots_given(
    client: impl rmcp::ClientHandler,
    calls: usize,
    is_changed: bool,
) -> TestResult<Vec<Vec<PathBuf>>> {
    let fake = Arc::new(Fake::answering(Ok(ResponseBody::Reply {
        text: Some("found".to_owned()),
        to_user: None,
    })));
    let (server_side, client_side) = tokio::io::duplex(64 * 1024);
    let server = Server::new(fake.clone()).serve(server_side);
    let client = client.serve(client_side);
    let (server, client) = tokio::join!(server, client);
    let (server, client) = (server?, client?);
    for call in 0..calls {
        if call == 1 && is_changed {
            client.notify_roots_list_changed().await?;
        }
        let Value::Object(arguments) = json!({"query": "where are orders summed"}) else {
            return Err("arguments are no object".into());
        };
        let params = CallToolRequestParams::new("cortex_where").with_arguments(arguments);
        tokio::time::timeout(CALL_WITHIN, client.call_tool(params)).await??;
    }
    client.cancel().await?;
    server.cancel().await?;
    let given = fake.roots.lock().map_err(|_| "poisoned")?.clone();
    Ok(given)
}

/// This system's two roots typed by hand, as URIs and as the folders they name.
fn two_roots() -> (&'static str, &'static str, Vec<PathBuf>) {
    if cfg!(windows) {
        (
            "file:///C:/work/one",
            "file:///D:/two%20words",
            vec![PathBuf::from("C:/work/one"), PathBuf::from("D:/two words")],
        )
    } else {
        (
            "file:///work/one",
            "file:///srv/two%20words",
            vec![PathBuf::from("/work/one"), PathBuf::from("/srv/two words")],
        )
    }
}

/// A client that names roots has its local folders given to the daemon with each call, in its
/// order, a root that is no local folder left out, asked once for two calls; a client that offers
/// no roots gives none.
#[tokio::test]
async fn the_client_s_roots_are_given_with_each_call() -> TestResult {
    let (first, second, expected) = two_roots();
    let client = WithRoots::new(vec![first, "https://example.test/three", second], false);
    let asked = Arc::clone(&client.asked);
    assert_eq!(
        roots_given(client, 2, false).await?,
        vec![expected.clone(), expected]
    );
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(
        roots_given((), 1, false).await?,
        vec![Vec::<PathBuf>::new()]
    );
    Ok(())
}

/// Once the client says its roots changed, they are asked again at a later call.
#[tokio::test]
async fn changed_roots_are_asked_again() -> TestResult {
    let (first, second, _) = two_roots();
    let client = WithRoots::new(vec![first, second], false);
    let asked = Arc::clone(&client.asked);
    // The notice and the next call may reach the server in either order: a few calls give it time.
    roots_given(client, 4, true).await?;
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    Ok(())
}

/// A client that offers roots and never names them: each call still answers, with no roots, and
/// the client is asked once in the session.
#[tokio::test]
async fn a_client_that_never_names_its_roots_is_asked_once() -> TestResult {
    let (first, _, _) = two_roots();
    let client = WithRoots::new(vec![first], true);
    let asked = Arc::clone(&client.asked);
    assert_eq!(
        roots_given(client, 3, false).await?,
        vec![Vec::<PathBuf>::new(); 3]
    );
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    Ok(())
}
