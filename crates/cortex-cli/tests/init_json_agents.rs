//! `cortex init --agents` for the agents whose hooks live in a JSON hooks file of their own shape
//! (Cursor, GitHub Copilot, Antigravity, Kiro, TRAE, Crush, OpenHands), each on a temporary git
//! repository with an empty home folder of the test's own, through the real binary: init writes
//! exactly the file typed here by hand from each agent's documentation and its adapter's table,
//! keeping the hooks the user already had; init again changes nothing; doctor finds the hooks
//! current, and, with one taken out by hand, says so with the init that puts it back; uninstall
//! leaves every file and folder as it was before init, those init made gone and those that were
//! there kept, and doctor then checks nothing of the agent's. Init refuses to write one scope of
//! Copilot's or Kiro's while the other holds Cortex's hooks, since both would run; and a switch
//! the user turned Cortex's hooks off with stays off, which doctor says.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::init::{git, run};
use common::{Printed, StopsDaemon, TestResult};
use cortex_core::canonical_root;
use serde_json::{Value, json};

/// Where a test's agent file is.
#[derive(Clone, Copy)]
enum Place {
    /// Under the home folder, written at the user's scope.
    Home(&'static str),
    /// Under the repository's root, written at the project's scope; git is told to leave it out.
    Repository(&'static str),
}

/// What an agent's file holds after init, given the command line of Cortex's hook at an event.
type Expected = fn(&dyn Fn(&str) -> String) -> Value;

/// One agent's run: its name and title, its file, what the file held before init (none: no file),
/// and what it holds after, given the command line of Cortex's hook at an event.
struct Case {
    /// The agent's name on Cortex's command line.
    agent: &'static str,
    /// The agent's name as init and doctor say it.
    title: &'static str,
    /// The file init writes.
    place: Place,
    /// The file's text before init, the user's own hooks in it.
    before: Option<&'static str>,
    /// The event whose hook is taken out by hand: the start of a session, as the agent names it.
    taken_out: &'static str,
    /// Folders, under the home folder or the repository as the file is, that are there, empty,
    /// before init.
    folders: &'static [&'static str],
    /// The file after init.
    expected: Expected,
}

/// Cursor: `~/.cursor/hooks.json`, a version and handlers of their own, the user's `stop` handler
/// first; the 1,500 ms start is 2 s, every other hook 1 s.
#[test]
fn init_sets_up_cursor() -> TestResult {
    checked(&Case {
        agent: "cursor",
        title: "Cursor",
        place: Place::Home(".cursor/hooks.json"),
        before: Some(r#"{"version": 1, "hooks": {"stop": [{"command": "notify.sh"}]}}"#),
        taken_out: "sessionStart",
        folders: &[],
        expected: |line| {
            let tools = "^(Read|Write|Grep|Shell)$";
            json!({"version": 1, "hooks": {
                "stop": [
                    {"command": "notify.sh"},
                    {"type": "command", "command": line("stop"), "timeout": 1}
                ],
                "sessionStart": [{"type": "command", "command": line("sessionStart"), "timeout": 2}],
                "beforeSubmitPrompt": [
                    {"type": "command", "command": line("beforeSubmitPrompt"), "timeout": 1}
                ],
                "postToolUse": [{"type": "command", "matcher": tools,
                    "command": line("postToolUse"), "timeout": 1}],
                "postToolUseFailure": [{"type": "command", "matcher": tools,
                    "command": line("postToolUseFailure"), "timeout": 1}],
                "preCompact": [{"type": "command", "command": line("preCompact"), "timeout": 1}],
                "subagentStop": [{"type": "command", "command": line("subagentStop"), "timeout": 1}],
                "sessionEnd": [{"type": "command", "command": line("sessionEnd"), "timeout": 1}]
            }})
        },
    })
}

/// Copilot's hooks under the key of this system's shell, with `timeoutSec`.
fn copilot_hooks(line: &dyn Fn(&str) -> String) -> serde_json::Map<String, Value> {
    let shell = if cfg!(windows) { "powershell" } else { "bash" };
    let tools = "view|create|edit|glob|grep|bash|powershell";
    let mut hooks = serde_json::Map::new();
    for (event, matcher, seconds) in [
        ("SessionStart", None, 2),
        ("UserPromptSubmit", None, 1),
        ("PostToolUse", Some(tools), 1),
        ("PostToolUseFailure", Some(tools), 1),
        ("PreCompact", None, 1),
        ("SubagentStart", None, 1),
        ("SubagentStop", None, 1),
        ("Stop", None, 1),
        ("SessionEnd", None, 1),
    ] {
        let mut handler = json!({"type": "command", shell: line(event), "timeoutSec": seconds});
        if let (Some(matcher), Some(handler)) = (matcher, handler.as_object_mut()) {
            handler.insert("matcher".to_owned(), matcher.into());
        }
        hooks.insert(event.to_owned(), json!([handler]));
    }
    hooks
}

/// Copilot at the project's scope: `.github/hooks/cortex.json`, made with its folders, which
/// uninstall takes away again with the file.
#[test]
fn init_sets_up_copilot_in_the_repository() -> TestResult {
    checked(&Case {
        agent: "copilot",
        title: "GitHub Copilot",
        place: Place::Repository(".github/hooks/cortex.json"),
        before: None,
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| json!({"version": 1, "hooks": copilot_hooks(line)}),
    })
}

/// Copilot at the user's scope: `~/.copilot/hooks/cortex.json`, which already held the user's
/// `Stop` hook, kept first.
#[test]
fn init_sets_up_copilot_for_the_user() -> TestResult {
    checked(&Case {
        agent: "copilot",
        title: "GitHub Copilot",
        place: Place::Home(".copilot/hooks/cortex.json"),
        before: Some(
            r#"{"version": 1, "hooks": {"Stop": [{"type": "command", "bash": "notify.sh", "powershell": "notify.ps1"}]}}"#,
        ),
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| {
            let mut hooks = copilot_hooks(line);
            if let Some(Value::Array(stop)) = hooks.get_mut("Stop") {
                stop.insert(
                    0,
                    json!({"type": "command", "bash": "notify.sh", "powershell": "notify.ps1"}),
                );
            }
            json!({"version": 1, "hooks": hooks})
        },
    })
}

/// Antigravity: `~/.gemini/config/hooks.json`, Cortex's events under its own hook name beside the
/// user's `lint`; the tool's event in a matcher group, the others as handlers of their own.
#[test]
fn init_sets_up_antigravity() -> TestResult {
    checked(&Case {
        agent: "antigravity",
        title: "Antigravity",
        place: Place::Home(".gemini/config/hooks.json"),
        before: Some(
            r#"{"lint": {"PostToolUse": [{"matcher": "run_command", "hooks": [{"type": "command", "command": "lint.sh", "timeout": 10}]}]}}"#,
        ),
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| {
            json!({
                "lint": {"PostToolUse": [{"matcher": "run_command",
                    "hooks": [{"type": "command", "command": "lint.sh", "timeout": 10}]}]},
                "cortex": {
                    "SessionStart": [
                        {"type": "command", "command": line("SessionStart"), "timeout": 2}
                    ],
                    "PreInvocation": [
                        {"type": "command", "command": line("PreInvocation"), "timeout": 1}
                    ],
                    "PostToolUse": [{
                        "matcher": "^(view_file|write_to_file|replace_file_content|multi_replace_file_content|grep_search|find_by_name|run_command)$",
                        "hooks": [{"type": "command", "command": line("PostToolUse"), "timeout": 1}]
                    }],
                    "Stop": [{"type": "command", "command": line("Stop"), "timeout": 1}]
                }
            })
        },
    })
}

/// Kiro's named hooks, each with its trigger, its matcher, its action and its timeout.
fn kiro_hooks(line: &dyn Fn(&str) -> String) -> Vec<Value> {
    let tools = "^(read|read_file|fs_write|fs_append|str_replace|grep_search|file_search|execute_bash|execute_pwsh)$";
    [
        ("SessionStart", None, 2),
        ("UserPromptSubmit", None, 1),
        ("PostToolUse", Some(tools), 1),
        ("Stop", None, 1),
        ("SessionEnd", None, 1),
    ]
    .into_iter()
    .map(|(event, matcher, seconds)| {
        let mut hook = json!({"name": format!("cortex {event}"), "trigger": event,
            "action": {"type": "command", "command": line(event)}, "timeout": seconds});
        if let (Some(matcher), Some(hook)) = (matcher, hook.as_object_mut()) {
            hook.insert("matcher".to_owned(), matcher.into());
        }
        hook
    })
    .collect()
}

/// Kiro at the project's scope: `.kiro/hooks/cortex.json`, made with its folders.
#[test]
fn init_sets_up_kiro_in_the_repository() -> TestResult {
    checked(&Case {
        agent: "kiro",
        title: "Kiro",
        place: Place::Repository(".kiro/hooks/cortex.json"),
        before: None,
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| json!({"version": "v1", "hooks": kiro_hooks(line)}),
    })
}

/// Kiro at the user's scope: `~/.kiro/hooks/cortex.json`, the user's hook first.
#[test]
fn init_sets_up_kiro_for_the_user() -> TestResult {
    checked(&Case {
        agent: "kiro",
        title: "Kiro",
        place: Place::Home(".kiro/hooks/cortex.json"),
        before: Some(
            r#"{"version": "v1", "hooks": [{"name": "fmt", "trigger": "PostFileSave", "matcher": "\\.rs$", "action": {"type": "command", "command": "cargo fmt"}}]}"#,
        ),
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| {
            let mut hooks = vec![json!({"name": "fmt", "trigger": "PostFileSave",
                "matcher": "\\.rs$", "action": {"type": "command", "command": "cargo fmt"}})];
            hooks.extend(kiro_hooks(line));
            json!({"version": "v1", "hooks": hooks})
        },
    })
}

/// TRAE: `~/.trae/hooks.json`, a version and Claude Code's nested shape, the user's `Stop` group
/// first.
#[test]
fn init_sets_up_trae() -> TestResult {
    checked(&Case {
        agent: "trae",
        title: "TRAE",
        place: Place::Home(".trae/hooks.json"),
        before: Some(
            r#"{"version": 1, "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "notify.sh"}]}]}}"#,
        ),
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| {
            let group = |event: &str, seconds: u64| json!({"hooks": [{"type": "command", "command": line(event), "timeout": seconds}]});
            let matched = |event: &str, matcher: &str| {
                json!({"matcher": matcher,
                    "hooks": [{"type": "command", "command": line(event), "timeout": 1}]})
            };
            json!({"version": 1, "hooks": {
                "Stop": [
                    {"hooks": [{"type": "command", "command": "notify.sh"}]},
                    group("Stop", 1)
                ],
                "SessionStart": [group("SessionStart", 2)],
                "UserPromptSubmit": [group("UserPromptSubmit", 1)],
                "PreToolUse": [matched("PreToolUse", "^Read$")],
                "PostToolUse": [matched("PostToolUse", "^(Read|Write|Edit|Glob|Grep|RunCommand)$")]
            }})
        },
    })
}

/// Crush: `~/.config/crush/crush.json`, handlers of their own with no `type`, beside the user's
/// options and after the user's `PreToolUse` handler.
#[test]
fn init_sets_up_crush() -> TestResult {
    checked(&Case {
        agent: "crush",
        title: "Crush",
        place: Place::Home(".config/crush/crush.json"),
        before: Some(
            r#"{"options": {"debug": false}, "hooks": {"PreToolUse": [{"matcher": "^bash$", "command": "./check.sh"}]}}"#,
        ),
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| {
            let tools = "^(view|write|edit|multiedit|glob|grep|bash)$";
            let plain =
                |event: &str, seconds: u64| json!([{"command": line(event), "timeout": seconds}]);
            let matched =
                |event: &str| json!({"matcher": tools, "command": line(event), "timeout": 1});
            json!({"options": {"debug": false}, "hooks": {
                "PreToolUse": [
                    {"matcher": "^bash$", "command": "./check.sh"},
                    {"matcher": "^view$", "command": line("PreToolUse"), "timeout": 1}
                ],
                "SessionStart": plain("SessionStart", 2),
                "TurnStart": plain("TurnStart", 1),
                "PostToolUse": [matched("PostToolUse")],
                "PostToolUseFailure": [matched("PostToolUseFailure")],
                "PreCompact": plain("PreCompact", 1),
                "TurnEnd": plain("TurnEnd", 1),
                "SessionEnd": plain("SessionEnd", 1)
            }})
        },
    })
}

/// OpenHands: `~/.openhands/hooks.json`, the events at the top of the file in Claude Code's nested
/// shape, after the user's `PreToolUse` group.
#[test]
fn init_sets_up_openhands() -> TestResult {
    checked(&Case {
        agent: "openhands",
        title: "OpenHands",
        place: Place::Home(".openhands/hooks.json"),
        before: Some(
            r#"{"PreToolUse": [{"matcher": "terminal", "hooks": [{"command": "block.sh", "timeout": 10}]}]}"#,
        ),
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| {
            let group = |event: &str, seconds: u64| json!([{"hooks": [{"type": "command", "command": line(event), "timeout": seconds}]}]);
            json!({
                "PreToolUse": [{"matcher": "terminal",
                    "hooks": [{"command": "block.sh", "timeout": 10}]}],
                "SessionStart": group("SessionStart", 2),
                "UserPromptSubmit": group("UserPromptSubmit", 1),
                "PostToolUse": [{"matcher": "^(terminal|file_editor)$",
                    "hooks": [{"type": "command", "command": line("PostToolUse"), "timeout": 1}]}],
                "Stop": group("Stop", 1),
                "SessionEnd": group("SessionEnd", 1)
            })
        },
    })
}

/// Runs `case` through init, init again, doctor, doctor with a hook taken out, and uninstall, as
/// the module says.
fn checked(case: &Case) -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let (file, scope) = match case.place {
        Place::Home(path) => (home.path().join(path), "user"),
        Place::Repository(path) => (repo.path().join(path), "project"),
    };
    let base = match case.place {
        Place::Home(_) => home.path(),
        Place::Repository(_) => repo.path(),
    };
    for folder in case.folders {
        std::fs::create_dir_all(base.join(folder))?;
    }
    if let Some(before) = case.before {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&file, before)?;
    }
    let before = snapshot(&[repo.path(), home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", case.agent, "--scope", scope],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}: {}", case.agent, done.stderr);
    let program = env!("CARGO_BIN_EXE_cortex");
    // The premise of the expected lines: the test binary's path needs no quoting.
    assert!(!program.contains(' '), "{program}");
    let program = program.replace('\\', "/");
    let agent = case.agent;
    let line = |event: &str| format!("{program} hook --agent {agent} --event {event}");
    let written = std::fs::read_to_string(&file)?;
    assert_eq!(
        serde_json::from_str::<Value>(&written)?,
        (case.expected)(&line),
        "{agent}"
    );
    let shown = written_to(&done, case.title)?;
    assert_eq!(
        std::fs::canonicalize(&shown)?,
        std::fs::canonicalize(&file)?,
        "{agent}"
    );
    if let Place::Repository(path) = case.place {
        let ignores = std::fs::read_to_string(repo.path().join(".gitignore"))?;
        assert!(
            ignores.lines().any(|line| line == path),
            "{agent}: {ignores}"
        );
    }
    let again = cortex(
        &["init", "--yes", "--agents", agent, "--scope", scope],
        repo.path(),
        home.path(),
    )?;
    assert!(again.is_success, "{agent}: {}", again.stderr);
    assert_eq!(std::fs::read_to_string(&file)?, written, "{agent}");
    let current = format!(
        "{}: the hooks in {} are current",
        case.title,
        shown.display()
    );
    assert!(
        again.stdout.lines().any(|line| line == current),
        "{}",
        again.stdout
    );
    let root = canonical_root(repo.path())?;
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(
        agents_lines(&doctor),
        [format!(
            "ok    agents: {}: Cortex's hooks are current in {}",
            case.title,
            shown.display()
        )],
        "{agent}"
    );
    let taken_out = without_event(&serde_json::from_str(&written)?, case.taken_out, agent)?;
    assert_ne!(taken_out, written, "{agent}");
    std::fs::write(&file, taken_out)?;
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    let scope_flag = match case.place {
        Place::Repository(_) => "",
        Place::Home(_) if matches!(agent, "copilot" | "kiro") => " --scope user",
        Place::Home(_) => "",
    };
    assert_eq!(
        agents_lines(&doctor),
        [
            format!(
                "FAIL  agents: {}: Cortex's hooks in {} are not what this build writes: one is \
                 missing or changed, or they run another binary",
                case.title,
                shown.display()
            ),
            format!(
                "      fix: run `cortex init --agents {agent}{scope_flag} --root {}`",
                root.display()
            ),
        ],
        "{agent}"
    );
    assert!(!doctor.is_success);
    // What init wrote goes back, so that uninstall finds the file as init left it.
    std::fs::write(&file, &written)?;
    let undone = cortex(&["uninstall", "--scope", scope], repo.path(), home.path())?;
    assert!(undone.is_success, "{agent}: {}", undone.stderr);
    assert_eq!(snapshot(&[repo.path(), home.path()])?, before, "{agent}");
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(agents_lines(&doctor), Vec::<String>::new(), "{agent}");
    Ok(())
}

/// Runs `cortex <args> --root <repo>` with `home` as the home folder and git on the PATH.
fn cortex(args: &[&str], repo: &Path, home: &Path) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).arg("--root").arg(repo);
    run(&mut command, home, &["git"])
}

/// The file init said it wrote `title`'s hooks to.
fn written_to(done: &Printed, title: &str) -> TestResult<PathBuf> {
    let prefix = format!("{title}: hooks written to ");
    let line = done
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .ok_or_else(|| done.stdout.clone())?;
    let path = line
        .split_once(", which serve")
        .map_or(line, |(path, _)| path);
    Ok(PathBuf::from(path))
}

/// The lines doctor's agents check printed, its fixes with them.
fn agents_lines(doctor: &Printed) -> Vec<String> {
    let lines: Vec<&str> = doctor.stdout.lines().collect();
    let mut found = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if line.contains("  agents: ") {
            found.push((*line).to_owned());
            if let Some(fix) = lines
                .get(at + 1)
                .filter(|next| next.starts_with("      fix: "))
            {
                found.push((*fix).to_owned());
            }
        }
    }
    found
}

/// `written` with every list item running Cortex's hook of `agent` at `event` taken out, as a
/// person would by hand, as pretty JSON.
fn without_event(written: &Value, event: &str, agent: &str) -> TestResult<String> {
    let marker = format!(" hook --agent {agent} --event {event}\"");
    let mut value = written.clone();
    take_out(&mut value, &marker)?;
    Ok(serde_json::to_string_pretty(&value)?)
}

/// Takes out of every list in `value` the items whose JSON holds `marker`.
fn take_out(value: &mut Value, marker: &str) -> TestResult {
    match value {
        Value::Array(items) => {
            let mut kept = Vec::new();
            for mut item in items.drain(..) {
                if serde_json::to_string(&item)?.contains(marker) {
                    continue;
                }
                take_out(&mut item, marker)?;
                kept.push(item);
            }
            *items = kept;
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                take_out(item, marker)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Every file and folder under `folders` but git's and Cortex's own (`.git`, `.cortex`), each with
/// its bytes (none for a folder), by path.
fn snapshot(folders: &[&Path]) -> TestResult<BTreeMap<PathBuf, Option<Vec<u8>>>> {
    let mut found = BTreeMap::new();
    let mut pending: Vec<PathBuf> = folders.iter().map(|folder| folder.to_path_buf()).collect();
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let path = entry.path();
            if matches!(entry.file_name().to_str(), Some(".git" | ".cortex")) {
                continue;
            }
            if entry.file_type()?.is_dir() {
                found.insert(path.clone(), None);
                pending.push(path);
            } else {
                found.insert(path.clone(), Some(std::fs::read(&path)?));
            }
        }
    }
    Ok(found)
}

/// Copilot at the project's scope in a repository that already has an empty `.github`: init makes
/// `.github/hooks` alone, and uninstall takes that away and keeps `.github`, which was there.
#[test]
fn init_sets_up_copilot_beside_an_empty_github_folder() -> TestResult {
    checked(&Case {
        agent: "copilot",
        title: "GitHub Copilot",
        place: Place::Repository(".github/hooks/cortex.json"),
        before: None,
        taken_out: "SessionStart",
        folders: &[".github"],
        expected: |line| json!({"version": 1, "hooks": copilot_hooks(line)}),
    })
}

/// OpenHands' events in snake case, as its documentation spells them, each a matcher group.
fn openhands_snake(line: &dyn Fn(&str) -> String) -> serde_json::Map<String, Value> {
    let group = |event: &str, seconds: u64| json!({"hooks": [{"type": "command", "command": line(event), "timeout": seconds}]});
    let mut events = serde_json::Map::new();
    events.insert(
        "session_start".to_owned(),
        json!([group("SessionStart", 2)]),
    );
    events.insert(
        "user_prompt_submit".to_owned(),
        json!([group("UserPromptSubmit", 1)]),
    );
    events.insert(
        "post_tool_use".to_owned(),
        json!([{"matcher": "^(terminal|file_editor)$",
            "hooks": [{"type": "command", "command": line("PostToolUse"), "timeout": 1}]}]),
    );
    events.insert("stop".to_owned(), json!([group("Stop", 1)]));
    events.insert("session_end".to_owned(), json!([group("SessionEnd", 1)]));
    events
}

/// OpenHands with no hooks file and no `~/.openhands`: init makes both, the events in snake case;
/// uninstall takes both away.
#[test]
fn init_sets_up_openhands_in_snake_case() -> TestResult {
    checked(&Case {
        agent: "openhands",
        title: "OpenHands",
        place: Place::Home(".openhands/hooks.json"),
        before: None,
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| Value::Object(openhands_snake(line)),
    })
}

/// OpenHands' file keeping its events under `hooks`, the user's `stop` in snake case and a key of
/// the user's beside: Cortex's events go under `hooks` in snake case, its `Stop` hook after the
/// user's in the same `stop`, and the user's key stays.
#[test]
fn init_sets_up_openhands_under_the_user_s_hooks() -> TestResult {
    checked(&Case {
        agent: "openhands",
        title: "OpenHands",
        place: Place::Home(".openhands/hooks.json"),
        before: Some(
            r#"{"hooks": {"stop": [{"hooks": [{"command": "notify.sh"}]}]}, "note": "kept"}"#,
        ),
        taken_out: "SessionStart",
        folders: &[],
        expected: |line| {
            let mut events = openhands_snake(line);
            if let Some(Value::Array(stop)) = events.get_mut("stop") {
                stop.insert(0, json!({"hooks": [{"command": "notify.sh"}]}));
            }
            json!({"hooks": events, "note": "kept"})
        },
    })
}

/// Copilot and Kiro run the hooks of both their scopes' files: with Cortex's hooks in the user's
/// file, init at the project's scope refuses, naming the file and the uninstall that takes them
/// out, and makes no file; once they are out it writes the repository's, and then init at the
/// user's scope refuses the same way.
#[test]
fn init_refuses_a_second_scope_of_copilot_and_kiro() -> TestResult {
    for (agent, title, project, user) in [
        (
            "copilot",
            "GitHub Copilot",
            ".github/hooks/cortex.json",
            ".copilot/hooks/cortex.json",
        ),
        (
            "kiro",
            "Kiro",
            ".kiro/hooks/cortex.json",
            ".kiro/hooks/cortex.json",
        ),
    ] {
        let repo = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        git(repo.path(), &["init", "-q"], home.path())?;
        let _daemon = StopsDaemon::of(repo.path(), home.path());
        let init = |scope: &str| {
            cortex(
                &["init", "--yes", "--agents", agent, "--scope", scope],
                repo.path(),
                home.path(),
            )
        };
        assert!(init("user")?.is_success, "{agent}");
        let refused = init("project")?;
        assert!(!refused.is_success, "{agent}");
        let said = format!(
            "holds Cortex's {title} hooks already, and {title} runs the hooks of both files"
        );
        assert!(refused.stderr.contains(&said), "{}", refused.stderr);
        assert!(
            refused
                .stderr
                .contains("(`cortex uninstall --scope user` undoes"),
            "{}",
            refused.stderr
        );
        assert!(!repo.path().join(project).exists(), "{agent}");
        let undone = cortex(&["uninstall", "--scope", "user"], repo.path(), home.path())?;
        assert!(undone.is_success, "{agent}: {}", undone.stderr);
        assert!(!home.path().join(user).exists(), "{agent}");
        assert!(init("project")?.is_success, "{agent}");
        let refused = init("user")?;
        assert!(!refused.is_success, "{agent}");
        assert!(refused.stderr.contains(&said), "{}", refused.stderr);
        assert!(
            refused.stderr.contains("(`cortex uninstall` undoes"),
            "{}",
            refused.stderr
        );
        assert!(!home.path().join(user).exists(), "{agent}");
    }
    Ok(())
}

/// A switch the user turned Cortex's hooks off with stays off when init runs again, and doctor
/// says so with the switch, not init, as the fix: Antigravity's on its hook name `cortex`, Kiro's
/// on Cortex's `Stop` hook.
#[test]
fn a_switch_the_user_turned_off_stays_off() -> TestResult {
    type TurnOff = fn(&mut Value) -> Option<()>;
    let cases: [(&str, &str, &str, TurnOff); 2] = [
        (
            "antigravity",
            "Antigravity",
            ".gemini/config/hooks.json",
            |value| {
                value
                    .get_mut("cortex")?
                    .as_object_mut()?
                    .insert("enabled".to_owned(), false.into());
                Some(())
            },
        ),
        ("kiro", "Kiro", ".kiro/hooks/cortex.json", |value| {
            let hooks = value.get_mut("hooks")?.as_array_mut()?;
            let stop = hooks
                .iter_mut()
                .find(|hook| hook.get("trigger").and_then(Value::as_str) == Some("Stop"))?;
            stop.as_object_mut()?
                .insert("enabled".to_owned(), false.into());
            Some(())
        }),
    ];
    for (agent, title, path, turn_off) in cases {
        let repo = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        git(repo.path(), &["init", "-q"], home.path())?;
        let _daemon = StopsDaemon::of(repo.path(), home.path());
        let init = || {
            cortex(
                &["init", "--yes", "--agents", agent, "--scope", "user"],
                repo.path(),
                home.path(),
            )
        };
        let done = init()?;
        assert!(done.is_success, "{agent}: {}", done.stderr);
        let shown = written_to(&done, title)?;
        let file = home.path().join(path);
        let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
        turn_off(&mut value).ok_or(agent)?;
        let off = serde_json::to_string_pretty(&value)?;
        std::fs::write(&file, &off)?;
        assert!(init()?.is_success, "{agent}");
        assert_eq!(
            serde_json::from_str::<Value>(&std::fs::read_to_string(&file)?)?,
            value,
            "{agent}"
        );
        let doctor = cortex(&["doctor"], repo.path(), home.path())?;
        assert_eq!(
            agents_lines(&doctor),
            [
                format!(
                    "FAIL  agents: {title}: Cortex's hooks in {} are turned off there \
                     (`\"enabled\": false`)",
                    shown.display()
                ),
                format!(
                    "      fix: take `\"enabled\": false` off Cortex's hooks in {} for {title} to \
                     run them; init leaves that switch as it is",
                    shown.display()
                ),
            ],
            "{agent}"
        );
    }
    Ok(())
}
