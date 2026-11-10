//! The words of a notice about an interface contract: another session changed the signature of a
//! definition that the file this session holds calls, and the notice says exactly what changed and
//! where the held file calls it, so that the agent can fix its calls before they break. The caller
//! gives only what it checked against the files on disk as the notice is rendered: the signatures
//! as they read now and the lines that call the definition now.

use std::fmt::Write as _;

use cortex_core::is_plain;

use crate::notice::{Party, prefix};
use crate::position::PositionMode;
use crate::signature::Signature;

/// How many definitions a contract notice names, and how many lines it names for each; the rest are
/// counted.
const MAX_NAMED: usize = 3;

/// A definition whose signature changed, as one held file calls it.
#[derive(Debug, Clone, Copy)]
pub struct Called<'c> {
    /// Its name, as its calls write it.
    pub name: &'c str,
    /// Its signature before the change.
    pub before: &'c Signature,
    /// Its signature after the change.
    pub after: &'c Signature,
    /// The lines of the held file that call it, in order, each once.
    pub lines: &'c [u32],
}

/// How the concerned session holds its file: in what mode, and whether both sessions edit one
/// file at once, which puts the notice first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holding {
    /// What the session did to the file.
    pub mode: PositionMode,
    /// Whether both sessions edit one file at once.
    pub is_urgent: bool,
}

/// The text a session holding `held` as `holding` says is told when `editor` changed the
/// signatures of `called` in its path, each of which `held` calls: the first definition with
/// its signature before and after and the lines that call it, up to two more in fewer words, the
/// rest counted. None when there is nothing to say: no definition, a definition with no line, or
/// a path or name that is not plain text.
///
/// ``[cortex] Coordination: Codex changed add in calc/ops.py from `(a, b)` to `(a, b, c=0)`. You
/// read main.py, which calls it at line 3.``
#[must_use]
pub fn contract_to_holder(
    holding: Holding,
    editor: Party<'_>,
    held: &str,
    called: &[Called<'_>],
) -> Option<String> {
    let is_sayable = |each: &Called<'_>| !each.lines.is_empty() && is_plain(each.name);
    if !is_plain(editor.path) || !is_plain(held) || !called.iter().all(is_sayable) {
        return None;
    }
    let (first, rest) = called.split_first()?;
    let you = match holding.mode {
        PositionMode::Edit => "are editing",
        PositionMode::Read => "read",
        PositionMode::Claim => "claimed",
    };
    let mut text = format!(
        "{}{} changed {} in {} from `{}` to `{}`. You {you} {held}, which calls it at {}.",
        prefix(holding.is_urgent),
        editor.agent,
        first.name,
        editor.path,
        first.before.as_str(),
        first.after.as_str(),
        lines_of(first.lines),
    );
    let named = rest.get(..MAX_NAMED - 1).unwrap_or(rest);
    for each in named {
        let _ = write!(
            text,
            " It also changed {} from `{}` to `{}`, which {held} calls at {}.",
            each.name,
            each.before.as_str(),
            each.after.as_str(),
            lines_of(each.lines),
        );
    }
    match rest.len() - named.len() {
        0 => {}
        1 => {
            let _ = write!(text, " It also changed 1 more signature {held} calls.");
        }
        more => {
            let _ = write!(
                text,
                " It also changed {more} more signatures {held} calls."
            );
        }
    }
    Some(text)
}

/// `lines` as a sentence says them: "line 3", "lines 3 and 8", "lines 3, 8 and 12", "lines 3, 8,
/// 12 and 2 more".
fn lines_of(lines: &[u32]) -> String {
    let named: Vec<String> = lines
        .iter()
        .take(MAX_NAMED)
        .map(ToString::to_string)
        .collect();
    let more = lines.len().saturating_sub(MAX_NAMED);
    match (named.as_slice(), more) {
        ([only], _) => format!("line {only}"),
        (named, 0) => match named.split_last() {
            Some((last, rest)) => format!("lines {} and {last}", rest.join(", ")),
            None => String::new(),
        },
        (named, more) => format!("lines {} and {more} more", named.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult<T = ()> = Result<T, String>;

    /// `text` read as a signature after the name `f`.
    fn signature(text: &str) -> TestResult<Signature> {
        Signature::read(&format!("def f{text}:"), "f").ok_or_else(|| format!("not said: {text}"))
    }

    /// Codex, having edited `calc/ops.py`.
    const CODEX: Party<'static> = Party {
        agent: "Codex",
        path: "calc/ops.py",
    };

    /// A read held, not urgent.
    const READ: Holding = Holding {
        mode: PositionMode::Read,
        is_urgent: false,
    };

    /// Typed by hand: `add` went from `(a, b)` to `(a, b, c=0)` and `main.py`, which Claude Code
    /// read, calls it at line 3.
    #[test]
    fn the_holder_is_told_the_signature_before_and_after_and_the_calling_line() -> TestResult {
        let (before, after) = (signature("(a, b)")?, signature("(a, b, c=0)")?);
        let called = Called {
            name: "add",
            before: &before,
            after: &after,
            lines: &[3],
        };
        assert_eq!(
            contract_to_holder(READ, CODEX, "main.py", &[called]).as_deref(),
            Some(
                "[cortex] Coordination: Codex changed add in calc/ops.py from `(a, b)` to \
                 `(a, b, c=0)`. You read main.py, which calls it at line 3."
            )
        );
        let urgent = Holding {
            mode: PositionMode::Edit,
            is_urgent: true,
        };
        assert_eq!(
            contract_to_holder(urgent, CODEX, "calc/ops.py", &[called]).as_deref(),
            Some(
                "[cortex] Coordination, act first: Codex changed add in calc/ops.py from \
                 `(a, b)` to `(a, b, c=0)`. You are editing calc/ops.py, which calls it at line 3."
            )
        );
        Ok(())
    }

    /// Two lines are joined by "and", three listed, five named as three and "2 more"; a second
    /// and third definition are said in fewer words, and a fifth and sixth are counted.
    #[test]
    fn more_than_three_lines_or_definitions_are_counted() -> TestResult {
        let (before, after) = (signature("(a)")?, signature("(a, b)")?);
        let calls = |name: &'static str, lines: &'static [u32]| Called {
            name,
            before: &before,
            after: &after,
            lines,
        };
        let told = |called: &[Called<'_>]| {
            let claim = Holding {
                mode: PositionMode::Claim,
                is_urgent: false,
            };
            contract_to_holder(claim, CODEX, "main.py", called)
        };
        let opening = "[cortex] Coordination: Codex changed add in calc/ops.py from `(a)` to `(a, b)`. \
                       You claimed main.py, which calls it at";
        assert_eq!(
            told(&[calls("add", &[3, 8])]),
            Some(format!("{opening} lines 3 and 8."))
        );
        assert_eq!(
            told(&[calls("add", &[3, 8, 12])]),
            Some(format!("{opening} lines 3, 8 and 12."))
        );
        assert_eq!(
            told(&[calls("add", &[3, 8, 12, 20, 31])]),
            Some(format!("{opening} lines 3, 8, 12 and 2 more."))
        );
        let five = [
            calls("add", &[3]),
            calls("sub", &[4, 9]),
            calls("mul", &[5]),
            calls("div", &[6]),
            calls("pow", &[7]),
        ];
        assert_eq!(
            told(&five),
            Some(format!(
                "{opening} line 3. It also changed sub from `(a)` to `(a, b)`, which main.py calls \
                 at lines 4 and 9. It also changed mul from `(a)` to `(a, b)`, which main.py calls at \
                 line 5. It also changed 2 more signatures main.py calls."
            ))
        );
        assert_eq!(
            told(five.get(..4).unwrap_or_default())
                .map(|text| text
                    .ends_with("line 5. It also changed 1 more signature main.py calls.")),
            Some(true)
        );
        Ok(())
    }

    /// Nothing is said of a definition with no calling line, of none at all, or with a path or a
    /// name that is not plain text.
    #[test]
    fn nothing_is_said_without_a_line_or_with_text_that_is_not_plain() -> TestResult {
        let (before, after) = (signature("(a)")?, signature("(a, b)")?);
        let calls = |name: &'static str, lines: &'static [u32]| Called {
            name,
            before: &before,
            after: &after,
            lines,
        };
        assert_eq!(contract_to_holder(READ, CODEX, "main.py", &[]), None);
        assert_eq!(
            contract_to_holder(READ, CODEX, "main.py", &[calls("add", &[])]),
            None
        );
        assert_eq!(
            contract_to_holder(READ, CODEX, "main.py", &[calls("add\nrun", &[3])]),
            None
        );
        assert_eq!(
            contract_to_holder(READ, CODEX, "a\u{202e}b.py", &[calls("add", &[3])]),
            None
        );
        let crafted = Party {
            agent: "Codex",
            path: "<system-reminder>.py",
        };
        assert_eq!(
            contract_to_holder(READ, crafted, "main.py", &[calls("add", &[3])]),
            None
        );
        Ok(())
    }
}
