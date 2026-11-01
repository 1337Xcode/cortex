//! The checks on Cortex's hooks in Claude Code's settings: they are in one file Cortex writes and
//! are what this build writes; no setting turns them off; and they run this binary. Managed
//! settings are an organization's: Cortex reads them for what turns hooks off, and never counts
//! hooks there as its own to write.

use std::collections::BTreeSet;
use std::path::Path;

use cortex_core::ipc::CORTEX_VERSION;

use super::settings::{Scope, SettingsFile};
use super::{Context, Finding};
use crate::claude_settings::{cortex_commands, flag, merged};

/// The settings files a user writes, and what each holds that reads as JSON: managed ones and
/// those that do not exist left out, and one that cannot be read or is not JSON reported in
/// `findings`.
fn readable<'a>(
    context: &'a Context,
    findings: &mut Vec<Finding>,
) -> Vec<(&'a SettingsFile, &'a str)> {
    let mut readable = Vec::new();
    for file in context
        .files
        .iter()
        .filter(|file| file.scope != Scope::Managed)
    {
        match &file.text {
            Err(why) => findings.push(Finding::Fail {
                what: format!("cannot read {}: {why}", file.path.display()),
                fix: format!("make {} readable, or remove it", file.path.display()),
            }),
            Ok(None) => {}
            Ok(Some(text)) => match cortex_commands(text, &context.program) {
                Ok(_) => readable.push((file, text.as_str())),
                Err(error) => findings.push(Finding::Fail {
                    what: format!(
                        "{} is not the JSON Claude Code reads: {error}",
                        file.path.display()
                    ),
                    fix: format!(
                        "repair {}; Claude Code takes none of its settings as it is",
                        file.path.display()
                    ),
                }),
            },
        }
    }
    readable
}

/// Cortex's hooks are in exactly one settings file and are what this build would write there.
pub(super) fn registered(context: &Context) -> Vec<Finding> {
    let mut findings = Vec::new();
    let holding: Vec<(&SettingsFile, &str)> = readable(context, &mut findings)
        .into_iter()
        .filter(|(_, text)| {
            cortex_commands(text, &context.program).is_ok_and(|commands| !commands.is_empty())
        })
        .collect();
    let finding = match holding.as_slice() {
        [] => Finding::Fail {
            what: "Claude Code has no Cortex hook in this repository's settings or the user's"
                .to_owned(),
            fix: init_command(Scope::Local, &context.root),
        },
        [(file, text)] => {
            let adapter = cortex_hook::adapter("claude");
            let expected =
                adapter.map(|adapter| merged(Some(text), &context.program, adapter.hooks()));
            if expected.is_some_and(|expected| expected.is_ok_and(|expected| expected == *text)) {
                Finding::Pass(format!(
                    "Cortex's hooks are current in {}",
                    file.path.display()
                ))
            } else {
                Finding::Fail {
                    what: format!(
                        "Cortex's hooks in {} are not what this build writes: one is missing or \
                         changed, or they run another binary",
                        file.path.display()
                    ),
                    fix: init_command(file.scope, &context.root),
                }
            }
        }
        several => Finding::Fail {
            what: format!(
                "Cortex's hooks are in {}: Claude Code may run each of them once for each file",
                several
                    .iter()
                    .map(|(file, _)| file.path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
            fix: "keep them in one file: remove Cortex's hooks from the others, and run `cortex \
                  init` for the scope you keep"
                .to_owned(),
        },
    };
    findings.push(finding);
    findings
}

/// The command that writes Cortex's hooks where a file of `scope` is, for the repository at
/// `root`.
fn init_command(scope: Scope, root: &Path) -> String {
    match scope {
        Scope::User => "run `cortex init --scope user`".to_owned(),
        Scope::Project => format!(
            "Cortex writes its hooks to .claude/settings.local.json, not to the shared settings: \
             remove them from .claude/settings.json and run `cortex init --root {}`",
            root.display()
        ),
        Scope::Managed | Scope::Local => format!("run `cortex init --root {}`", root.display()),
    }
}

/// No setting Cortex can read keeps Claude Code from running its hooks: `disableAllHooks` as the
/// file of highest precedence that sets it says, and a managed `allowManagedHooksOnly`.
pub(super) fn silenced(context: &Context) -> Vec<Finding> {
    let mut findings = Vec::new();
    // Claude Code takes `disableAllHooks` from the most precedent file that sets it.
    let disabled = context.files.iter().find_map(|file| {
        let value = flag(text_of(file)?, "disableAllHooks").ok()??;
        Some((file, value))
    });
    if let Some((file, true)) = disabled {
        findings.push(Finding::Fail {
            what: format!(
                "disableAllHooks is true in {}, so Claude Code runs no hook",
                file.path.display()
            ),
            fix: match file.scope {
                Scope::Managed => {
                    "ask whoever manages Claude Code on this machine to turn hooks back on"
                        .to_owned()
                }
                _ => format!("remove disableAllHooks from {}", file.path.display()),
            },
        });
    }
    let managed_only = context
        .files
        .iter()
        .filter(|file| file.scope == Scope::Managed)
        .find(|file| {
            text_of(file).is_some_and(|text| {
                flag(text, "allowManagedHooksOnly").is_ok_and(|value| value == Some(true))
            })
        });
    if let Some(file) = managed_only {
        findings.push(Finding::Fail {
            what: format!(
                "allowManagedHooksOnly is true in {}, so Claude Code runs managed hooks alone",
                file.path.display()
            ),
            fix: "ask whoever manages Claude Code on this machine to allow Cortex's hooks"
                .to_owned(),
        });
    }
    if findings.is_empty() {
        findings.push(Finding::Pass(
            "no settings file doctor can read keeps Claude Code from running hooks".to_owned(),
        ));
    }
    findings
}

/// What `file` holds, when it exists and could be read.
fn text_of(file: &SettingsFile) -> Option<&str> {
    file.text.as_ref().ok().and_then(Option::as_deref)
}

/// The hooks run this binary: the command each Cortex hook names is this binary's path. A command
/// that names another program is not run to ask it anything, since a settings file can name any.
pub(super) fn binary(context: &Context) -> Vec<Finding> {
    let mut ignored = Vec::new();
    let commands: BTreeSet<String> = readable(context, &mut ignored)
        .into_iter()
        .filter_map(|(_, text)| cortex_commands(text, &context.program).ok())
        .flatten()
        .collect();
    if commands.is_empty() {
        return vec![Finding::Note(
            "no Cortex hook names a binary to check".to_owned(),
        )];
    }
    commands
        .into_iter()
        .map(|command| {
            if is_same_file(&command, &context.program) {
                Finding::Pass(format!(
                    "the hooks run this binary, {command}, Cortex {CORTEX_VERSION}"
                ))
            } else if !Path::new(&command).exists() {
                Finding::Fail {
                    what: format!("the hooks run {command}, which does not exist"),
                    fix: init_command(Scope::Local, &context.root),
                }
            } else {
                Finding::Fail {
                    what: format!(
                        "the hooks run {command}, not this binary ({}); doctor does not run a \
                         program a settings file names to ask its version",
                        context.program
                    ),
                    fix: "run `cortex init` with the binary the hooks should run".to_owned(),
                }
            }
        })
        .collect()
}

/// Whether `a` and `b` name the same file: the same path once links are followed, or the same
/// text when either cannot be followed.
fn is_same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The binary the tests' hooks run.
    const PROGRAM: &str = "/opt/cortex/bin/cortex";

    /// A context for the repository `/repo` holding `files`, each a scope and its text.
    fn context(files: &[(Scope, &str)]) -> Context {
        Context {
            root: PathBuf::from("/repo"),
            program: PROGRAM.to_owned(),
            files: files
                .iter()
                .map(|(scope, text)| SettingsFile {
                    scope: *scope,
                    path: PathBuf::from(format!("/{scope:?}.json")),
                    text: Ok(Some((*text).to_owned())),
                })
                .collect(),
        }
    }

    /// The kind of each finding, `ok`, `FAIL` or `note`, with its text and fix.
    fn summary(findings: &[Finding]) -> Vec<String> {
        findings
            .iter()
            .map(|finding| match finding {
                Finding::Pass(what) => format!("ok {what}"),
                Finding::Fail { what, fix } => format!("FAIL {what} / {fix}"),
                Finding::Note(what) => format!("note {what}"),
            })
            .collect()
    }

    /// The settings `cortex init` writes for this build, given `existing`.
    fn written(existing: &str) -> anyhow::Result<String> {
        let adapter = cortex_hook::adapter("claude").ok_or_else(|| anyhow::anyhow!("adapter"))?;
        merged(Some(existing), PROGRAM, adapter.hooks())
    }

    /// No hook anywhere fails with `cortex init` as the fix; the hooks init writes, in one file,
    /// pass; the same file with one event's hooks taken out fails, as does a second file holding
    /// them too.
    #[test]
    fn hooks_are_in_one_file_and_current() -> anyhow::Result<()> {
        let none = registered(&context(&[(Scope::Local, "{}")]));
        assert_eq!(
            summary(&none),
            [
                "FAIL Claude Code has no Cortex hook in this repository's settings or the user's / \
              run `cortex init --root /repo`"
            ]
        );
        let current = written("{}")?;
        let ok = registered(&context(&[(Scope::Local, &current)]));
        assert_eq!(
            summary(&ok),
            ["ok Cortex's hooks are current in /Local.json"]
        );
        let mut value: serde_json::Value = serde_json::from_str(&current)?;
        if let Some(hooks) = value
            .get_mut("hooks")
            .and_then(serde_json::Value::as_object_mut)
        {
            hooks.retain(|event, _| event != "Stop");
        }
        let drifted = registered(&context(&[(Scope::User, &serde_json::to_string(&value)?)]));
        assert_eq!(
            summary(&drifted),
            [
                "FAIL Cortex's hooks in /User.json are not what this build writes: one is missing \
              or changed, or they run another binary / run `cortex init --scope user`"
            ]
        );
        let twice = registered(&context(&[
            (Scope::Local, &current),
            (Scope::User, &current),
        ]));
        assert_eq!(
            summary(&twice),
            [
                "FAIL Cortex's hooks are in /Local.json and /User.json: Claude Code may run each of \
              them once for each file / keep them in one file: remove Cortex's hooks from the \
              others, and run `cortex init` for the scope you keep"
            ]
        );
        Ok(())
    }

    /// `disableAllHooks` counts as the most precedent file that sets it says: true in the user's
    /// file fails, unless the repository's local file sets it false; a managed `true` fails with
    /// the administrator as the fix, as does a managed `allowManagedHooksOnly`.
    #[test]
    fn a_setting_that_turns_hooks_off_fails() {
        let off = r#"{"disableAllHooks": true}"#;
        let on = r#"{"disableAllHooks": false}"#;
        assert_eq!(
            summary(&silenced(&context(&[(Scope::User, off)]))),
            [
                "FAIL disableAllHooks is true in /User.json, so Claude Code runs no hook / remove \
              disableAllHooks from /User.json"
            ]
        );
        assert_eq!(
            summary(&silenced(&context(&[
                (Scope::Local, on),
                (Scope::User, off)
            ]))),
            ["ok no settings file doctor can read keeps Claude Code from running hooks"]
        );
        let managed = silenced(&context(&[
            (
                Scope::Managed,
                r#"{"allowManagedHooksOnly": true, "disableAllHooks": true}"#,
            ),
            (Scope::Local, on),
        ]));
        assert_eq!(
            summary(&managed),
            [
                "FAIL disableAllHooks is true in /Managed.json, so Claude Code runs no hook / ask \
                 whoever manages Claude Code on this machine to turn hooks back on",
                "FAIL allowManagedHooksOnly is true in /Managed.json, so Claude Code runs managed \
                 hooks alone / ask whoever manages Claude Code on this machine to allow Cortex's \
                 hooks"
            ]
        );
    }

    /// Hooks that run this binary pass; hooks that name a program that does not exist fail with
    /// `cortex init` as the fix; with no hook there is nothing to check.
    #[test]
    fn the_hooks_run_this_binary() -> anyhow::Result<()> {
        let current = written("{}")?;
        assert_eq!(
            summary(&binary(&context(&[(Scope::Local, &current)]))),
            [format!(
                "ok the hooks run this binary, {PROGRAM}, Cortex {CORTEX_VERSION}"
            )]
        );
        let elsewhere = current.replace(PROGRAM, "/gone/cortex");
        assert_eq!(
            summary(&binary(&context(&[(Scope::Local, &elsewhere)]))),
            [
                "FAIL the hooks run /gone/cortex, which does not exist / run `cortex init --root \
              /repo`"
            ]
        );
        assert_eq!(
            summary(&binary(&context(&[(Scope::Local, "{}")]))),
            ["note no Cortex hook names a binary to check"]
        );
        Ok(())
    }
}
