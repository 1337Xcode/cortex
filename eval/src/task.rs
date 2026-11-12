//! An evaluation task, as `eval/tasks/<id>.toml` writes it: the testbed it runs in, the sessions an
//! agent works through in turn, the check that says whether it succeeded, the files a correct
//! solution touches, the project rules a run may note before its first session, and the
//! regression, if any, that breaks the copy before its sessions begin.
//!
//! Every session but the last is a warm-up, work in the area that makes it familiar, as a
//! developer's earlier days would. The last is the measured one, a new session in the familiar
//! area: only its work orders count for precision.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use cortex_core::FactKind;
use serde::Deserialize;

/// One task.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Task {
    /// Its id: its file's name without `.toml`.
    pub(crate) id: String,
    /// The testbed it runs in, by name.
    pub(crate) testbed: String,
    /// What kind of work it is, for the report.
    pub(crate) kind: String,
    /// The prompts of the warm-up sessions, one session each, in order.
    #[serde(default)]
    pub(crate) warmups: Vec<String>,
    /// The prompt of the measured session.
    pub(crate) prompt: String,
    /// The success check: a program and its arguments, run in the task's folder after every
    /// session; exit code 0 is success.
    pub(crate) check: Vec<String>,
    /// The files a correct solution touches, repository-relative with `/`.
    pub(crate) touches: Vec<String>,
    /// The tools the agent may use without asking, as Claude Code's `--allowedTools` takes them.
    pub(crate) tools: Vec<String>,
    /// The project rules a run notes with `cortex note` before its first session, when its arm
    /// notes them: rules the code at the edit does not show.
    #[serde(default)]
    pub(crate) notes: Vec<Note>,
    /// The name of a file in the suite's `regressions` folder holding a unified diff: a change
    /// committed into the copy, as earlier history, before Cortex is installed, so that the task
    /// starts from a broken state its agent's own test run shows.
    #[serde(default)]
    pub(crate) regression: Option<String>,
    /// Whether the task needs the repository's git history as it stood at the pinned commit: its
    /// copy is then a clone cut at that commit, not a copy of the files made a repository of one
    /// commit.
    #[serde(default)]
    pub(crate) history: bool,
}

/// A project rule, as `cortex note` takes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Note {
    /// The rule.
    pub(crate) text: String,
    /// What it is about, each a file, a file, `:` and a symbol it defines, or `.` for the whole
    /// repository.
    pub(crate) at: Vec<String>,
    /// What kind of rule it is, by the name `cortex note --kind` takes.
    pub(crate) kind: String,
}

impl Note {
    /// The arguments of the `cortex note` command that notes it.
    pub(crate) fn args(&self) -> Vec<String> {
        let mut args = vec!["note".to_owned(), self.text.clone()];
        for anchor in &self.at {
            args.extend(["--at".to_owned(), anchor.clone()]);
        }
        args.extend(["--kind".to_owned(), self.kind.clone()]);
        args
    }
}

impl Task {
    /// The task in `text`, read from the file `file`, checked.
    ///
    /// # Errors
    ///
    /// Text that is not a task, an id that is not the file's name, or a task missing its prompt,
    /// check, files or tools.
    pub(crate) fn parse(text: &str, file: &Path) -> anyhow::Result<Self> {
        let task: Self =
            toml::from_str(text).with_context(|| format!("reading the task {}", file.display()))?;
        let stem = file.file_stem().and_then(|stem| stem.to_str());
        if stem != Some(task.id.as_str()) {
            bail!("the task {} has the id {}", file.display(), task.id);
        }
        let blank = |text: &str| text.trim().is_empty();
        if blank(&task.prompt) || task.warmups.iter().any(|prompt| blank(prompt)) {
            bail!("the task {} has an empty prompt", task.id);
        }
        if task.check.first().is_none_or(|program| blank(program)) {
            bail!("the task {} has no check", task.id);
        }
        if task.touches.is_empty() || task.tools.is_empty() {
            bail!("the task {} names no file it touches or no tool", task.id);
        }
        if let Some(path) = task.touches.iter().find(|path| !is_repository_path(path)) {
            bail!("the task {} touches {path}, not a repository path", task.id);
        }
        for note in &task.notes {
            check_note(&task.id, note)?;
        }
        if let Some(name) = task
            .regression
            .as_deref()
            .filter(|name| !is_file_name(name))
        {
            bail!(
                "the task {} has the regression {name}, not a file name in the regressions folder",
                task.id
            );
        }
        Ok(task)
    }

    /// The file holding the task's regression in `folder`, the suite's regressions folder; none
    /// for a task that starts from the testbed as it is.
    pub(crate) fn regression_in(&self, folder: &Path) -> Option<PathBuf> {
        self.regression.as_ref().map(|name| folder.join(name))
    }

    /// Refuses the task when its regression names a file that is not in `folder`, the suite's
    /// regressions folder.
    ///
    /// # Errors
    ///
    /// The regression's file is missing.
    pub(crate) fn check_regression(&self, folder: &Path) -> anyhow::Result<()> {
        match self.regression_in(folder) {
            Some(path) if !path.is_file() => bail!(
                "the task {} names the regression {}, which is not a file",
                self.id,
                path.display()
            ),
            _ => Ok(()),
        }
    }
}

/// Whether `path` is a path inside the repository, written with `/`.
fn is_repository_path(path: &str) -> bool {
    !(path.contains('\\') || path.starts_with('/') || path.contains(".."))
}

/// Whether `name` is the name of a file directly in a folder: no separator, no drive, not empty and
/// not a way up.
fn is_file_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != ".."
}

/// Refuses `note` of task `id` when it has no text, is about nothing, names a path outside the
/// repository or a kind `cortex note` does not take.
///
/// # Errors
///
/// What is wrong with the note.
fn check_note(id: &str, note: &Note) -> anyhow::Result<()> {
    if note.text.trim().is_empty() || note.at.is_empty() {
        bail!("the task {id} has a note with no text or about nothing");
    }
    let path_of = |anchor: &str| {
        anchor
            .split_once(':')
            .map_or(anchor, |(path, _)| path)
            .to_owned()
    };
    if let Some(anchor) = note
        .at
        .iter()
        .find(|anchor| anchor.as_str() != "." && !is_repository_path(&path_of(anchor)))
    {
        bail!("the task {id} has a note about {anchor}, not a repository path");
    }
    if FactKind::from_name(&note.kind).is_none() {
        bail!(
            "the task {id} has a note of the kind {}, which cortex note does not take",
            note.kind
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task as a file writes it.
    const TASK: &str = "id = \"fix-tax\"\ntestbed = \"shop\"\nkind = \"fix a failing test\"\n\
                        warmups = [\"Read shop/billing/tax.py and explain it.\"]\n\
                        prompt = \"The tax test fails; fix shop/billing/tax.py.\"\n\
                        check = [\"python\", \"-m\", \"unittest\", \"-q\"]\n\
                        touches = [\"shop/billing/tax.py\"]\n\
                        tools = [\"Read\", \"Edit\"]\n";

    /// A task file is read as written; its id must be its file's name.
    #[test]
    fn a_task_is_read_as_written() -> anyhow::Result<()> {
        let task = Task::parse(TASK, Path::new("tasks/fix-tax.toml"))?;
        assert_eq!(task.id, "fix-tax");
        assert_eq!(task.warmups, ["Read shop/billing/tax.py and explain it."]);
        assert_eq!(task.check, ["python", "-m", "unittest", "-q"]);
        assert_eq!(task.touches, ["shop/billing/tax.py"]);
        assert!(Task::parse(TASK, Path::new("tasks/other.toml")).is_err());
        Ok(())
    }

    /// A task missing its check, naming a path outside the repository, or with an unknown key is
    /// refused.
    #[test]
    fn a_broken_task_is_refused() {
        let file = Path::new("fix-tax.toml");
        let no_check = TASK.replace(
            "check = [\"python\", \"-m\", \"unittest\", \"-q\"]",
            "check = []",
        );
        assert!(Task::parse(&no_check, file).is_err());
        let outside = TASK.replace("\"shop/billing/tax.py\"]", "\"../secret.py\"]");
        assert!(Task::parse(&outside, file).is_err());
        let unknown = format!("{TASK}extra = 1\n");
        assert!(Task::parse(&unknown, file).is_err());
    }

    /// A task's notes are read as written, and become the arguments of `cortex note`: the text,
    /// `--at` before each anchor, then `--kind`. A task without notes has none.
    #[test]
    fn notes_are_read_and_become_note_commands() -> anyhow::Result<()> {
        let text = format!(
            "{TASK}[[notes]]\ntext = \"Tax rates live in rates.toml.\"\n\
             at = [\"shop/billing/tax.py:Tax\", \".\"]\nkind = \"convention\"\n"
        );
        let task = Task::parse(&text, Path::new("fix-tax.toml"))?;
        let [note] = &task.notes[..] else {
            anyhow::bail!("one note: {:?}", task.notes);
        };
        assert_eq!(
            note.args(),
            [
                "note",
                "Tax rates live in rates.toml.",
                "--at",
                "shop/billing/tax.py:Tax",
                "--at",
                ".",
                "--kind",
                "convention"
            ]
        );
        assert_eq!(Task::parse(TASK, Path::new("fix-tax.toml"))?.notes, []);
        Ok(())
    }

    /// A task's regression is read as written and found in the regressions folder; a task without
    /// one has none, and a regression whose file is missing is refused.
    #[test]
    fn a_regression_is_read_and_found_in_its_folder() -> anyhow::Result<()> {
        let file = Path::new("fix-tax.toml");
        let text = format!("{TASK}regression = \"tax-rounding.patch\"\n");
        let task = Task::parse(&text, file)?;
        assert_eq!(task.regression.as_deref(), Some("tax-rounding.patch"));
        assert_eq!(Task::parse(TASK, file)?.regression, None);

        let folder = tempfile::tempdir()?;
        assert!(task.check_regression(folder.path()).is_err());
        std::fs::write(folder.path().join("tax-rounding.patch"), "a diff\n")?;
        task.check_regression(folder.path())?;
        assert_eq!(
            task.regression_in(folder.path()),
            Some(folder.path().join("tax-rounding.patch"))
        );
        let plain = Task::parse(TASK, file)?;
        plain.check_regression(folder.path())?;
        assert_eq!(plain.regression_in(folder.path()), None);
        Ok(())
    }

    /// A task needs history only when it says so: the field is read as written, left out it is
    /// false, and a value that is not a boolean is refused.
    #[test]
    fn a_task_says_whether_it_needs_history() -> anyhow::Result<()> {
        let file = Path::new("fix-tax.toml");
        assert!(!Task::parse(TASK, file)?.history);
        assert!(Task::parse(&format!("{TASK}history = true\n"), file)?.history);
        assert!(!Task::parse(&format!("{TASK}history = false\n"), file)?.history);
        assert!(Task::parse(&format!("{TASK}history = \"yes\"\n"), file).is_err());
        Ok(())
    }

    /// A regression that is not a plain file name, so that it could leave the regressions folder,
    /// is refused.
    #[test]
    fn a_regression_outside_its_folder_is_refused() {
        let file = Path::new("fix-tax.toml");
        for name in [
            "../tax.patch",
            "..",
            ".",
            "sub/tax.patch",
            "sub\\\\tax.patch",
            "/etc/tax.patch",
            "C:tax.patch",
            "",
        ] {
            let text = format!("{TASK}regression = \"{name}\"\n");
            assert!(Task::parse(&text, file).is_err(), "{name}");
        }
    }

    /// A note with no text, about nothing, about a path outside the repository, of a kind
    /// `cortex note` does not take, or with an unknown key is refused.
    #[test]
    fn a_broken_note_is_refused() {
        let file = Path::new("fix-tax.toml");
        for note in [
            "text = \" \"\nat = [\"a.py\"]\nkind = \"convention\"\n",
            "text = \"x\"\nat = []\nkind = \"convention\"\n",
            "text = \"x\"\nat = [\"../a.py:A\"]\nkind = \"convention\"\n",
            "text = \"x\"\nat = [\"a.py\"]\nkind = \"rule\"\n",
            "text = \"x\"\nat = [\"a.py\"]\nkind = \"convention\"\nsource = \"user\"\n",
        ] {
            let text = format!("{TASK}[[notes]]\n{note}");
            assert!(Task::parse(&text, file).is_err(), "{note}");
        }
    }
}
