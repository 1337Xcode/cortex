//! `cortex init --agents` for the agents whose hooks are files of Cortex's own (the plugins of
//! OpenCode, Kilo Code, Amp and OpenClaw, and Cline's hook scripts) and whose MCP server goes into
//! their JSON settings, and for the agents init only tells the user about (Hermes Agent, goose),
//! each on a temporary git repository with an empty home folder of the test's own, through the
//! real binary. Init writes exactly the files expected here: a plugin as its reviewed fixture with
//! this binary's path, a hook script and an MCP entry typed by hand from each agent's
//! documentation; the user's own files, keys and comments stay; init again changes nothing; doctor
//! finds everything current; uninstall leaves every file and folder as it was before init. Init
//! refuses to write over a file of the user's, or one scope while the other holds Cortex's files,
//! and says what to add to a file it does not rewrite.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::init::{agents_lines, cortex, git, run, run_with, snapshot};
use common::{StopsDaemon, TestResult};
use serde_json::{Value, json};

/// Which folder a test's file is under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    /// The home folder.
    Home,
    /// The repository's root.
    Repository,
}

/// What a file holds after init.
enum Holds {
    /// These very bytes.
    Text(String),
    /// This JSON, whatever its layout.
    Json(Value),
}

/// A file init writes: where, what it holds after, and whether it must be executable.
struct Written {
    /// The folder it is under.
    base: Base,
    /// Its path there, with `/` between folders.
    path: String,
    /// What it holds.
    holds: Holds,
    /// Whether the agent runs it as a program.
    is_executable: bool,
}

/// One agent's run.
struct Case {
    /// The agent's name on Cortex's command line.
    agent: &'static str,
    /// The scope init runs at.
    scope: &'static str,
    /// The user's own files before init, each under its folder with what it holds.
    before: &'static [(Base, &'static str, &'static str)],
    /// The files init writes, given this binary's path.
    written: fn(&str) -> TestResult<Vec<Written>>,
    /// The beginnings of lines init says of what the user is to do.
    told: &'static [&'static str],
    /// The lines uninstall says once it took the agent's files out, beyond what it undid.
    said_by_uninstall: &'static [&'static str],
}

/// The path `relative`, with `/` between folders, under `base` of `repo` and `home`.
fn path_of(base: Base, relative: &str, repo: &Path, home: &Path) -> PathBuf {
    let folder = match base {
        Base::Home => home,
        Base::Repository => repo,
    };
    relative
        .split('/')
        .fold(folder.to_path_buf(), |path, part| path.join(part))
}

/// This binary's path as it knows it, which init writes: the path Cargo gives the tests, with the
/// separators of Windows there, since a target folder given with `/` gives a path of both.
fn program() -> String {
    let given = env!("CARGO_BIN_EXE_cortex");
    if cfg!(windows) {
        given.replace('/', "\\")
    } else {
        given.to_owned()
    }
}

/// The plugin file `name` of `agent` as its reviewed fixture has it, a script running `program`.
fn plugin(agent: &str, name: &str, program: &str) -> TestResult<String> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(agent)
        .join("plugin-2026-11-06/generated")
        .join(name);
    let text = std::fs::read_to_string(fixture)?;
    let line = "const CORTEX = \"cortex\";";
    let is_script = Path::new(name)
        .extension()
        .is_some_and(|extension| extension == "js");
    assert_eq!(
        text.matches(line).count(),
        usize::from(is_script),
        "{agent}: {name}"
    );
    let program = serde_json::to_string(program)?;
    Ok(text.replace(line, &format!("const CORTEX = {program};")))
}

/// A plugin file of `agent`'s, `name` in its fixture, written at `path` under `base`.
fn plugin_at(
    agent: &str,
    name: &str,
    (base, path): (Base, &str),
    program: &str,
) -> TestResult<Written> {
    Ok(Written {
        base,
        path: path.to_owned(),
        holds: Holds::Text(plugin(agent, name, program)?),
        is_executable: false,
    })
}

/// A JSON settings file at `path` under the home folder, holding `value`.
fn settings_at(path: &str, value: Value) -> Written {
    Written {
        base: Base::Home,
        path: path.to_owned(),
        holds: Holds::Json(value),
        is_executable: false,
    }
}

/// The events of Cline's hooks, as its adapter registers them.
const CLINE_EVENTS: [&str; 6] = [
    "TaskStart",
    "TaskResume",
    "UserPromptSubmit",
    "PostToolUse",
    "PreCompact",
    "TaskComplete",
];

/// Cortex's Cline hook scripts in `folder` under `base`, for this system, typed by hand: on
/// Windows a PowerShell script with a UTF-8 byte order mark passing its standard input on in
/// UTF-8, elsewhere a POSIX shell script.
fn cline_scripts(base: Base, folder: &str, program: &str) -> Vec<Written> {
    let program = program.replace('\\', "/");
    CLINE_EVENTS
        .iter()
        .map(|event| {
            let first = format!(
                "# Cortex's Cline hook for {event}, written by `cortex init`; `cortex uninstall` \
                 removes it."
            );
            let (name, text) = if cfg!(windows) {
                (
                    format!("{event}.ps1"),
                    format!(
                        "\u{feff}{first}\ntry {{ [Console]::InputEncoding = [Console]::OutputEncoding = \
                         [Text.UTF8Encoding]::new($false) }} catch {{}}\n$OutputEncoding = \
                         [Text.UTF8Encoding]::new($false)\n[Console]::In.ReadToEnd() | {program} \
                         hook --agent cline --event {event}\n"
                    ),
                )
            } else {
                (
                    (*event).to_owned(),
                    format!(
                        "#!/bin/sh\n{first}\nexec {program} hook --agent cline --event {event}\n"
                    ),
                )
            };
            Written {
                base,
                path: format!("{folder}/{name}"),
                holds: Holds::Text(text),
                is_executable: true,
            }
        })
        .collect()
}

/// Cline's MCP settings with the user's server first and Cortex's after it.
fn cline_mcp(program: &str) -> Written {
    settings_at(
        ".cline/data/settings/cline_mcp_settings.json",
        json!({"mcpServers": {
            "files": {"command": "node", "args": ["files.js"], "disabled": false},
            "cortex": {"command": program, "args": ["mcp"]}
        }}),
    )
}

/// The user's Cline MCP settings before init.
const CLINE_MCP_BEFORE: &str =
    r#"{"mcpServers": {"files": {"command": "node", "args": ["files.js"], "disabled": false}}}"#;

/// OpenCode at the project's scope: the plugin beside the user's own in `.opencode/plugins/`, and
/// Cortex's MCP server after the user's in the user's `opencode.json`, its other keys kept.
#[test]
fn init_sets_up_opencode_in_the_repository() -> TestResult {
    checked(&Case {
        agent: "opencode",
        scope: "project",
        before: &[
            (
                Base::Repository,
                ".opencode/plugins/mine.js",
                "export const Mine = async () => ({});\n",
            ),
            (
                Base::Home,
                ".config/opencode/opencode.json",
                r#"{"$schema": "https://opencode.ai/config.json", "mcp": {"github": {"type": "remote", "url": "https://example.test/mcp"}}, "theme": "dark"}"#,
            ),
        ],
        written: |program| {
            Ok(vec![
                plugin_at(
                    "opencode",
                    "cortex.js",
                    (Base::Repository, ".opencode/plugins/cortex.js"),
                    program,
                )?,
                settings_at(
                    ".config/opencode/opencode.json",
                    json!({
                        "$schema": "https://opencode.ai/config.json",
                        "mcp": {
                            "github": {"type": "remote", "url": "https://example.test/mcp"},
                            "cortex": {"type": "local", "command": [program, "mcp"]}
                        },
                        "theme": "dark"
                    }),
                ),
            ])
        },
        told: &["opencode: to do yourself: run OpenCode 1.x"],
        said_by_uninstall: &[],
    })
}

/// Kilo Code at the user's scope: the plugin in `~/.config/kilo/plugin/`, and Cortex's MCP server
/// in a `kilo.json` made beside the user's `kilo.jsonc`, whose comments are never touched.
#[test]
fn init_sets_up_kilo_for_the_user() -> TestResult {
    checked(&Case {
        agent: "kilo",
        scope: "user",
        before: &[(
            Base::Home,
            ".config/kilo/kilo.jsonc",
            "{\n  // the user's model\n  \"model\": \"x\",\n}\n",
        )],
        written: |program| {
            Ok(vec![
                plugin_at(
                    "kilo",
                    "cortex.js",
                    (Base::Home, ".config/kilo/plugin/cortex.js"),
                    program,
                )?,
                settings_at(
                    ".config/kilo/kilo.json",
                    json!({"mcp": {"cortex": {"type": "local", "command": [program, "mcp"]}}}),
                ),
            ])
        },
        told: &[],
        said_by_uninstall: &[],
    })
}

/// Amp at the user's scope: the plugin beside the user's own, and Cortex's MCP server under the
/// top-level key `amp.mcpServers` after the user's, the user's other setting kept.
#[test]
fn init_sets_up_amp_for_the_user() -> TestResult {
    checked(&Case {
        agent: "amp",
        scope: "user",
        before: &[
            (
                Base::Home,
                ".config/amp/settings.json",
                r#"{"amp.notifications.enabled": false, "amp.mcpServers": {"playwright": {"command": "npx", "args": ["-y", "@playwright/mcp"]}}}"#,
            ),
            (
                Base::Home,
                ".config/amp/plugins/mine.js",
                "export default function () {}\n",
            ),
        ],
        written: |program| {
            Ok(vec![
                plugin_at(
                    "amp",
                    "cortex.js",
                    (Base::Home, ".config/amp/plugins/cortex.js"),
                    program,
                )?,
                settings_at(
                    ".config/amp/settings.json",
                    json!({
                        "amp.notifications.enabled": false,
                        "amp.mcpServers": {
                            "playwright": {"command": "npx", "args": ["-y", "@playwright/mcp"]},
                            "cortex": {"command": program, "args": ["mcp"]}
                        }
                    }),
                ),
            ])
        },
        told: &[],
        said_by_uninstall: &[],
    })
}

/// OpenClaw: the plugin folder in `~/.openclaw/extensions/cortex/`, whatever the scope, Cortex's
/// MCP server under `mcp.servers` beside the user's gateway settings, and the plugin's switches
/// said, never written; once uninstall takes the plugin out, it says the user's block for it can
/// go. On Windows OpenClaw gets nothing (see [`openclaw_is_set_up_inside_wsl_on_windows`]).
#[cfg(not(windows))]
#[test]
fn init_sets_up_openclaw() -> TestResult {
    checked(&Case {
        agent: "openclaw",
        scope: "project",
        before: &[(
            Base::Home,
            ".openclaw/openclaw.json",
            r#"{"gateway": {"port": 18789}}"#,
        )],
        written: |program| {
            let mut written = Vec::new();
            for name in [
                "cortex/index.js",
                "cortex/package.json",
                "cortex/openclaw.plugin.json",
            ] {
                let path = format!(".openclaw/extensions/{name}");
                written.push(plugin_at("openclaw", name, (Base::Home, &path), program)?);
            }
            written.push(settings_at(
                ".openclaw/openclaw.json",
                json!({
                    "gateway": {"port": 18789},
                    "mcp": {"servers": {"cortex": {"command": program, "args": ["mcp"]}}}
                }),
            ));
            Ok(written)
        },
        told: &[
            "OpenClaw: to do yourself: turn Cortex's plugin on with `openclaw plugins enable \
             cortex`",
            "OpenClaw: to do yourself: let its hooks see the conversation: set \
             `plugins.entries.cortex.hooks.allowConversationAccess` to true",
        ],
        said_by_uninstall: &[
            "note: if you added `plugins.entries.cortex` to ~/.openclaw/openclaw.json, as init \
             said, it can go now that Cortex's OpenClaw plugin is gone",
        ],
    })
}

/// Cline at the project's scope: a hook script per event in `.clinerules/hooks/` beside the
/// user's own notes, and Cortex's MCP server in the user's MCP settings after the user's.
#[test]
fn init_sets_up_cline_in_the_repository() -> TestResult {
    checked(&Case {
        agent: "cline",
        scope: "project",
        before: &[
            (Base::Repository, ".clinerules/hooks/notes.md", "my hooks\n"),
            (
                Base::Home,
                ".cline/data/settings/cline_mcp_settings.json",
                CLINE_MCP_BEFORE,
            ),
        ],
        written: |program| {
            let mut written = cline_scripts(Base::Repository, ".clinerules/hooks", program);
            written.push(cline_mcp(program));
            Ok(written)
        },
        told: &["Cline: to do yourself: keep Cline's Hooks setting on"],
        said_by_uninstall: &[],
    })
}

/// Cline at the user's scope: the hook scripts in `Cline/Hooks/` of the documents folder, made
/// with its folders, which uninstall takes away again. The documents folder is the system's: on
/// Windows its answer must lie in the test's home folder, or the test would write the user's own.
#[test]
fn init_sets_up_cline_for_the_user() -> TestResult {
    documents_are_in_home()?;
    checked(&Case {
        agent: "cline",
        scope: "user",
        before: &[(
            Base::Home,
            ".cline/data/settings/cline_mcp_settings.json",
            CLINE_MCP_BEFORE,
        )],
        written: |program| {
            let mut written = cline_scripts(Base::Home, "Documents/Cline/Hooks", program);
            written.push(cline_mcp(program));
            Ok(written)
        },
        told: &["Cline: to do yourself: keep Cline's Hooks setting on"],
        said_by_uninstall: &[],
    })
}

/// Runs `case` through init, init again, doctor, uninstall and doctor again, as the module says.
fn checked(case: &Case) -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    with_profile_folders(home.path())?;
    let path = |base: Base, relative: &str| path_of(base, relative, repo.path(), home.path());
    let program = program();
    // The premise of the expected scripts: the test binary's path needs no quoting.
    assert!(!program.contains(' '), "{program}");
    let written = (case.written)(&program)?;
    for (base, relative, text) in case.before {
        let file = path(*base, relative);
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&file, text)?;
        // A script an older build wrote is executable, as init made it.
        let is_script = written
            .iter()
            .any(|w| w.base == *base && w.path == *relative && w.is_executable);
        if is_script {
            made_executable(&file)?;
        }
    }
    let before = snapshot(&[repo.path(), home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let agent = case.agent;
    let init = || {
        cortex(
            &["init", "--yes", "--agents", agent, "--scope", case.scope],
            repo.path(),
            home.path(),
        )
    };
    let done = init()?;
    assert!(done.is_success, "{agent}: {}", done.stderr);
    let mut texts = Vec::new();
    for file in &written {
        let text = std::fs::read_to_string(path(file.base, &file.path))?;
        match &file.holds {
            Holds::Text(expected) => assert_eq!(&text, expected, "{agent}: {}", file.path),
            Holds::Json(expected) => {
                assert_eq!(
                    &serde_json::from_str::<Value>(&text)?,
                    expected,
                    "{agent}: {}",
                    file.path
                );
            }
        }
        assert_executable(&path(file.base, &file.path), file.is_executable)?;
        let was_there = case
            .before
            .iter()
            .any(|(base, relative, _)| *base == file.base && *relative == file.path);
        if file.base == Base::Repository && !was_there {
            let ignores = std::fs::read_to_string(repo.path().join(".gitignore"))?;
            assert!(ignores.lines().any(|line| line == file.path), "{ignores}");
        }
        texts.push(text);
    }
    for (base, relative, text) in case.before {
        if !written
            .iter()
            .any(|file| file.base == *base && file.path == *relative)
        {
            assert_eq!(
                &std::fs::read_to_string(path(*base, relative))?,
                text,
                "{agent}"
            );
        }
    }
    let said = done
        .stdout
        .lines()
        .filter(|line| line.contains(" written to "))
        .count();
    assert_eq!(said, written.len(), "{}", done.stdout);
    for told in case.told {
        assert!(
            done.stdout.lines().any(|line| line.starts_with(told)),
            "{told}: {}",
            done.stdout
        );
    }
    let again = init()?;
    assert!(again.is_success, "{agent}: {}", again.stderr);
    assert!(!again.stdout.contains(" written to "), "{}", again.stdout);
    for (file, text) in written.iter().zip(&texts) {
        assert_eq!(&std::fs::read_to_string(path(file.base, &file.path))?, text);
    }
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    let lines = agents_lines(&doctor);
    let current = lines
        .iter()
        .filter(|line| line.contains(" is current in "))
        .count();
    assert_eq!(current, written.len(), "{lines:?}");
    assert!(
        lines.iter().all(|line| line.starts_with("ok    agents: ")),
        "{lines:?}"
    );
    let undone = cortex(&["uninstall", "--scope", "user"], repo.path(), home.path())?;
    assert!(undone.is_success, "{agent}: {}", undone.stderr);
    for said in case.said_by_uninstall {
        assert!(
            undone.stdout.lines().any(|line| line == *said),
            "{said}: {}",
            undone.stdout
        );
    }
    assert_eq!(snapshot(&[repo.path(), home.path()])?, before, "{agent}");
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(agents_lines(&doctor), Vec::<String>::new(), "{agent}");
    Ok(())
}

/// Makes in `home` the folder `AppData/Roaming` on Windows, which every real profile has and which
/// PowerShell makes when it is missing, as it is when init asks it for the documents folder: so
/// that a test comparing the home folder before and after sees only what Cortex did.
fn with_profile_folders(home: &Path) -> TestResult {
    if cfg!(windows) {
        std::fs::create_dir_all(home.join("AppData").join("Roaming"))?;
    }
    Ok(())
}

/// Checks that the system names the documents folder inside the home folder a test gives it, as
/// Windows does unless the user moved the folder (to OneDrive, say): the premise of a test of
/// Cline's user scope, which would otherwise write in the user's own documents. Elsewhere the
/// test's home has no `user-dirs.dirs`, so the folder is `~/Documents`.
fn documents_are_in_home() -> TestResult {
    if !cfg!(windows) {
        return Ok(());
    }
    let probe = tempfile::tempdir()?;
    let root = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let output =
        Command::new(Path::new(&root).join("System32/WindowsPowerShell/v1.0/powershell.exe"))
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Environment]::GetFolderPath('MyDocuments', 'DoNotVerify')",
            ])
            .env("USERPROFILE", probe.path())
            .output()?;
    let named = PathBuf::from(String::from_utf8(output.stdout)?.trim());
    if named != probe.path().join("Documents") {
        return Err(format!(
            "this machine keeps the documents folder at {}, outside any test's home, so this \
             test cannot run here without writing in it",
            named.display()
        )
        .into());
    }
    Ok(())
}

/// Checks that the file at `path` is executable, or not, as `is_executable` says, where files
/// carry that bit.
#[cfg(unix)]
fn assert_executable(path: &Path, is_executable: bool) -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(path)?.permissions().mode();
    assert_eq!(mode & 0o100 != 0, is_executable, "{}", path.display());
    Ok(())
}

/// Makes the file at `path` executable by its owner, where files carry that bit.
#[cfg(unix)]
fn made_executable(path: &Path) -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Nothing to do where files carry no executable bit.
#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the Unix version of the same function can fail"
)]
fn made_executable(_: &Path) -> TestResult {
    Ok(())
}

/// Nothing to check where files carry no executable bit.
#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the Unix version of the same function can fail"
)]
fn assert_executable(_: &Path, _: bool) -> TestResult {
    Ok(())
}

/// A hook script of the user's with the name of one of Cortex's is never written over: init for
/// Cline refuses, says why, and writes nothing at all.
#[test]
fn init_refuses_a_user_s_hook_of_the_same_name() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    with_profile_folders(home.path())?;
    let name = if cfg!(windows) {
        "TaskStart.ps1"
    } else {
        "TaskStart"
    };
    let hooks = repo.path().join(".clinerules").join("hooks");
    std::fs::create_dir_all(&hooks)?;
    std::fs::write(hooks.join(name), "echo mine\n")?;
    let before = snapshot(&[repo.path(), home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let refused = cortex(
        &["init", "--yes", "--agents", "cline", "--scope", "project"],
        repo.path(),
        home.path(),
    )?;
    assert!(!refused.is_success, "{}", refused.stdout);
    assert!(
        refused
            .stderr
            .contains("is a file of yours, not Cortex's, and Cortex does not write over it"),
        "{}",
        refused.stderr
    );
    assert_eq!(snapshot(&[repo.path(), home.path()])?, before);
    Ok(())
}

/// OpenCode loads the plugins of both its folders: with Cortex's in the user's, init at the
/// project's scope refuses, naming the folder and the uninstall that takes it out, and writes no
/// plugin; once it is out, the repository's is written, and then init at the user's scope refuses
/// the same way.
#[test]
fn init_refuses_a_second_scope_of_a_plugin() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let init = |scope: &str| {
        cortex(
            &["init", "--yes", "--agents", "opencode", "--scope", scope],
            repo.path(),
            home.path(),
        )
    };
    let project = repo.path().join(".opencode/plugins/cortex.js");
    let user = home.path().join(".config/opencode/plugins/cortex.js");
    let said =
        "holds Cortex's opencode plugin already, and opencode loads the plugins of both folders";
    assert!(init("user")?.is_success);
    let refused = init("project")?;
    assert!(!refused.is_success);
    assert!(refused.stderr.contains(said), "{}", refused.stderr);
    assert!(
        refused
            .stderr
            .contains("(`cortex uninstall --scope user` undoes"),
        "{}",
        refused.stderr
    );
    assert!(!project.exists());
    let undone = cortex(&["uninstall", "--scope", "user"], repo.path(), home.path())?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert!(!user.exists());
    assert!(init("project")?.is_success);
    let refused = init("user")?;
    assert!(!refused.is_success);
    assert!(refused.stderr.contains(said), "{}", refused.stderr);
    assert!(
        refused.stderr.contains("(`cortex uninstall` undoes"),
        "{}",
        refused.stderr
    );
    assert!(!user.exists());
    Ok(())
}

/// A settings file with comments is never rewritten: init writes OpenCode's plugin, leaves the
/// user's `opencode.jsonc` byte for byte, and says the entry to add there; uninstall then leaves
/// everything as it was.
#[test]
fn a_settings_file_with_comments_is_told_not_written() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let settings = home.path().join(".config/opencode/opencode.jsonc");
    std::fs::create_dir_all(home.path().join(".config/opencode"))?;
    let text = "{\n  // mine\n  \"theme\": \"dark\"\n}\n";
    std::fs::write(&settings, text)?;
    let before = snapshot(&[repo.path(), home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "opencode", "--scope", "user"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(std::fs::read_to_string(&settings)?, text);
    assert!(
        home.path()
            .join(".config/opencode/plugins/cortex.js")
            .is_file()
    );
    let program = serde_json::to_string(&program())?;
    let entry = format!(
        "under `mcp`: \"cortex\": {{\"type\":\"local\",\"command\":[{program},\"mcp\"]}}; Cortex \
         leaves that file as it is, since it is not JSON (line 2, column 3), and it never \
         rewrites a file with comments"
    );
    let told = done
        .stdout
        .lines()
        .find(|line| line.starts_with("opencode: to do yourself: add Cortex's MCP server to "))
        .ok_or_else(|| done.stdout.clone())?;
    assert!(told.ends_with(&entry), "{told}");
    let undone = cortex(&["uninstall", "--scope", "user"], repo.path(), home.path())?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(snapshot(&[repo.path(), home.path()])?, before);
    Ok(())
}

/// Settings the user changed after init keep the change: uninstall takes only Cortex's MCP server
/// out of Amp's, and removes its plugin, which the user left as init wrote it.
#[test]
fn uninstall_takes_cortex_s_server_out_of_changed_settings() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "amp", "--scope", "user"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    let settings = home.path().join(".config/amp/settings.json");
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&settings)?)?;
    value
        .as_object_mut()
        .ok_or("not an object")?
        .insert("amp.dangerouslyAllowAll".to_owned(), false.into());
    std::fs::write(&settings, serde_json::to_string_pretty(&value)?)?;
    let undone = cortex(&["uninstall", "--scope", "user"], repo.path(), home.path())?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&std::fs::read_to_string(&settings)?)?,
        json!({"amp.dangerouslyAllowAll": false})
    );
    assert!(!home.path().join(".config/amp/plugins").exists());
    Ok(())
}

/// The lines init says to add to Hermes Agent's configuration for this binary, typed by hand.
fn hermes_lines(program: &str) -> String {
    let forward = program.replace('\\', "/");
    let hook = |event: &str| format!("'{forward} hook --agent hermes --event {event}'");
    format!(
        "hooks:\n  on_session_start:\n    - command: {}\n      timeout: 2\n  pre_llm_call:\n    - \
         command: {}\n      timeout: 1\n  post_tool_call:\n    - matcher: \
         '^(read_file|write_file|patch|search_files|terminal)$'\n      command: {}\n      timeout: \
         1\n  on_session_end:\n    - command: {}\n      timeout: 1\nmcp_servers:\n  cortex:\n    \
         command: '{}'\n    args: ['mcp']",
        hook("on_session_start"),
        hook("pre_llm_call"),
        hook("post_tool_call"),
        hook("on_session_end"),
        program
    )
}

/// Hermes Agent's configuration file under the home folder.
const HERMES_CONFIG: &str = if cfg!(windows) {
    "AppData/Local/hermes/config.yaml"
} else {
    ".hermes/config.yaml"
};

/// Hermes Agent and goose get nothing written: init says the lines to add to each one's YAML and
/// the approval that is the user's, and leaves the home folder as it was; doctor then finds the
/// lines once the user adds them, and says when some are missing.
#[test]
fn init_tells_hermes_and_goose_and_writes_nothing() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let homes_before = snapshot(&[home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &[
            "init",
            "--yes",
            "--agents",
            "hermes,goose",
            "--scope",
            "user",
        ],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(snapshot(&[home.path()])?, homes_before);
    let program = &program();
    let hermes = path_of(Base::Home, HERMES_CONFIG, repo.path(), home.path());
    let expected = format!(
        "Hermes Agent: to do yourself: Cortex writes nothing for it, since its configuration is \
         YAML, which Cortex does not write, and Hermes runs a hook only once you approve it; add \
         these lines to {} yourself, beside any keys of the same name there:\n{}\n",
        hermes.display(),
        hermes_lines(program)
    );
    assert!(done.stdout.contains(&expected), "{}", done.stdout);
    assert!(
        done.stdout.contains(
            "Hermes Agent: to do yourself: approve each of Cortex's hooks when Hermes first asks"
        ),
        "{}",
        done.stdout
    );
    let goose_file = if cfg!(windows) {
        "AppData/Roaming/Block/goose/config/config.yaml"
    } else {
        ".config/goose/config.yaml"
    };
    let goose = format!(
        "goose: to do yourself: Cortex writes nothing for it, since its configuration is YAML, \
         which Cortex does not write; and it gets no hooks, since Cortex has no adapter for \
         goose's hook payloads yet; add these lines to {} yourself, beside any keys of the same \
         name there:\nextensions:\n  cortex:\n    name: cortex\n    type: stdio\n    cmd: '{program}'\n    \
         args: ['mcp']\n    enabled: true\n    env_keys: []\n    envs: {{}}\n    timeout: 300\n",
        path_of(Base::Home, goose_file, repo.path(), home.path()).display()
    );
    assert!(done.stdout.contains(&goose), "{}", done.stdout);
    std::fs::create_dir_all(hermes.parent().ok_or("no folder")?)?;
    std::fs::write(&hermes, format!("model: x\n{}\n", hermes_lines(program)))?;
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    let lines = agents_lines(&doctor);
    let found = format!(
        "ok    agents: Hermes Agent: Cortex's lines are in {}",
        hermes.display()
    );
    assert!(
        lines.iter().any(|line| line.starts_with(&found)),
        "{lines:?}"
    );
    let without_one = hermes_lines(program).replace("on_session_end", "on_session_finish");
    std::fs::write(&hermes, without_one)?;
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(
        agents_lines(&doctor),
        [
            format!(
                "FAIL  agents: Hermes Agent: {} holds some of Cortex's lines, not all this build \
                 says",
                hermes.display()
            ),
            format!(
                "      fix: add the lines `cortex init --agents hermes` prints to {}",
                hermes.display()
            ),
        ]
    );
    Ok(())
}

/// `cortex init --global` sets a plugin agent up in the user's folders, and `cortex uninstall
/// --global` leaves the home folder as it was.
#[test]
fn global_init_sets_up_a_plugin_agent() -> TestResult {
    let home = tempfile::tempdir()?;
    let before = snapshot(&[home.path()])?;
    let global = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.args(args);
        run(&mut command, home.path(), &["git"])
    };
    let done = global(&["init", "--global", "--yes", "--agents", "kilo"])?;
    assert!(done.is_success, "{}", done.stderr);
    let program = &program();
    let written = std::fs::read_to_string(home.path().join(".config/kilo/plugin/cortex.js"))?;
    assert_eq!(written, plugin("kilo", "cortex.js", program)?);
    let settings = std::fs::read_to_string(home.path().join(".config/kilo/kilo.json"))?;
    assert_eq!(
        serde_json::from_str::<Value>(&settings)?,
        json!({"mcp": {"cortex": {"type": "local", "command": [program, "mcp"]}}})
    );
    let undone = global(&["uninstall", "--global"])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(snapshot(&[home.path()])?, before);
    Ok(())
}

/// On Windows, where OpenClaw's Gateway runs in WSL and no folder of its own is documented, init
/// writes nothing for OpenClaw and says to set it up inside WSL.
#[cfg(windows)]
#[test]
fn openclaw_is_set_up_inside_wsl_on_windows() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let homes_before = snapshot(&[home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "openclaw", "--scope", "user"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert!(
        done.stdout.lines().any(|line| line.starts_with(
            "OpenClaw: to do yourself: set Cortex up for OpenClaw inside WSL, where its Gateway \
             runs"
        )),
        "{}",
        done.stdout
    );
    assert_eq!(snapshot(&[home.path()])?, homes_before);
    Ok(())
}

/// An older build's OpenClaw plugin, whose `package.json` and `openclaw.plugin.json` differ from
/// this build's and carry no marker, is Cortex's by its entry script's marker: init writes this
/// build's over it, and uninstall gives the older files back byte for byte.
#[cfg(not(windows))]
#[test]
fn init_replaces_an_older_build_s_openclaw_plugin() -> TestResult {
    checked(&Case {
        agent: "openclaw",
        scope: "user",
        before: &[
            (
                Base::Home,
                ".openclaw/extensions/cortex/index.js",
                "// Cortex's plugin for OpenClaw, written by `cortex init`; `cortex uninstall` \
                 removes it.\nexport default {};\n",
            ),
            (
                Base::Home,
                ".openclaw/extensions/cortex/package.json",
                "{\"name\": \"cortex\", \"version\": \"0.9.0\"}\n",
            ),
            (
                Base::Home,
                ".openclaw/extensions/cortex/openclaw.plugin.json",
                "{\"id\": \"cortex\"}\n",
            ),
        ],
        written: |program| {
            let mut written = Vec::new();
            for name in [
                "cortex/index.js",
                "cortex/package.json",
                "cortex/openclaw.plugin.json",
            ] {
                let path = format!(".openclaw/extensions/{name}");
                written.push(plugin_at("openclaw", name, (Base::Home, &path), program)?);
            }
            written.push(settings_at(
                ".openclaw/openclaw.json",
                json!({"mcp": {"servers": {"cortex": {"command": program, "args": ["mcp"]}}}}),
            ));
            Ok(written)
        },
        told: &[],
        said_by_uninstall: &[],
    })
}

/// Cline's hook for a session's start under this system's name.
const CLINE_TASK_START: &str = if cfg!(windows) {
    ".clinerules/hooks/TaskStart.ps1"
} else {
    ".clinerules/hooks/TaskStart"
};

/// An older build's Cline hook, carrying the marker and running another program, is replaced by
/// this build's, and uninstall gives it back byte for byte.
#[test]
fn init_replaces_an_older_build_s_cline_hook() -> TestResult {
    checked(&Case {
        agent: "cline",
        scope: "project",
        before: &[
            (
                Base::Repository,
                CLINE_TASK_START,
                "# Cortex's Cline hook for TaskStart, written by `cortex init`; `cortex uninstall` \
                 removes it.\n/old/cortex hook --agent cline --event TaskStart\n",
            ),
            (
                Base::Home,
                ".cline/data/settings/cline_mcp_settings.json",
                CLINE_MCP_BEFORE,
            ),
        ],
        written: |program| {
            let mut written = cline_scripts(Base::Repository, ".clinerules/hooks", program);
            written.push(cline_mcp(program));
            Ok(written)
        },
        told: &[],
        said_by_uninstall: &[],
    })
}

/// A plugin the user edited after init is theirs: uninstall keeps it as it is and says so.
#[test]
fn uninstall_keeps_a_plugin_the_user_edited() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "amp", "--scope", "user"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    let plugin = home.path().join(".config/amp/plugins/cortex.js");
    let edited = format!("{}// mine\n", std::fs::read_to_string(&plugin)?);
    std::fs::write(&plugin, &edited)?;
    let undone = cortex(&["uninstall", "--scope", "user"], repo.path(), home.path())?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(std::fs::read_to_string(&plugin)?, edited);
    assert!(
        undone.stdout.lines().any(|line| line.starts_with("kept ")
            && line.ends_with("cortex.js, which changed since init")),
        "{}",
        undone.stdout
    );
    Ok(())
}

/// Amp loads one plugin of a name, the repository's first, so Cortex's plugin is written at both
/// scopes, neither refusing the other.
#[test]
fn amp_takes_cortex_s_plugin_at_both_scopes() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    for scope in ["user", "project"] {
        let done = cortex(
            &["init", "--yes", "--agents", "amp", "--scope", scope],
            repo.path(),
            home.path(),
        )?;
        assert!(done.is_success, "{scope}: {}", done.stderr);
    }
    assert!(home.path().join(".config/amp/plugins/cortex.js").is_file());
    assert!(repo.path().join(".amp/plugins/cortex.js").is_file());
    Ok(())
}

/// With `XDG_CONFIG_HOME` naming an absolute folder, Amp's plugin and settings go there and
/// nothing under `~/.config`; doctor finds them current there, and uninstall leaves everything as
/// it was.
#[test]
fn xdg_config_home_moves_amp_s_files() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let xdg = home.path().join("xdg-config");
    git(repo.path(), &["init", "-q"], home.path())?;
    let before = snapshot(&[repo.path(), home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let with_xdg = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.args(args).arg("--root").arg(repo.path());
        run_with(
            &mut command,
            home.path(),
            &["git"],
            &[("XDG_CONFIG_HOME", xdg.as_path())],
        )
    };
    let done = with_xdg(&["init", "--yes", "--agents", "amp", "--scope", "user"])?;
    assert!(done.is_success, "{}", done.stderr);
    let program = program();
    assert_eq!(
        std::fs::read_to_string(xdg.join("amp/plugins/cortex.js"))?,
        plugin("amp", "cortex.js", &program)?
    );
    assert_eq!(
        serde_json::from_str::<Value>(&std::fs::read_to_string(xdg.join("amp/settings.json"))?)?,
        json!({"amp.mcpServers": {"cortex": {"command": program, "args": ["mcp"]}}})
    );
    assert!(!home.path().join(".config").exists());
    let lines = agents_lines(&with_xdg(&["doctor"])?);
    let current = lines
        .iter()
        .filter(|line| line.contains(" is current in "))
        .count();
    assert_eq!(current, 2, "{lines:?}");
    let undone = with_xdg(&["uninstall", "--scope", "user"])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(snapshot(&[repo.path(), home.path()])?, before);
    Ok(())
}

/// A server of another program named `cortex` in Amp's settings is the user's: the file stays as
/// it is, and init says the entry to add under another name.
#[test]
fn another_program_s_server_name_is_told_another_name() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let settings = home.path().join(".config/amp/settings.json");
    std::fs::create_dir_all(home.path().join(".config/amp"))?;
    let text = r#"{"amp.mcpServers": {"cortex": {"command": "npx", "args": ["cortex-mcp"]}}}"#;
    std::fs::write(&settings, text)?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "amp", "--scope", "user"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(std::fs::read_to_string(&settings)?, text);
    let program = serde_json::to_string(&program())?;
    let told = format!(
        "yourself, under `amp.mcpServers`: \"cortex-index\": {{\"command\":{program},\"args\":\
         [\"mcp\"]}}; Cortex leaves that file as it is, since its `amp.mcpServers.cortex` is an \
         MCP server of another program"
    );
    assert!(
        done.stdout.lines().any(|line| line
            .starts_with("Amp: to do yourself: add Cortex's MCP server to ")
            && line.ends_with(&told)),
        "{}",
        done.stdout
    );
    Ok(())
}

/// `"enabled": false` on Cortex's MCP server in Kilo Code's settings is the user's switch: init
/// again keeps it, and doctor says so with the switch, not init, as the fix.
#[test]
fn a_switch_turning_cortex_s_server_off_stays_off() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let init = || {
        cortex(
            &["init", "--yes", "--agents", "kilo", "--scope", "user"],
            repo.path(),
            home.path(),
        )
    };
    assert!(init()?.is_success);
    let settings = home.path().join(".config/kilo/kilo.json");
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&settings)?)?;
    value
        .pointer_mut("/mcp/cortex")
        .and_then(Value::as_object_mut)
        .ok_or("no entry")?
        .insert("enabled".to_owned(), false.into());
    std::fs::write(&settings, serde_json::to_string_pretty(&value)?)?;
    assert!(init()?.is_success);
    assert_eq!(
        serde_json::from_str::<Value>(&std::fs::read_to_string(&settings)?)?,
        value
    );
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    let lines = agents_lines(&doctor);
    let shown = lines
        .iter()
        .find_map(|line| {
            line.strip_prefix("FAIL  agents: Kilo Code: Cortex's MCP server in ")?
                .strip_suffix(" is turned off there (`\"enabled\": false`)")
        })
        .ok_or_else(|| format!("{lines:?}"))?;
    let fix = format!(
        "      fix: take `\"enabled\": false` off Cortex's MCP server in {shown} for Kilo Code to \
         start it; init leaves that switch as it is"
    );
    assert!(lines.contains(&fix), "{lines:?}");
    Ok(())
}

/// On Unix a Cline hook's executable bit is its switch: doctor says one the user turned off is
/// theirs to turn on, and init writing a newer script over it keeps it off.
#[cfg(unix)]
#[test]
fn a_hook_the_user_turned_off_stays_off() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let init = || {
        cortex(
            &["init", "--yes", "--agents", "cline", "--scope", "project"],
            repo.path(),
            home.path(),
        )
    };
    assert!(init()?.is_success);
    let hook = repo.path().join(".clinerules/hooks/TaskStart");
    let current = std::fs::read_to_string(&hook)?;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o600))?;
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    let lines = agents_lines(&doctor);
    assert!(
        lines.iter().any(
            |line| line.starts_with("FAIL  agents: Cline: Cortex's hook ")
                && line.ends_with("TaskStart is turned off there (not executable)")
        ),
        "{lines:?}"
    );
    std::fs::write(
        &hook,
        "#!/bin/sh\n# Cortex's Cline hook for TaskStart, written by `cortex init`\nexec /old/cortex\n",
    )?;
    assert!(init()?.is_success);
    assert_eq!(std::fs::read_to_string(&hook)?, current);
    assert_eq!(
        std::fs::metadata(&hook)?.permissions().mode() & 0o777,
        0o600
    );
    Ok(())
}

/// A program whose path holds a space and a quote: Cline's scripts call it quoted for their shell,
/// and the lines init prints for Hermes Agent, pasted into its configuration as they are, are what
/// doctor then finds.
#[test]
fn a_program_path_with_a_space_and_a_quote() -> TestResult {
    let folder = tempfile::tempdir()?;
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let bin = folder.path().join("o'bin dir");
    std::fs::create_dir(&bin)?;
    let copy = bin.join(Path::new(&program()).file_name().ok_or("no file name")?);
    std::fs::copy(program(), &copy)?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let at = |args: &[&str]| {
        let mut command = Command::new(&copy);
        command.args(args).arg("--root").arg(repo.path());
        run(&mut command, home.path(), &["git"])
    };
    let done = at(&[
        "init",
        "--yes",
        "--agents",
        "cline,hermes",
        "--scope",
        "project",
    ])?;
    assert!(done.is_success, "{}", done.stderr);
    let copy_text = copy.to_str().ok_or("not UTF-8")?;
    let last = if cfg!(windows) {
        format!(
            "[Console]::In.ReadToEnd() | & '{}' hook --agent cline --event TaskStart",
            copy_text.replace('\'', "''")
        )
    } else {
        format!(
            "exec '{}' hook --agent cline --event TaskStart",
            copy_text.replace('\'', r"'\''")
        )
    };
    let hook = std::fs::read_to_string(repo.path().join(CLINE_TASK_START))?;
    assert_eq!(hook.lines().last(), Some(last.as_str()), "{hook}");
    let lines: Vec<&str> = done
        .stdout
        .lines()
        .skip_while(|line| !line.starts_with("Hermes Agent: to do yourself: Cortex writes"))
        .skip(1)
        .take_while(|line| !line.starts_with("Hermes Agent: "))
        .collect();
    assert!(lines.first() == Some(&"hooks:"), "{}", done.stdout);
    let hermes = path_of(Base::Home, HERMES_CONFIG, repo.path(), home.path());
    std::fs::create_dir_all(hermes.parent().ok_or("no folder")?)?;
    std::fs::write(&hermes, format!("{}\n", lines.join("\n")))?;
    let doctor = at(&["doctor"])?;
    let lines = agents_lines(&doctor);
    let found = format!(
        "ok    agents: Hermes Agent: Cortex's lines are in {}",
        hermes.display()
    );
    assert!(
        lines.iter().any(|line| line.starts_with(&found)),
        "{lines:?}"
    );
    let current = lines
        .iter()
        .filter(|line| line.starts_with("ok    agents: Cline: Cortex's hook is current in "))
        .count();
    assert_eq!(current, CLINE_EVENTS.len(), "{lines:?}");
    Ok(())
}
