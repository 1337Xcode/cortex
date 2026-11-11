//! `cortex init --agents` for the agents Cortex reaches through MCP alone (Zed, Continue, Warp,
//! Windsurf's Cascade, Roo Code) and for Aider, which has neither hooks nor MCP, each on a
//! temporary git repository with an empty home folder of the test's own, through the real binary.
//! Init writes exactly the files typed here by hand from each agent's documentation: Cortex's MCP
//! server in the user's settings, beside the user's servers, and a rules file of Cortex's own in
//! the rules folder an agent reads, which the exclude file git never commits leaves out, never the
//! `.gitignore`. It never writes a file the user owns and other agents read (`AGENTS.md`, `.rules`,
//! committed here): it says the lines to add, word for word. Init again changes nothing; doctor
//! finds what init wrote current and says the user's steps; uninstall leaves every file and folder
//! as it was before init, the exclude file included. Init refuses to write over a rules file of
//! the user's, or to make a first file in Roo Code's rules folder while its `.roorules` would be
//! hidden by it.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::init::{agents_lines, cortex, git, run, snapshot};
use common::{StopsDaemon, TestResult};
use serde_json::{Value, json};

/// The rules an agent with Cortex's MCP server is given, typed here by hand.
const MCP_RULES: &str = "# Cortex\n\
\n\
Cortex keeps a map of the code of the repository you work in. When a task needs code you\n\
have not read yet and the `cortex_where` tool (Cortex's MCP server) is available, call it first\n\
with the task in one sentence: it names the files and line spans the task most likely touches,\n\
and why.\n";

/// The rules Aider's model is given, typed here by hand.
const AIDER_RULES: &str = "# Cortex\n\
\n\
Cortex keeps a map of the code of the repository you work in. When a task needs files that\n\
are not in the chat yet, first suggest the shell command `cortex where \"<the task in one sentence>\"`:\n\
it names the files and line spans the task most likely touches, and why.\n";

/// The files the user owns and other agents read, committed in every test's repository, which
/// init never touches.
const SHARED: [(&str, &str); 3] = [
    ("AGENTS.md", "# Team rules\n\nRun the tests.\n"),
    (".rules", "Keep functions short.\n"),
    ("CONVENTIONS.md", "- Use tabs.\n"),
];

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

/// A file init writes: where, and what it holds after.
struct Written {
    /// The folder it is under.
    base: Base,
    /// Its path there, with `/` between folders.
    path: &'static str,
    /// What it holds.
    holds: Holds,
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
    written: fn(&str) -> Vec<Written>,
    /// Exactly what init says the user is to do, each text whole, given this binary's path.
    told: fn(&str) -> Vec<String>,
    /// The lines doctor says beyond those of what is current, given this binary's path.
    doctor_says: fn(&str) -> Vec<String>,
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

/// This binary's path as it knows it, which init writes, with the separators of Windows there.
fn program() -> String {
    let given = env!("CARGO_BIN_EXE_cortex");
    if cfg!(windows) {
        given.replace('/', "\\")
    } else {
        given.to_owned()
    }
}

/// Cortex's MCP entry running `program`, as JSON.
fn entry(program: &str) -> Value {
    json!({"command": program, "args": ["mcp"]})
}

/// Cortex's rules file for the agent `title`, opening with front matter holding `keys`.
fn front_matter_rules(title: &str, keys: &str) -> String {
    format!(
        "---\ndescription: \"Cortex's rules for {title}, written by `cortex init`; `cortex \
         uninstall` removes them\"\n{keys}\n---\n\n{MCP_RULES}"
    )
}

/// Roo Code's rules file, opening with a comment.
fn roo_rules() -> String {
    format!(
        "<!-- Cortex's rules for Roo Code, written by `cortex init`; `cortex uninstall` removes \
         them. -->\n\n{MCP_RULES}"
    )
}

/// Zed's user settings file under the home folder, where Zed reads it on this system.
const ZED_SETTINGS: &str = if cfg!(windows) {
    "AppData/Roaming/Zed/settings.json"
} else {
    ".config/zed/settings.json"
};

/// The Devin configuration folder's MCP file Windsurf's Cascade reads, under the home folder.
const DEVIN_MCP: &str = if cfg!(windows) {
    "AppData/Roaming/devin/mcp_config.json"
} else {
    ".config/devin/mcp_config.json"
};

/// What init tells the user to add for Zed, whole.
fn zed_told() -> String {
    let personal = if cfg!(windows) {
        "%APPDATA%\\Zed\\AGENTS.md"
    } else {
        "~/.config/zed/AGENTS.md"
    };
    format!(
        "Zed: to do yourself: add these lines to the instructions Zed alone reads, {personal}; \
         Cortex does not write it, since it is yours, nor a repository's instructions file, which \
         other agents read too, Codex and Claude Code among them, which get Cortex through hooks \
         and have no `cortex_where` tool:\n{MCP_RULES}"
    )
}

/// What init tells the user to add for Warp, whole.
fn warp_told() -> String {
    format!(
        "Warp: to do yourself: add these lines as a Global Rule in Warp Drive (Personal, Rules, \
         Global, or `/add-rule`), which Warp alone reads, or to a repository's WARP.md where it \
         has one; Cortex writes neither, since they are yours, and not AGENTS.md, which other \
         agents read too, Codex among them, which gets Cortex through hooks and has no \
         `cortex_where` tool:\n{MCP_RULES}"
    )
}

/// The step of adding Cortex's server to Roo Code's global MCP settings by hand, for `program`.
fn roo_mcp(program: &str) -> String {
    format!(
        "add Cortex's MCP server yourself, under `mcpServers` in Roo Code's global MCP settings, \
         which its MCP panel opens with Edit Global MCP (their file is in the editor's storage, at \
         a path Roo Code does not document, so Cortex does not write it): \"cortex\": \
         {{\"command\":{},\"args\":[\"mcp\"]}}",
        json!(program)
    )
}

/// What init tells the user to add to Roo Code's global MCP settings for `program`.
fn roo_mcp_told(program: &str) -> String {
    format!("Roo Code: to do yourself: {}\n", roo_mcp(program))
}

/// Doctor's line saying that step for `program`, after `said` (`yours to do`, or `found; yours to
/// do` for a Roo Code nothing was written for).
fn roo_mcp_step(program: &str, said: &str) -> String {
    format!(
        "ok    agents: Roo Code: {said}, which doctor cannot check: {}",
        roo_mcp(program)
    )
}

/// Zed at the project's scope: Cortex's server after the user's in plain JSON settings, the
/// user's other key kept, and the lines to add said, never written to the committed `.rules` or
/// `AGENTS.md`; doctor says them as the user's.
#[test]
fn init_sets_up_zed() -> TestResult {
    checked(&Case {
        agent: "zed",
        scope: "project",
        before: &[(
            Base::Home,
            ZED_SETTINGS,
            r#"{"theme": "One Dark", "context_servers": {"github": {"command": "gh-mcp", "args": []}}}"#,
        )],
        written: |program| {
            vec![Written {
                base: Base::Home,
                path: ZED_SETTINGS,
                holds: Holds::Json(json!({
                    "theme": "One Dark",
                    "context_servers": {
                        "github": {"command": "gh-mcp", "args": []},
                        "cortex": entry(program)
                    }
                })),
            }]
        },
        told: |_| vec![zed_told()],
        doctor_says: |_| {
            let told = zed_told();
            let first = told
                .lines()
                .next()
                .unwrap_or_default()
                .replace("Zed: to do yourself: ", "");
            vec![format!(
                "ok    agents: Zed: yours to do, which doctor cannot check: {first} `cortex init \
                 --agents zed` prints them"
            )]
        },
    })
}

/// Continue at the project's scope: its rules file beside the user's own in `.continue/rules/`,
/// and Cortex's server in a `cortex.json` of its own in the user's `~/.continue/mcpServers/`.
#[test]
fn init_sets_up_continue_in_the_repository() -> TestResult {
    checked(&Case {
        agent: "continue",
        scope: "project",
        before: &[(
            Base::Repository,
            ".continue/rules/mine.md",
            "---\nname: Mine\n---\n\nUse tabs.\n",
        )],
        written: |program| {
            vec![
                Written {
                    base: Base::Repository,
                    path: ".continue/rules/cortex.md",
                    holds: Holds::Text(front_matter_rules(
                        "Continue",
                        "name: Cortex\nalwaysApply: true",
                    )),
                },
                Written {
                    base: Base::Home,
                    path: ".continue/mcpServers/cortex.json",
                    holds: Holds::Json(json!({"mcpServers": {"cortex": entry(program)}})),
                },
            ]
        },
        told: |_| Vec::new(),
        doctor_says: |_| Vec::new(),
    })
}

/// Continue at the user's scope: its rules file in `~/.continue/rules/`.
#[test]
fn init_sets_up_continue_for_the_user() -> TestResult {
    checked(&Case {
        agent: "continue",
        scope: "user",
        before: &[],
        written: |program| {
            vec![
                Written {
                    base: Base::Home,
                    path: ".continue/rules/cortex.md",
                    holds: Holds::Text(front_matter_rules(
                        "Continue",
                        "name: Cortex\nalwaysApply: true",
                    )),
                },
                Written {
                    base: Base::Home,
                    path: ".continue/mcpServers/cortex.json",
                    holds: Holds::Json(json!({"mcpServers": {"cortex": entry(program)}})),
                },
            ]
        },
        told: |_| Vec::new(),
        doctor_says: |_| Vec::new(),
    })
}

/// Warp: Cortex's server after the user's in `~/.warp/.mcp.json`, and the rules said.
#[test]
fn init_sets_up_warp() -> TestResult {
    checked(&Case {
        agent: "warp",
        scope: "project",
        before: &[(
            Base::Home,
            ".warp/.mcp.json",
            r#"{"mcpServers": {"files": {"command": "npx", "args": ["-y", "files-mcp"]}}}"#,
        )],
        written: |program| {
            vec![Written {
                base: Base::Home,
                path: ".warp/.mcp.json",
                holds: Holds::Json(json!({"mcpServers": {
                    "files": {"command": "npx", "args": ["-y", "files-mcp"]},
                    "cortex": entry(program)
                }})),
            }]
        },
        told: |_| vec![warp_told()],
        doctor_says: |_| {
            let told = warp_told();
            let first = told
                .lines()
                .next()
                .unwrap_or_default()
                .replace("Warp: to do yourself: ", "");
            vec![format!(
                "ok    agents: Warp: yours to do, which doctor cannot check: {first} `cortex init \
                 --agents warp` prints them"
            )]
        },
    })
}

/// Windsurf at the project's scope: its rules file in `.windsurf/rules/`, always on, and Cortex's
/// server in the Devin configuration folder's `mcp_config.json`.
#[test]
fn init_sets_up_windsurf_in_the_repository() -> TestResult {
    checked(&Case {
        agent: "windsurf",
        scope: "project",
        before: &[],
        written: |program| {
            vec![
                Written {
                    base: Base::Repository,
                    path: ".windsurf/rules/cortex.md",
                    holds: Holds::Text(front_matter_rules("Windsurf", "trigger: always_on")),
                },
                Written {
                    base: Base::Home,
                    path: DEVIN_MCP,
                    holds: Holds::Json(json!({"mcpServers": {"cortex": entry(program)}})),
                },
            ]
        },
        told: |_| Vec::new(),
        doctor_says: |_| Vec::new(),
    })
}

/// Windsurf at the user's scope: Cortex's server, and the lines to add to the user's global rules
/// said, since that one file is the user's.
#[test]
fn init_sets_up_windsurf_for_the_user() -> TestResult {
    checked(&Case {
        agent: "windsurf",
        scope: "user",
        before: &[],
        written: |program| {
            vec![Written {
                base: Base::Home,
                path: DEVIN_MCP,
                holds: Holds::Json(json!({"mcpServers": {"cortex": entry(program)}})),
            }]
        },
        told: |_| {
            vec![format!(
                "Windsurf: to do yourself: for every repository, add these lines to Windsurf's \
                 global rules (Customizations, Rules, + Global), a file of yours, which Cortex does \
                 not write:\n{MCP_RULES}"
            )]
        },
        doctor_says: |_| {
            vec![
                "ok    agents: Windsurf: yours to do, which doctor cannot check: for every \
                 repository, add these lines to Windsurf's global rules (Customizations, Rules, + \
                 Global), a file of yours, which Cortex does not write: `cortex init --agents \
                 windsurf` prints them"
                    .to_owned(),
            ]
        },
    })
}

/// Roo Code at the project's scope: its rules file beside the user's own in `.roo/rules/`, whose
/// file keeps the user's `.roorules` hidden already, and the MCP entry to add said.
#[test]
fn init_sets_up_roo_in_the_repository() -> TestResult {
    checked(&Case {
        agent: "roo",
        scope: "project",
        before: &[
            (Base::Repository, ".roo/rules/mine.md", "Use tabs.\n"),
            (Base::Repository, ".roorules", "Old rules.\n"),
        ],
        written: |_| {
            vec![Written {
                base: Base::Repository,
                path: ".roo/rules/cortex.md",
                holds: Holds::Text(roo_rules()),
            }]
        },
        told: |program| vec![roo_mcp_told(program)],
        doctor_says: |program| vec![roo_mcp_step(program, "yours to do")],
    })
}

/// Roo Code at the user's scope: nothing written, since a file in `~/.roo/rules/` would hide each
/// repository's `.roorules`; the rules and the MCP entry to add said.
#[test]
fn init_tells_roo_at_the_user_s_scope() -> TestResult {
    checked(&Case {
        agent: "roo",
        scope: "user",
        before: &[],
        written: |_| Vec::new(),
        told: |program| {
            vec![
                format!(
                    "Roo Code: to do yourself: for every repository, put these lines in a file of \
                     yours in ~/.roo/rules/ (Cortex writes none there, since while that folder \
                     holds a file Roo Code reads no repository's .roorules or .clinerules):\n\
                     {MCP_RULES}"
                ),
                roo_mcp_told(program),
            ]
        },
        doctor_says: |_| Vec::new(),
    })
}

/// Runs `case` through doctor, init, init again, doctor, uninstall and doctor again, as the module
/// says.
fn checked(case: &Case) -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let agent = case.agent;
    with_shared_files(repo.path(), home.path())?;
    let path = |base: Base, relative: &str| path_of(base, relative, repo.path(), home.path());
    let program = program();
    let written = (case.written)(&program);
    for (base, relative, text) in case.before {
        let file = path(*base, relative);
        std::fs::create_dir_all(file.parent().ok_or("no folder")?)?;
        std::fs::write(&file, text)?;
    }
    let before = snapshot(&[repo.path(), home.path()])?;
    let exclude = repo.path().join(".git").join("info").join("exclude");
    let exclude_before = std::fs::read_to_string(&exclude)?;
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(agents_lines(&doctor), Vec::<String>::new(), "{agent}");
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let init = || {
        cortex(
            &["init", "--yes", "--agents", agent, "--scope", case.scope],
            repo.path(),
            home.path(),
        )
    };
    let done = init()?;
    assert!(done.is_success, "{agent}: {}", done.stderr);
    assert_shared_untouched(repo.path())?;
    let mut texts = Vec::new();
    for file in &written {
        let text = std::fs::read_to_string(path(file.base, file.path))?;
        match &file.holds {
            Holds::Text(expected) => assert_eq!(&text, expected, "{agent}: {}", file.path),
            Holds::Json(expected) => assert_eq!(
                &serde_json::from_str::<Value>(&text)?,
                expected,
                "{agent}: {}",
                file.path
            ),
        }
        texts.push(text);
    }
    assert_left_out(repo.path(), &exclude_before, &written)?;
    let said = done.stdout.matches(" written to ").count();
    assert_eq!(said, written.len(), "{}", done.stdout);
    for told in (case.told)(&program) {
        assert!(done.stdout.contains(&told), "{told}\n---\n{}", done.stdout);
    }
    let again = init()?;
    assert!(again.is_success, "{agent}: {}", again.stderr);
    assert!(!again.stdout.contains(" written to "), "{}", again.stdout);
    for (file, text) in written.iter().zip(&texts) {
        assert_eq!(&std::fs::read_to_string(path(file.base, file.path))?, text);
    }
    assert_left_out(repo.path(), &exclude_before, &written)?;
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    let lines = agents_lines(&doctor);
    let (current, others): (Vec<String>, Vec<String>) = lines
        .into_iter()
        .partition(|line| line.contains(" is current in "));
    assert_eq!(current.len(), written.len(), "{current:?}");
    assert!(
        current
            .iter()
            .all(|line| line.starts_with("ok    agents: ")),
        "{current:?}"
    );
    assert_eq!(others, (case.doctor_says)(&program), "{agent}");
    let undone = cortex(&["uninstall", "--scope", "user"], repo.path(), home.path())?;
    assert!(undone.is_success, "{agent}: {}", undone.stderr);
    assert_eq!(snapshot(&[repo.path(), home.path()])?, before, "{agent}");
    assert_eq!(
        std::fs::read_to_string(&exclude)?,
        exclude_before,
        "{agent}"
    );
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(agents_lines(&doctor), Vec::<String>::new(), "{agent}");
    Ok(())
}

/// Makes the repository at `repo`, with `home` as the home folder, and commits [`SHARED`] in it;
/// on Windows also makes `AppData/Roaming` in `home`, which every real profile has.
fn with_shared_files(repo: &Path, home: &Path) -> TestResult {
    with_shared_files_from(repo, home, &["init", "-q"])
}

/// [`with_shared_files`], the repository made by git run with `init`.
fn with_shared_files_from(repo: &Path, home: &Path, init: &[&str]) -> TestResult {
    git(repo, init, home)?;
    for (name, text) in SHARED {
        std::fs::write(repo.join(name), text)?;
    }
    git(
        repo,
        &["add", "--", "AGENTS.md", ".rules", "CONVENTIONS.md"],
        home,
    )?;
    git(
        repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-q",
            "-m",
            "shared rules",
        ],
        home,
    )?;
    if cfg!(windows) {
        std::fs::create_dir_all(home.join("AppData").join("Roaming"))?;
    }
    Ok(())
}

/// Checks that [`SHARED`] hold what they were committed with.
fn assert_shared_untouched(repo: &Path) -> TestResult {
    for (name, text) in SHARED {
        assert_eq!(std::fs::read_to_string(repo.join(name))?, text, "{name}");
    }
    Ok(())
}

/// Checks that each of `written` in the repository is left out of git through the exclude file
/// alone: the exclude file holds what it held before, `exclude_before`, then Cortex's comment and
/// the files' lines, typed here by hand; the `.gitignore` names none of them.
fn assert_left_out(repo: &Path, exclude_before: &str, written: &[Written]) -> TestResult {
    let lines: Vec<&str> = written
        .iter()
        .filter(|file| file.base == Base::Repository)
        .map(|file| file.path)
        .collect();
    let exclude = std::fs::read_to_string(repo.join(".git").join("info").join("exclude"))?;
    let expected = if lines.is_empty() {
        exclude_before.to_owned()
    } else {
        let separator = if exclude_before.is_empty() || exclude_before.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        format!(
            "{exclude_before}{separator}# Cortex\n{}\n",
            lines.join("\n")
        )
    };
    assert_eq!(exclude, expected);
    let ignores = std::fs::read_to_string(repo.join(".gitignore"))?;
    for line in lines {
        assert!(!ignores.lines().any(|ignored| ignored == line), "{ignores}");
    }
    Ok(())
}

/// A rules file of the user's at the path of Cortex's is never written over: init for Continue
/// refuses, says why, and writes nothing at all.
#[test]
fn init_refuses_a_user_s_rules_file_of_the_same_name() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files(repo.path(), home.path())?;
    let rules = repo.path().join(".continue").join("rules");
    std::fs::create_dir_all(&rules)?;
    std::fs::write(
        rules.join("cortex.md"),
        "My own notes on the cortex module.\n",
    )?;
    let before = snapshot(&[repo.path(), home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let refused = cortex(
        &["init", "--yes", "--agents", "continue"],
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

/// Roo Code reads a repository's `.roorules` only while its rules folder holds no file: init
/// refuses to make the first one there, saying why, and writes nothing; with a file of the user's
/// in the folder, the same init writes Cortex's (see [`init_sets_up_roo_in_the_repository`]).
#[test]
fn init_refuses_to_hide_roo_s_rules_file() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files(repo.path(), home.path())?;
    std::fs::write(repo.path().join(".roorules"), "Old rules.\n")?;
    std::fs::create_dir_all(repo.path().join(".roo").join("rules"))?;
    let before = snapshot(&[repo.path(), home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let refused = cortex(
        &["init", "--yes", "--agents", "roo"],
        repo.path(),
        home.path(),
    )?;
    assert!(!refused.is_success, "{}", refused.stdout);
    assert!(
        refused.stderr.contains(".roorules only while ")
            && refused
                .stderr
                .contains("holds no file, so Cortex's rules file there would hide it"),
        "{}",
        refused.stderr
    );
    assert_eq!(snapshot(&[repo.path(), home.path()])?, before);
    Ok(())
}

/// Aider gets nothing written: init says the lines for the user's conventions file, word for
/// word, and leaves the home folder and the shared files as they were; doctor says nothing of
/// Aider until its program is on the `PATH`, then that the steps are the user's.
#[test]
fn init_tells_aider_and_writes_nothing() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files(repo.path(), home.path())?;
    let homes_before = snapshot(&[home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "aider"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(snapshot(&[home.path()])?, homes_before);
    assert_shared_untouched(repo.path())?;
    let told = format!(
        "Aider: to do yourself: add these lines to the conventions file you give Aider \
         (`--read`, or `read:` in .aider.conf.yml); Cortex writes neither, since they are yours, \
         and Aider reads no file of Cortex's own:\n{AIDER_RULES}"
    );
    assert!(done.stdout.contains(&told), "{}", done.stdout);
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(agents_lines(&doctor), Vec::<String>::new());
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin)?;
    if cfg!(windows) {
        std::fs::write(bin.join("aider.cmd"), "@echo aider\r\n")?;
    } else {
        std::fs::write(bin.join("aider"), "#!/bin/sh\necho aider\n")?;
        made_executable(&bin.join("aider"))?;
    }
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(
        agents_lines(&doctor),
        [
            "ok    agents: Aider: found; yours to do, which doctor cannot check: add these lines to \
             the conventions file you give Aider (`--read`, or `read:` in .aider.conf.yml); Cortex \
             writes neither, since they are yours, and Aider reads no file of Cortex's own: \
             `cortex init --agents aider` prints them"
        ]
    );
    Ok(())
}

/// Makes the file at `path` executable, as a program on the `PATH` is.
#[cfg(unix)]
fn made_executable(path: &Path) -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Nothing: Windows runs a program by its name.
#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the Unix version of the same function can fail"
)]
fn made_executable(_: &Path) -> TestResult {
    Ok(())
}

/// Zed's settings with comments are never rewritten: init leaves them byte for byte and says the
/// entry to add, with the rules.
#[test]
fn zed_settings_with_comments_are_told_not_written() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files(repo.path(), home.path())?;
    let settings = path_of(Base::Home, ZED_SETTINGS, repo.path(), home.path());
    std::fs::create_dir_all(settings.parent().ok_or("no folder")?)?;
    let text = "// Zed settings\n{\n  \"theme\": \"One Dark\"\n}\n";
    std::fs::write(&settings, text)?;
    let homes_before = snapshot(&[home.path()])?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "zed"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(snapshot(&[home.path()])?, homes_before);
    let start = format!(
        "Zed: to do yourself: add Cortex's MCP server to {} yourself, under `context_servers`: \
         \"cortex\": {{\"command\":{},\"args\":[\"mcp\"]}}; Cortex leaves that file as it is, \
         since ",
        settings.display(),
        json!(program())
    );
    let told = done
        .stdout
        .lines()
        .find(|line| line.starts_with(&start))
        .ok_or_else(|| done.stdout.clone())?;
    assert!(
        told.ends_with("and it never rewrites a file with comments"),
        "{told}"
    );
    assert!(done.stdout.contains(&zed_told()), "{}", done.stdout);
    Ok(())
}

/// `cortex init --global` sets Continue, Windsurf and Roo Code up in the user's folders alone and
/// says the rest; `cortex uninstall --global` leaves the home folder as it was.
#[test]
fn global_init_sets_up_the_mcp_agents() -> TestResult {
    let home = tempfile::tempdir()?;
    if cfg!(windows) {
        std::fs::create_dir_all(home.path().join("AppData").join("Roaming"))?;
    }
    let before = snapshot(&[home.path()])?;
    let global = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.args(args);
        run(&mut command, home.path(), &["git"])
    };
    let done = global(&[
        "init",
        "--global",
        "--yes",
        "--agents",
        "continue,windsurf,roo",
    ])?;
    assert!(done.is_success, "{}", done.stderr);
    let program = program();
    let read = |relative: &str| {
        std::fs::read_to_string(path_of(Base::Home, relative, home.path(), home.path()))
    };
    assert_eq!(
        read(".continue/rules/cortex.md")?,
        front_matter_rules("Continue", "name: Cortex\nalwaysApply: true")
    );
    for file in [".continue/mcpServers/cortex.json", DEVIN_MCP] {
        assert_eq!(
            serde_json::from_str::<Value>(&read(file)?)?,
            json!({"mcpServers": {"cortex": entry(&program)}}),
            "{file}"
        );
    }
    assert!(!home.path().join(".roo").exists());
    assert!(
        done.stdout.contains(&roo_mcp_told(&program)),
        "{}",
        done.stdout
    );
    assert!(
        done.stdout
            .contains("Windsurf: to do yourself: for every repository, add these lines"),
        "{}",
        done.stdout
    );
    let undone = global(&["uninstall", "--global"])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(snapshot(&[home.path()])?, before);
    Ok(())
}

/// In a linked working tree, whose ignore lines go to the exclude file git keeps for every working
/// tree, the rules file's line joins that block, typed here by hand; uninstall gives the exclude
/// file back byte for byte.
#[test]
fn a_linked_tree_s_rules_file_joins_the_exclude_block() -> TestResult {
    let top = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let repo = top.path().join("main");
    let linked = top.path().join("linked");
    std::fs::create_dir(&repo)?;
    with_shared_files(&repo, home.path())?;
    let spelled = linked.to_str().ok_or("a path that is not UTF-8")?;
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feature", spelled],
        home.path(),
    )?;
    let exclude = repo.join(".git").join("info").join("exclude");
    let before = std::fs::read_to_string(&exclude)?;
    let _daemon = StopsDaemon::of(&repo, home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "continue"],
        &linked,
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(
        std::fs::read_to_string(linked.join(".continue").join("rules").join("cortex.md"))?,
        front_matter_rules("Continue", "name: Cortex\nalwaysApply: true")
    );
    let separator = if before.is_empty() || before.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    assert_eq!(
        std::fs::read_to_string(&exclude)?,
        format!("{before}{separator}# Cortex\n.cortex/\n.continue/rules/cortex.md\n")
    );
    assert!(!linked.join(".gitignore").exists());
    // The repository is the main working tree's, whose runs uninstall undoes.
    let undone = cortex(&["uninstall", "--scope", "user"], &repo, home.path())?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert_eq!(std::fs::read_to_string(&exclude)?, before);
    assert!(!linked.join(".continue").exists());
    Ok(())
}

/// A `.gitignore` that leaves Continue's folder out already gets no exclude line for the rules
/// file: the exclude file stays as it was.
#[test]
fn an_ignored_rules_folder_gets_no_exclude_line() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files(repo.path(), home.path())?;
    std::fs::write(repo.path().join(".gitignore"), ".continue/\n")?;
    let exclude = repo.path().join(".git").join("info").join("exclude");
    let before = std::fs::read_to_string(&exclude)?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "continue"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert!(repo.path().join(".continue/rules/cortex.md").is_file());
    assert_eq!(std::fs::read_to_string(&exclude)?, before);
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".gitignore"))?,
        ".continue/\n# Cortex\n.cortex/\n"
    );
    Ok(())
}

/// A repository made from an empty template has no `.git/info/`: init makes it for the exclude
/// file, and uninstall takes both away again.
#[test]
fn a_missing_info_folder_is_made_and_taken_away() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files_from(repo.path(), home.path(), &["init", "-q", "--template="])?;
    let info = repo.path().join(".git").join("info");
    assert!(!info.exists(), "the premise: git made no info folder");
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "roo"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(
        std::fs::read_to_string(info.join("exclude"))?,
        "# Cortex\n.roo/rules/cortex.md\n"
    );
    let undone = cortex(&["uninstall", "--scope", "user"], repo.path(), home.path())?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert!(!info.exists());
    Ok(())
}

/// A `.clinerules` folder, as Cline's hooks make, is no rules file of Roo Code's to hide: init
/// writes Roo's rules file all the same.
#[test]
fn a_clinerules_folder_does_not_stop_roo() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files(repo.path(), home.path())?;
    let hooks = repo.path().join(".clinerules").join("hooks");
    std::fs::create_dir_all(&hooks)?;
    std::fs::write(hooks.join("notes.md"), "my hooks\n")?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "roo"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".roo/rules/cortex.md"))?,
        roo_rules()
    );
    Ok(())
}

/// A server named `cortex` of another program in Warp's settings is the user's: the file stays
/// byte for byte, and init says the entry to add under `cortex-index`.
#[test]
fn another_program_s_cortex_server_stays_in_warp() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files(repo.path(), home.path())?;
    let settings = home.path().join(".warp").join(".mcp.json");
    std::fs::create_dir_all(settings.parent().ok_or("no folder")?)?;
    let text = r#"{"mcpServers": {"cortex": {"command": "npx", "args": ["other-cortex"]}}}"#;
    std::fs::write(&settings, text)?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = cortex(
        &["init", "--yes", "--agents", "warp"],
        repo.path(),
        home.path(),
    )?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(std::fs::read_to_string(&settings)?, text);
    let told = format!(
        "Warp: to do yourself: add Cortex's MCP server to {} yourself, under `mcpServers`: \
         \"cortex-index\": {{\"command\":{},\"args\":[\"mcp\"]}}; Cortex leaves that file as it \
         is, since its `mcpServers.cortex` is an MCP server of another program\n",
        settings.display(),
        json!(program())
    );
    assert!(done.stdout.contains(&told), "{}", done.stdout);
    Ok(())
}

/// Roo Code found on this machine with nothing written for it here: doctor says each step init
/// says, the rules for the user's folder and the MCP entry to add, as the user's.
#[test]
fn doctor_says_a_found_roo_s_steps() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    with_shared_files(repo.path(), home.path())?;
    let rules = home.path().join(".roo").join("rules");
    std::fs::create_dir_all(&rules)?;
    std::fs::write(rules.join("mine.md"), "Use tabs.\n")?;
    let doctor = cortex(&["doctor"], repo.path(), home.path())?;
    assert_eq!(
        agents_lines(&doctor),
        [
            "ok    agents: Roo Code: found; yours to do, which doctor cannot check: for every \
             repository, put these lines in a file of yours in ~/.roo/rules/ (Cortex writes none \
             there, since while that folder holds a file Roo Code reads no repository's .roorules \
             or .clinerules): `cortex init --agents roo` prints them"
                .to_owned(),
            roo_mcp_step(&program(), "found; yours to do"),
        ]
    );
    Ok(())
}
