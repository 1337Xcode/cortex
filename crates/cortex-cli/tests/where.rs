//! `cortex where` and `cortex why` through the binary: what the engine finds for a question, as
//! the daemon writes it; what it says when there is nothing to show, before the daemon's graph and
//! after; that a question the daemon could not answer fails the command; and why code is where it
//! is, or the daemon's refusal of a target that names nothing.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::fake_daemon::{FakeDaemon, Script, kinds};
use common::git::{commit, git};
use common::{Daemon, Printed, TestResult, graph_of, isolate, printed, wait_for};
use cortex_core::{RepoId, canonical_root};
use cortex_store::{Writer, WriterOptions};

/// `src/orders/service.py`, its function on lines 1 to 3, its body indented with tabs, which the
/// command prints as they are.
const SERVICE: &str = "def create_order(cart):\n\ttotal = sum(cart)\n\treturn total\n";

/// `src/orders/model.py`, its class on lines 1 to 3.
const MODEL: &str = "class Order:\n    def __init__(self):\n        self.lines = []\n";

/// A question naming `create_order`, with "orders" in both files' paths.
const QUESTION: &str = "fix create_order in the orders service";

/// A repository whose two commits change the service and the model together, pinned to medium
/// mode; its home folder; and its daemon, once the graph holds the co-change links. When
/// `self_check` is given, it is stored as the self-check's record, as it is, in a store made before
/// the daemon starts.
fn repository(
    self_check: Option<&str>,
) -> TestResult<(tempfile::TempDir, tempfile::TempDir, Daemon)> {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    git(root, home.path(), &["init", "-q"])?;
    std::fs::create_dir_all(root.join("src/orders"))?;
    for (message, suffix) in [("one", ""), ("two", "\n")] {
        std::fs::write(
            root.join("src/orders/service.py"),
            format!("{SERVICE}{suffix}"),
        )?;
        std::fs::write(root.join("src/orders/model.py"), format!("{MODEL}{suffix}"))?;
        commit(
            root,
            home.path(),
            message,
            &["src/orders/service.py", "src/orders/model.py"],
        )?;
    }
    std::fs::create_dir_all(root.join(".cortex"))?;
    std::fs::write(root.join(".cortex/config.toml"), "mode = \"medium\"\n")?;
    if let Some(record) = self_check {
        drop(Writer::open(&WriterOptions {
            folder: &root.join(".cortex"),
            repo_id: &RepoId::of(&canonical_root(root)?),
            mmap_bytes: 0,
            now_ms: 1,
        })?);
        let store = rusqlite::Connection::open(root.join(".cortex/cortex.db"))?;
        store.execute(
            "INSERT INTO meta (key, value) VALUES ('self_check', ?1)",
            [record],
        )?;
    }
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some_and(|graph| graph.has_cochange)))?;
    Ok((repo, home, daemon))
}

/// Runs `cortex where` with `args` for the repository at `root`, `home` being the home folder.
fn cortex_where(root: &Path, home: &Path, args: &[&str]) -> TestResult<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.arg("where").args(args).arg("--root").arg(root);
    isolate(&mut command, home);
    Ok(command.output()?)
}

/// A question naming `create_order` gets the service, its lines 1 to 3 as the file holds them, under
/// the line saying what the evidence was and what the gate would do; at a limit of one file, the
/// model is not named.
#[test]
fn a_question_gets_what_the_engine_finds() -> TestResult {
    let (repo, home, _daemon) = repository(None)?;
    let output = cortex_where(repo.path(), home.path(), &[QUESTION, "--limit", "1"])?;
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let printed = String::from_utf8(output.stdout)?;
    let mut lines = printed.lines();
    let heading = lines.next().ok_or("no heading")?;
    assert!(heading.starts_with("[cortex] Where · "), "{printed}");
    assert!(heading.contains(" · the gate would "), "{printed}");
    assert!(printed.contains("src/orders/service.py:1-3"), "{printed}");
    assert!(!printed.contains("src/orders/model.py"), "{printed}");
    for code in SERVICE.lines() {
        assert!(
            printed.lines().any(|line| line.ends_with(code)),
            "{code}: {printed}"
        );
    }
    Ok(())
}

/// A question whose words name nothing the repository holds is answered with nothing, and the
/// command says that the graph is built and nothing was found.
#[test]
fn a_question_that_finds_nothing_says_so() -> TestResult {
    let (repo, home, _daemon) = repository(None)?;
    let output = cortex_where(repo.path(), home.path(), &["zqxv wvrt"])?;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "nothing found: the question points at no file the engine can show\n"
    );
    Ok(())
}

/// Before the daemon's first graph a question gets no answer, and the command says the graph is
/// not built yet, having asked the daemon its status: the fake daemon, whose first index is not
/// done, answers the question with nothing and its status with no graph.
#[test]
fn before_the_graph_the_command_says_so() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::create_dir(repo.path().join(".cortex"))?;
    let fake = FakeDaemon::start(repo.path(), home.path(), Script::Indexing(2))?;
    let output = cortex_where(repo.path(), home.path(), &[QUESTION])?;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "nothing yet: the daemon has not built its graph; `cortex status` says when it has\n"
    );
    let requests = fake.requests()?;
    assert_eq!(kinds(&requests), ["hello", "where", "hello", "status"]);
    Ok(())
}

/// A question the daemon could not answer fails the command with the daemon's reason: here the
/// self-check's stored record is not one, which every retrieval reads.
#[test]
fn a_question_the_daemon_could_not_answer_fails() -> TestResult {
    let (repo, home, _daemon) = repository(Some("{not a record"))?;
    let output = cortex_where(repo.path(), home.path(), &[QUESTION])?;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let said = String::from_utf8(output.stderr)?;
    assert!(
        said.starts_with("cortex: error: the daemon could not answer: "),
        "{said}"
    );
    Ok(())
}

/// A limit outside 1 to 5 is refused before anything is asked.
#[test]
fn a_limit_outside_one_to_five_is_refused() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    for limit in ["0", "6"] {
        let output = cortex_where(repo.path(), home.path(), &[QUESTION, "--limit", limit])?;
        assert_eq!(output.status.code(), Some(2), "{limit}: {output:?}");
    }
    Ok(())
}

/// Runs `cortex` with `args` from the folder `cwd`, `home` being the home folder.
fn cortex_in(cwd: &Path, home: &Path, args: &[&str]) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).current_dir(cwd);
    isolate(&mut command, home);
    printed(&mut command, b"")
}

/// `cortex why`, worked by hand from [`repository`]: both commits changed the service and the
/// model together, so each changed with the other in 2 of its 2 commits, a confidence of 1.00; no
/// reference ties them and no task has run, so nothing else links them. A rule noted on
/// `create_order` is a fact about it. Asked of `service.py:2` from the service's own folder, the
/// target is `create_order` (lines 1 to 3), and the answer names the model and the rule. A name
/// nothing has is refused: the command fails with the daemon's words and prints nothing else.
#[test]
fn why_names_what_code_is_linked_to_and_the_rules_about_it() -> TestResult {
    let (repo, home, _daemon) = repository(None)?;
    let (root, home) = (repo.path(), home.path());
    let rule = "Orders are summed in cents.";
    let at = "src/orders/service.py:create_order";
    let noted = cortex_in(root, home, &["note", rule, "--at", at])?;
    assert_eq!(noted.stdout, "fact 1 added\n", "{}", noted.stderr);
    let root_text = root.to_str().ok_or("a root that is not text")?;
    let why = ["why", "service.py:2", "--root", root_text];
    let answer = cortex_in(&root.join("src/orders"), home, &why)?;
    assert!(answer.is_success, "{}", answer.stderr);
    assert_eq!(answer.stderr, "");
    assert_eq!(
        answer.stdout,
        format!(
            "[cortex] Why · {at} · what is known of its file, its own facts first\n\
             Changed together in history:\n  - src/orders/model.py (1.00)\n\
             Project facts:\n  - {rule} (anchored to {at})\n"
        )
    );
    let refused = cortex_in(root, home, &["why", "refund"])?;
    assert!(!refused.is_success, "{}", refused.stdout);
    assert_eq!(refused.stdout, "");
    assert_eq!(
        refused.stderr,
        "cortex: error: refund names no file or symbol Cortex indexes\n"
    );
    Ok(())
}

/// `cortex mcp` through the binary, spoken to in JSON-RPC lines as an agent does: it starts the
/// session as `cortex`, offers `cortex_note` and `cortex_where`, and answers the question with the
/// text `cortex where` prints for it; a rule noted on `create_order`, named from the repository's
/// root, becomes fact 1, a candidate, as `cortex facts list` shows; when the agent closes its
/// input, the server exits 0 having printed nothing but the protocol, and nothing on stderr.
#[test]
fn the_mcp_tool_answers_as_cortex_where_prints() -> TestResult {
    let (repo, home, _daemon) = repository(None)?;
    let printed = cortex_where(repo.path(), home.path(), &[QUESTION])?;
    assert!(printed.status.success(), "{printed:?}");
    let expected = String::from_utf8(printed.stdout)?;
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "cortex_where", "arguments": {"query": QUESTION}}
    });
    let rule = "Orders are created from a whole cart.";
    let note = serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "cortex_note", "arguments": {
            "text": rule, "anchors": ["src/orders/service.py:create_order"]
        }}
    });
    let lines = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#.to_owned(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_owned(),
        call.to_string(),
        note.to_string(),
    ];
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.arg("mcp").arg("--root").arg(repo.path());
    isolate(&mut command, home.path());
    let output = talk(command, &lines)?;
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let replies: Vec<serde_json::Value> = String::from_utf8(output.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let reply = |id: u64| {
        replies
            .iter()
            .find(|reply| reply["id"] == id)
            .ok_or(format!("no reply {id}: {replies:?}"))
    };
    assert_eq!(reply(1)?["result"]["serverInfo"]["name"], "cortex");
    let mut tools = reply(2)?["result"]["tools"]
        .as_array()
        .ok_or("no tools")?
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    tools.sort();
    assert_eq!(tools, ["cortex_note", "cortex_where"]);
    let answer = &reply(3)?["result"];
    assert_eq!(answer["isError"], false);
    assert_eq!(
        answer["content"][0]["text"].as_str(),
        Some(expected.as_str())
    );
    let noted = &reply(4)?["result"];
    assert_eq!(noted["isError"], false, "{noted}");
    assert_eq!(
        noted["content"][0]["text"].as_str(),
        Some(
            "Noted as fact 1, a candidate: given to agents once the user confirms it or it is \
             noted again in another task."
        )
    );
    assert_eq!(replies.len(), 4, "{replies:?}");
    let mut list = Command::new(env!("CARGO_BIN_EXE_cortex"));
    list.args(["facts", "list", "--root"]).arg(repo.path());
    isolate(&mut list, home.path());
    let listed = list.output()?;
    assert_eq!(
        String::from_utf8(listed.stdout)?,
        format!("   1  candidate  convention  {rule}  (at src/orders/service.py:create_order)\n")
    );
    Ok(())
}

/// Runs `command`, writes `lines` to its input one by one and closes it, and returns what it
/// printed once it exits, within 60 seconds.
fn talk(mut command: Command, lines: &[String]) -> TestResult<Output> {
    use std::io::Write as _;
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("no stdin")?;
    for line in lines {
        writeln!(input, "{line}")?;
    }
    drop(input);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(child.wait_with_output()));
    Ok(receiver.recv_timeout(std::time::Duration::from_secs(60))??)
}
