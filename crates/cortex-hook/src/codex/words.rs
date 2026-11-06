//! A shell command's first pipeline split into the words of its stages, as POSIX shells and
//! PowerShell both read the parts Cortex looks at: the pipeline ends at the first `;`, `&&`, `||`
//! or line break outside quotes, its stages are split at `|`, and a word ends at a space outside
//! quotes; quotes that wrap a word whole are taken off. A redirect of standard error is dropped,
//! since it changes nothing the agent sees on its standard output; any other redirect (standard
//! output to a file, a file to standard input, both streams) makes the pipeline one Cortex does
//! not read, since what the agent saw is not what the command's words say.

/// The stages of `command`'s first pipeline, each as its words; none when a stage redirects
/// anything but standard error.
pub(super) fn stages(command: &str) -> Option<Vec<Vec<&str>>> {
    let mut stages = vec![Vec::new()];
    let mut rest = command;
    let mut is_redirect_target = false;
    loop {
        rest = rest.trim_start_matches([' ', '\t']);
        if rest.is_empty()
            || rest.starts_with(['\n', '\r', ';'])
            || rest.starts_with("&&")
            || rest.starts_with("||")
        {
            return Some(stages);
        }
        if let Some(after) = rest.strip_prefix('|') {
            stages.push(Vec::new());
            rest = after;
            continue;
        }
        let (word, after) = word(rest);
        rest = after;
        if is_redirect_target {
            is_redirect_target = false;
            continue;
        }
        if word.has_redirect {
            // `2>file`, `2>>file`, `2>&1`, `2>$null` are dropped; `2>` and `2>>` alone name their
            // file in the next word, dropped with them.
            if !word.raw.starts_with("2>") {
                return None;
            }
            is_redirect_target = word.raw == "2>" || word.raw == "2>>";
            continue;
        }
        stages.last_mut()?.push(word.text);
    }
}

/// One word of a command.
struct Word<'a> {
    /// The word as written, quotes and all.
    raw: &'a str,
    /// The word without the quotes that wrap it whole.
    text: &'a str,
    /// Whether it holds a `<` or `>` outside quotes: a redirect.
    has_redirect: bool,
}

/// The word `text` starts with, and the text after it: up to a space, a line break, `;`, `|` or
/// `&&` outside quotes.
fn word(text: &str) -> (Word<'_>, &str) {
    let mut quote = None;
    let mut has_redirect = false;
    let mut end = text.len();
    for (at, character) in text.char_indices() {
        let is_end = match (quote, character) {
            (Some(open), close) if open == close => {
                quote = None;
                false
            }
            (None, '\'' | '"') => {
                quote = Some(character);
                false
            }
            (None, ' ' | '\t' | '\n' | '\r' | ';' | '|') => true,
            (None, '&') => text.get(at..).is_some_and(|rest| rest.starts_with("&&")),
            (None, '<' | '>') => {
                has_redirect = true;
                false
            }
            (Some(_) | None, _) => false,
        };
        if is_end {
            end = at;
            break;
        }
    }
    let (raw, after) = text.split_at_checked(end).unwrap_or((text, ""));
    let word = Word {
        raw,
        text: unquote(raw),
        has_redirect,
    };
    (word, after)
}

/// `word` without the quotes that wrap it whole, as in `'a b'`; as it is otherwise.
fn unquote(word: &str) -> &str {
    ['\'', '"']
        .into_iter()
        .find_map(|quote| {
            word.strip_prefix(quote)?
                .strip_suffix(quote)
                .filter(|inner| !inner.contains(quote))
        })
        .unwrap_or(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `stages` as the words of [`stages`] hold them.
    fn owned<'a>(stages: &[&[&'a str]]) -> Vec<Vec<&'a str>> {
        stages.iter().map(|stage| stage.to_vec()).collect()
    }

    /// A pipeline's stages and words: quotes taken off a word they wrap whole, separators inside
    /// quotes kept, the pipeline ended by `;`, `&&`, `||` or a line break.
    #[test]
    fn a_pipeline_is_split_into_its_stages_words() {
        let cases: [(&str, &[&[&str]]); 7] = [
            ("cat calc/ops.py", &[&["cat", "calc/ops.py"]]),
            (
                "rg -n 'a|b; c && d' calc",
                &[&["rg", "-n", "a|b; c && d", "calc"]],
            ),
            (
                "cat a.py | head -n 5",
                &[&["cat", "a.py"], &["head", "-n", "5"]],
            ),
            ("cat a.py; rm -rf build", &[&["cat", "a.py"]]),
            ("cat a.py && python main.py", &[&["cat", "a.py"]]),
            ("cat a.py || true\npython x", &[&["cat", "a.py"]]),
            (
                "rg \"def divide\"  calc\t",
                &[&["rg", "def divide", "calc"]],
            ),
        ];
        for (command, expected) in cases {
            assert_eq!(stages(command), Some(owned(expected)), "{command}");
        }
    }

    /// A redirect of standard error is dropped with its target; a redirect of standard output, of
    /// standard input or of both streams makes the pipeline unread; `<` and `>` inside quotes are
    /// no redirect.
    #[test]
    fn redirects_of_errors_are_dropped_and_others_refused() {
        let dropped: [(&str, &[&[&str]]); 5] = [
            ("rg -n divide 2>/dev/null", &[&["rg", "-n", "divide"]]),
            (
                "rg -n divide 2>&1 | head -50",
                &[&["rg", "-n", "divide"], &["head", "-50"]],
            ),
            ("rg divide 2> errors.txt calc", &[&["rg", "divide", "calc"]]),
            (
                "Select-String divide a.py 2>$null",
                &[&["Select-String", "divide", "a.py"]],
            ),
            ("rg '->' calc", &[&["rg", "->", "calc"]]),
        ];
        for (command, expected) in dropped {
            assert_eq!(stages(command), Some(owned(expected)), "{command}");
        }
        for refused in [
            "rg divide >hits.txt",
            "rg divide > hits.txt",
            "cat <calc/ops.py",
            "cat calc/ops.py>out.py",
            "rg divide &>all.txt",
            "rg divide | head -5 > top.txt",
            "Get-Content a.py *> out.txt",
        ] {
            assert_eq!(stages(refused), None, "{refused}");
        }
    }

    /// An empty command is one empty stage.
    #[test]
    fn an_empty_command_is_one_empty_stage() {
        assert_eq!(stages("").as_deref(), Some(&[Vec::<&str>::new()][..]));
        assert_eq!(
            stages("  ; cat a.py").as_deref(),
            Some(&[Vec::<&str>::new()][..])
        );
    }
}
