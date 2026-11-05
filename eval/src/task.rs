//! An evaluation task, as `eval/tasks/<id>.toml` writes it: the testbed it runs in, the sessions an
//! agent works through in turn, the check that says whether it succeeded, and the files a correct
//! solution touches.
//!
//! Every session but the last is a warm-up, work in the area that makes it familiar, as a
//! developer's earlier days would. The last is the measured one, a new session in the familiar
//! area: only its work orders count for precision.

use std::path::Path;

use anyhow::{Context, bail};
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
        if let Some(path) = task
            .touches
            .iter()
            .find(|path| path.contains('\\') || path.starts_with('/') || path.contains(".."))
        {
            bail!("the task {} touches {path}, not a repository path", task.id);
        }
        Ok(task)
    }
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
}
