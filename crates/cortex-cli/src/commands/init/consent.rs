//! The questions `cortex init` asks once: measure Cortex's effect against a holdout of tasks (yes
//! by default), and share facts with the team through a committed `.cortex/shared/` (no). Each
//! answer is kept where it takes effect: the holdout in the repository's `.cortex/config.toml` (or
//! the user's own `~/.cortex/config.toml`, when the user set it there or ran `cortex init
//! --global`), and the sharing in the repository's `.gitignore` (a team that shares has committed
//! it). `--global` asks the holdout alone: sharing is decided for one repository at a time, by
//! `cortex init` there. A question answered there is not asked again. `--yes` takes the defaults; otherwise questions go to stderr and answers come a
//! line at a time from a terminal. Without one init refuses rather than wait on input that may
//! never come.

use std::io::{BufRead, Read as _};

use anyhow::{Context as _, bail};
use cortex_core::{
    ConfigLayer, DEFAULT_HOLDOUT_SHARE, config_keys_set, load_config, setting_value, with_setting,
};

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

/// How many answers that read as neither yes nor no a question takes before init gives up.
const ATTEMPTS: usize = 3;

/// The most of one answer read: a word.
const MAX_ANSWER_BYTES: u64 = 1_024;

/// The comment above an answer init writes into a configuration file.
const ANSWER_NOTE: &str = "The answer to a question `cortex init` asked; change it here.";

/// The key that keeps the holdout answer.
const HOLDOUT_KEY: &str = "holdout.share";

/// The answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Answers {
    /// Measure against a holdout.
    pub(super) holdout: bool,
    /// Share facts with the team.
    pub(super) sharing: bool,
}

/// The answers already kept, `None` for each question not answered yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Kept {
    /// Measure against a holdout.
    holdout: Option<bool>,
    /// Share facts with the team.
    sharing: Option<bool>,
}

/// A configuration file as init read it.
#[derive(Debug, Clone, Copy)]
pub(super) struct ConfigFile<'a> {
    /// Its text, or `None` when there is none.
    pub(super) text: Option<&'a str>,
    /// Its path, for messages.
    pub(super) origin: &'a str,
}

impl ConfigFile<'_> {
    /// The file as a configuration layer, when it exists.
    fn layer(&self) -> Option<ConfigLayer<'_>> {
        self.text.map(|text| ConfigLayer {
            origin: self.origin,
            text,
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

/// The configuration `file`, the repository's or the user's, with the holdout answer added,
/// unless it was kept.
///
/// # Errors
///
/// The answer cannot be added (see [`answered`]).
pub(super) fn with_answers(
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
    answered(file, HOLDOUT_KEY, &share.to_string()).map(Some)
}

/// The text of `file` (empty when there is none) with `key` set to the value `value` writes, by
/// the configuration's own editor: under the file's table for it, else in a new table at its end,
/// after a comment saying init wrote it, in the file's own line ending, every other line kept.
///
/// # Errors
///
/// The editor cannot place the key without changing anything else the file says, as when the
/// file sets its table inline: Cortex does not rewrite what the user wrote, and says to set the
/// key by hand.
fn answered(file: ConfigFile<'_>, key: &str, value: &str) -> anyhow::Result<String> {
    let layer = ConfigLayer {
        origin: file.origin,
        text: file.text.unwrap_or_default(),
    };
    with_setting(&layer, key, &setting_value(value), Some(ANSWER_NOTE)).with_context(|| {
        format!(
            "adding an answer to {}: set `{key}` there by hand",
            file.origin
        )
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// Nothing kept.
    const NONE_KEPT: Kept = Kept {
        holdout: None,
        sharing: None,
    };

    /// No file.
    const NO_FILE: ConfigFile<'static> = ConfigFile {
        text: None,
        origin: "f",
    };

    /// Empty lines take the defaults (yes, no); `NO` and ` yes ` read in any case and spacing; a
    /// line that is neither is asked again; with `--yes` nothing is read.
    #[test]
    fn answers_are_lines_or_defaults() -> anyhow::Result<()> {
        let defaults = answers(NONE_KEPT, false, Some(&mut Cursor::new("\n\n")))?;
        let expected = Answers {
            holdout: true,
            sharing: false,
        };
        assert_eq!(defaults, expected);
        let given = answers(
            NONE_KEPT,
            false,
            Some(&mut Cursor::new("NO\nmaybe\n yes \n")),
        )?;
        assert_eq!(
            given,
            Answers {
                holdout: false,
                sharing: true,
            }
        );
        assert_eq!(
            answers(NONE_KEPT, true, None::<&mut Cursor<&str>>)?,
            expected
        );
        Ok(())
    }

    /// Kept answers are not asked: with both kept, nothing is read, terminal or not; with a
    /// question left and no terminal, init refuses naming it; input that ends before an answer,
    /// or three answers that are neither yes nor no, refuse too.
    #[test]
    fn kept_answers_are_not_asked_and_missing_ones_refuse() {
        let all = Kept {
            holdout: Some(false),
            sharing: Some(true),
        };
        assert!(matches!(
            answers(all, false, None::<&mut Cursor<&str>>),
            Ok(Answers {
                holdout: false,
                sharing: true,
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

    /// The holdout is kept by either file, the user's winning over the default and the
    /// repository's over the user's; the sharing comes from the ignores as given.
    #[test]
    fn answers_are_kept_where_they_may_be() -> anyhow::Result<()> {
        let users = ConfigFile {
            text: Some("[holdout]\nshare = 0\n"),
            origin: "user",
        };
        assert_eq!(
            kept(users, NO_FILE, None)?,
            Kept {
                holdout: Some(false),
                sharing: None,
            }
        );
        let repository = ConfigFile {
            text: Some("holdout.share = 0.1\n"),
            origin: "repo",
        };
        assert_eq!(
            kept(users, repository, Some(true))?,
            Kept {
                holdout: Some(true),
                sharing: Some(true),
            }
        );
        assert_eq!(
            kept(NO_FILE, NO_FILE, Some(false))?,
            Kept {
                holdout: None,
                sharing: Some(false),
            }
        );
        Ok(())
    }

    /// An answer not kept goes at the end of its file under its table, after the user's text, in
    /// the file's own line ending, or under the file's own `[holdout]` table; a kept one is not
    /// written again; a file setting its `holdout` table inline refuses, naming the key to set by
    /// hand.
    #[test]
    fn answers_are_added_at_the_end() -> anyhow::Result<()> {
        let answers = Answers {
            holdout: true,
            sharing: false,
        };
        let file = ConfigFile {
            text: Some("mode = \"small\""),
            ..NO_FILE
        };
        assert_eq!(
            with_answers(file, NONE_KEPT, answers)?.as_deref(),
            Some(
                "mode = \"small\"\n\n# The answer to a question `cortex init` asked; change it \
                 here.\n[holdout]\nshare = 0.1\n"
            )
        );
        let crlf = ConfigFile {
            text: Some("mode = \"small\"\r\n"),
            ..NO_FILE
        };
        let declined = Answers {
            holdout: false,
            ..answers
        };
        assert_eq!(
            with_answers(crlf, NONE_KEPT, declined)?.as_deref(),
            Some(
                "mode = \"small\"\r\n\r\n# The answer to a question `cortex init` asked; change \
                 it here.\r\n[holdout]\r\nshare = 0\r\n"
            )
        );
        let kept = Kept {
            holdout: Some(true),
            sharing: Some(false),
        };
        assert_eq!(with_answers(NO_FILE, kept, answers)?, None);
        let own_table = ConfigFile {
            text: Some("[holdout]\n"),
            ..NO_FILE
        };
        assert_eq!(
            with_answers(own_table, NONE_KEPT, answers)?.as_deref(),
            Some(
                "[holdout]\n# The answer to a question `cortex init` asked; change it here.\n\
                 share = 0.1\n"
            )
        );
        let inline_table = ConfigFile {
            text: Some("holdout = { other = 1 }\n"),
            ..NO_FILE
        };
        let refused = with_answers(inline_table, NONE_KEPT, answers);
        assert!(
            refused.as_ref().is_err_and(|error| error.to_string()
                == "adding an answer to f: set `holdout.share` there by hand"),
            "{refused:?}"
        );
        Ok(())
    }
}
