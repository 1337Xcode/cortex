//! The questions `cortex init` asks once: measure Cortex's effect against a holdout of tasks (yes
//! by default), share facts with the team through a committed `.cortex/shared/` (no), and allow
//! optional upkeep by a language model (no). Each answer is kept where it takes effect: the holdout
//! in the repository's `.cortex/config.toml` (or the user's own, when the user set it there), the
//! sharing in the repository's `.gitignore` (a team that shares has committed it), and the upkeep,
//! which reaches the network, in the user's own `~/.cortex/config.toml`, the one file a repository
//! cannot write. A question answered there is not asked again. `--yes` takes the defaults;
//! otherwise questions go to stderr and answers come a line at a time from a terminal. Without one
//! init refuses rather than wait on input that may never come.

use std::io::{BufRead, Read as _};

use anyhow::{Context as _, bail};
use cortex_core::{ConfigLayer, DEFAULT_HOLDOUT_SHARE, config_keys_set, load_config};

use crate::output;

/// One question: what is asked, and the answer an empty line gives.
struct Question {
    /// What is asked.
    text: &'static str,
    /// The answer an empty line gives.
    default: bool,
}

/// Measuring Cortex's effect against a holdout of tasks.
const HOLDOUT: Question = Question {
    text: "Measure Cortex's effect: leave one task in ten without its help, to compare with?",
    default: true,
};

/// Sharing facts with the team.
const SHARING: Question = Question {
    text: "Share facts with your team through .cortex/shared/, committed with the repository?",
    default: false,
};

/// Optional upkeep by a language model.
const LLM_UPKEEP: Question = Question {
    text: "Allow optional upkeep by a language model, with an API key you provide, proposing \
           facts for you to confirm?",
    default: false,
};

/// How many answers that read as neither yes nor no a question takes before init gives up.
const ATTEMPTS: usize = 3;

/// The most of one answer read: a word.
const MAX_ANSWER_BYTES: u64 = 1_024;

/// The key that keeps the holdout answer.
const HOLDOUT_KEY: &str = "holdout.share";

/// The key that keeps the upkeep answer.
const LLM_UPKEEP_KEY: &str = "upkeep.llm.enabled";

/// The answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Answers {
    /// Measure against a holdout.
    pub(super) holdout: bool,
    /// Share facts with the team.
    pub(super) sharing: bool,
    /// Allow optional upkeep by a language model.
    pub(super) llm_upkeep: bool,
}

/// The answers already kept, `None` for each question not answered yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Kept {
    /// Measure against a holdout.
    holdout: Option<bool>,
    /// Share facts with the team.
    sharing: Option<bool>,
    /// Allow optional upkeep by a language model.
    llm_upkeep: Option<bool>,
}

/// A configuration file as init read it.
#[derive(Debug, Clone, Copy)]
pub(super) struct ConfigFile<'a> {
    /// Its text, or `None` when there is none.
    pub(super) text: Option<&'a str>,
    /// Its path, for messages.
    pub(super) origin: &'a str,
    /// Whether it is the user's own.
    pub(super) is_users: bool,
}

impl ConfigFile<'_> {
    /// The file as a configuration layer, when it exists.
    fn layer(&self) -> Option<ConfigLayer<'_>> {
        self.text.map(|text| ConfigLayer {
            origin: self.origin,
            text,
            is_users: self.is_users,
        })
    }

    /// Whether the file sets `key`.
    ///
    /// # Errors
    ///
    /// The file is not valid TOML.
    fn sets(&self, key: &str) -> anyhow::Result<bool> {
        let Some(layer) = self.layer() else {
            return Ok(false);
        };
        Ok(config_keys_set(&layer)?.iter().any(|set| set == key))
    }
}

/// The answers kept in the user's own configuration `users`, the repository's `repository`, and,
/// for `sharing`, the repository's ignores.
///
/// # Errors
///
/// A configuration is not valid TOML or gives a key an unusable value.
pub(super) fn kept(
    users: ConfigFile<'_>,
    repository: ConfigFile<'_>,
    sharing: Option<bool>,
) -> anyhow::Result<Kept> {
    let layers: Vec<ConfigLayer<'_>> = [users.layer(), repository.layer()]
        .into_iter()
        .flatten()
        .collect();
    let config = load_config(&layers)?.config;
    let is_holdout_set = users.sets(HOLDOUT_KEY)? || repository.sets(HOLDOUT_KEY)?;
    Ok(Kept {
        holdout: is_holdout_set.then_some(config.holdout_share > 0.0),
        sharing,
        llm_upkeep: users
            .sets(LLM_UPKEEP_KEY)?
            .then_some(config.is_llm_upkeep_enabled),
    })
}

/// The answers: those `kept`, and for the rest the defaults when `is_yes`, or else each asked on
/// stderr and answered by a line of `terminal`.
///
/// # Errors
///
/// A question is left to ask with no terminal, an answer cannot be read, the terminal's input ends
/// before one comes, or none of [`ATTEMPTS`] answers reads as yes or no.
pub(super) fn answers(
    kept: Kept,
    is_yes: bool,
    mut terminal: Option<&mut impl BufRead>,
) -> anyhow::Result<Answers> {
    let mut answer = |kept: Option<bool>, question: &Question| match (kept, terminal.as_mut()) {
        (Some(answer), _) => Ok(answer),
        (None, _) if is_yes => Ok(question.default),
        (None, Some(terminal)) => asked(question, terminal),
        (None, None) => bail!(
            "init asks \"{}\" in a terminal, and has none: run it in one, or pass --yes to take \
             the defaults",
            question.text
        ),
    };
    Ok(Answers {
        holdout: answer(kept.holdout, &HOLDOUT)?,
        sharing: answer(kept.sharing, &SHARING)?,
        llm_upkeep: answer(kept.llm_upkeep, &LLM_UPKEEP)?,
    })
}

/// The answer to `question`, asked on stderr and read as a line of `input`.
///
/// # Errors
///
/// The answer cannot be read, `input` ends first, or none of [`ATTEMPTS`] answers reads as yes or
/// no.
fn asked(question: &Question, input: &mut impl BufRead) -> anyhow::Result<bool> {
    let choices = if question.default { "[Y/n]" } else { "[y/N]" };
    for _ in 0..ATTEMPTS {
        output::ask(&format!("{} {choices}", question.text))?;
        let mut line = String::new();
        let read = input
            .by_ref()
            .take(MAX_ANSWER_BYTES)
            .read_line(&mut line)
            .context("reading an answer")?;
        if read == 0 {
            bail!("no answer to \"{}\": the input ended", question.text);
        }
        if let Some(answer) = parsed(&line, question.default) {
            return Ok(answer);
        }
    }
    bail!("no answer to \"{}\" reads as yes or no", question.text)
}

/// The answer a line gives: an empty one the default, `y` or `yes` yes, `n` or `no` no, in any
/// case; anything else none.
fn parsed(line: &str, default: bool) -> Option<bool> {
    match line.trim().to_ascii_lowercase().as_str() {
        "" => Some(default),
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

/// The repository's configuration `file` with the holdout answer added at its end, unless it was
/// kept.
///
/// # Errors
///
/// The configuration with the answer added does not load (see [`appended`]).
pub(super) fn repository_config(
    file: ConfigFile<'_>,
    kept: Kept,
    answers: Answers,
) -> anyhow::Result<Option<String>> {
    if kept.holdout.is_some() {
        return Ok(None);
    }
    let share = if answers.holdout {
        DEFAULT_HOLDOUT_SHARE
    } else {
        0.0
    };
    appended(file, HOLDOUT_KEY, &format!("[holdout]\nshare = {share}")).map(Some)
}

/// The user's own configuration `file` with the upkeep answer added at its end, unless it was
/// kept.
///
/// # Errors
///
/// The configuration with the answer added does not load (see [`appended`]).
pub(super) fn users_config(
    file: ConfigFile<'_>,
    kept: Kept,
    answers: Answers,
) -> anyhow::Result<Option<String>> {
    if kept.llm_upkeep.is_some() {
        return Ok(None);
    }
    let table = format!("[upkeep.llm]\nenabled = {}", answers.llm_upkeep);
    appended(file, LLM_UPKEEP_KEY, &table).map(Some)
}

/// The text of `file` (empty when there is none) with the lines of `table`, which sets `key`,
/// added at its end after a comment, in the file's own line ending.
///
/// # Errors
///
/// The result does not load, as when the file has a table of the same name without the key:
/// Cortex does not rewrite a table the user wrote, and says to set the key by hand.
fn appended(file: ConfigFile<'_>, key: &str, table: &str) -> anyhow::Result<String> {
    let before = file.text.unwrap_or_default();
    let ending = if before.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut text = before.to_owned();
    if !text.is_empty() {
        if !text.ends_with('\n') {
            text.push_str(ending);
        }
        text.push_str(ending);
    }
    let comment = "# The answer to a question `cortex init` asked; change it here.";
    for line in std::iter::once(comment).chain(table.lines()) {
        text.push_str(line);
        text.push_str(ending);
    }
    load_config(&[ConfigLayer {
        origin: file.origin,
        text: &text,
        is_users: file.is_users,
    }])
    .with_context(|| {
        format!(
            "adding an answer to {}: set `{key}` there by hand",
            file.origin
        )
    })?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// Nothing kept.
    const NONE_KEPT: Kept = Kept {
        holdout: None,
        sharing: None,
        llm_upkeep: None,
    };

    /// No file.
    const NO_FILE: ConfigFile<'static> = ConfigFile {
        text: None,
        origin: "f",
        is_users: false,
    };

    /// Empty lines take the defaults (yes, no, no); `y`, `NO` and ` yes ` read in any case and
    /// spacing; a line that is neither is asked again; with `--yes` nothing is read.
    #[test]
    fn answers_are_lines_or_defaults() -> anyhow::Result<()> {
        let defaults = answers(NONE_KEPT, false, Some(&mut Cursor::new("\n\n\n")))?;
        let expected = Answers {
            holdout: true,
            sharing: false,
            llm_upkeep: false,
        };
        assert_eq!(defaults, expected);
        let given = answers(
            NONE_KEPT,
            false,
            Some(&mut Cursor::new("NO\nmaybe\n yes \ny\n")),
        )?;
        assert_eq!(
            given,
            Answers {
                holdout: false,
                sharing: true,
                llm_upkeep: true,
            }
        );
        assert_eq!(
            answers(NONE_KEPT, true, None::<&mut Cursor<&str>>)?,
            expected
        );
        Ok(())
    }

    /// Kept answers are not asked: with all three kept, nothing is read, terminal or not; with a
    /// question left and no terminal, init refuses naming it; input that ends before an answer,
    /// or three answers that are neither yes nor no, refuse too.
    #[test]
    fn kept_answers_are_not_asked_and_missing_ones_refuse() {
        let all = Kept {
            holdout: Some(false),
            sharing: Some(true),
            llm_upkeep: Some(false),
        };
        assert!(matches!(
            answers(all, false, None::<&mut Cursor<&str>>),
            Ok(Answers {
                holdout: false,
                sharing: true,
                llm_upkeep: false
            })
        ));
        let no_terminal = answers(NONE_KEPT, false, None::<&mut Cursor<&str>>);
        assert!(
            no_terminal.as_ref().is_err_and(|error| error
                .to_string()
                .starts_with("init asks \"Measure Cortex's effect")),
            "{no_terminal:?}"
        );
        let ended = answers(NONE_KEPT, false, Some(&mut Cursor::new("y\n")));
        assert!(
            ended.as_ref().is_err_and(|error| error.to_string()
                == "no answer to \"Share facts with your team through .cortex/shared/, committed \
                    with the repository?\": the input ended"),
            "{ended:?}"
        );
        let unread = answers(NONE_KEPT, false, Some(&mut Cursor::new("a\nb\nc\n")));
        assert!(
            unread
                .as_ref()
                .is_err_and(|error| error.to_string().ends_with("reads as yes or no")),
            "{unread:?}"
        );
    }

    /// The holdout is kept by either file, the user's winning; the upkeep only by the user's: a
    /// repository's that sets it keeps nothing, and its value is not taken.
    #[test]
    fn answers_are_kept_where_they_may_be() -> anyhow::Result<()> {
        let users = ConfigFile {
            text: Some("[holdout]\nshare = 0\n"),
            origin: "user",
            is_users: true,
        };
        let repository = ConfigFile {
            text: Some("[upkeep.llm]\nenabled = true\n"),
            origin: "repo",
            is_users: false,
        };
        assert_eq!(
            kept(users, repository, None)?,
            Kept {
                holdout: Some(false),
                sharing: None,
                llm_upkeep: None,
            }
        );
        let users = ConfigFile {
            text: Some("upkeep.llm.enabled = true\n"),
            ..users
        };
        let repository = ConfigFile {
            text: Some("holdout.share = 0.1\n"),
            ..repository
        };
        assert_eq!(
            kept(users, repository, Some(true))?,
            Kept {
                holdout: Some(true),
                sharing: Some(true),
                llm_upkeep: Some(true),
            }
        );
        Ok(())
    }

    /// An answer not kept goes at the end of its file under its table, after the user's text, in
    /// the file's own line ending; a kept one is not written again; a file with its own `[holdout]`
    /// table refuses, naming the key to set by hand.
    #[test]
    fn answers_are_added_at_the_end() -> anyhow::Result<()> {
        let answers = Answers {
            holdout: true,
            sharing: false,
            llm_upkeep: false,
        };
        let file = ConfigFile {
            text: Some("mode = \"small\""),
            ..NO_FILE
        };
        assert_eq!(
            repository_config(file, NONE_KEPT, answers)?.as_deref(),
            Some(
                "mode = \"small\"\n\n# The answer to a question `cortex init` asked; change it \
                 here.\n[holdout]\nshare = 0.1\n"
            )
        );
        let crlf = ConfigFile {
            text: Some("mode = \"small\"\r\n"),
            is_users: true,
            ..NO_FILE
        };
        assert_eq!(
            users_config(crlf, NONE_KEPT, answers)?.as_deref(),
            Some(
                "mode = \"small\"\r\n\r\n# The answer to a question `cortex init` asked; change \
                 it here.\r\n[upkeep.llm]\r\nenabled = false\r\n"
            )
        );
        let kept = Kept {
            holdout: Some(true),
            sharing: Some(false),
            llm_upkeep: Some(false),
        };
        assert_eq!(repository_config(NO_FILE, kept, answers)?, None);
        assert_eq!(users_config(NO_FILE, kept, answers)?, None);
        let own_table = ConfigFile {
            text: Some("[holdout]\n"),
            ..NO_FILE
        };
        let refused = repository_config(own_table, NONE_KEPT, answers);
        assert!(
            refused.as_ref().is_err_and(|error| error.to_string()
                == "adding an answer to f: set `holdout.share` there by hand"),
            "{refused:?}"
        );
        Ok(())
    }
}
