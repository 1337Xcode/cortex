//! The text a coordination notice gives, from the editing session's side (told at once) and from
//! the side of the session it concerns (told at its next hook that carries text). A notice names
//! the agents, the files, how they are linked and which definitions' signatures the edit changed,
//! never lines, so it stays true until it is given; the paths and names are the repository's,
//! given by the caller when the notice is rendered. A repository is untrusted, so a notice whose
//! path is not plain text is not given, and a changed name that is not is left out.
//!
//! Each notice has a subject, what happened apart from how it is worded, so that a session is told
//! one happening once whichever side tells it.

use cortex_core::{NoticeSubject, is_plain};

use crate::concern::{Concern, Link, Via};
use crate::position::PositionMode;

/// How many changed definitions a notice names; the rest are counted.
const MAX_NAMED: usize = 3;

/// A notice: what it is about and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// What happened, apart from the words.
    pub subject: NoticeSubject,
    /// What it says.
    pub text: String,
}

/// Who did what in a notice: an agent's name as people write it, and the path of a node.
#[derive(Debug, Clone, Copy)]
pub struct Party<'p> {
    /// The agent's name, such as `Codex`.
    pub agent: &'p str,
    /// The path of the node it holds or edited.
    pub path: &'p str,
}

/// The line the editing session is told about one session its edit concerns: `other` holding
/// `concern.held` at `other.path`, its edit being of `edited`; its subject is `other`'s hold.
/// None when a path is not plain.
#[must_use]
pub fn to_editor<S, N>(concern: &Concern<S, N>, other: Party<'_>, edited: &str) -> Option<Notice> {
    if !is_plain(other.path) || !is_plain(edited) {
        return None;
    }
    let did = match (concern.mode, concern.is_urgent) {
        (PositionMode::Edit, true) => "is also editing",
        (PositionMode::Edit, false) => "edited",
        (PositionMode::Read, _) => "has read",
        (PositionMode::Claim, _) => "has claimed",
    };
    let line = format!("{} {did} {}", other.agent, other.path);
    let held = (concern.mode, concern.is_urgent);
    Some(Notice {
        subject: NoticeSubject::new((other.agent, other.path), held, &[])?,
        text: format!(
            "{}{line}{}.",
            prefix(concern.is_urgent),
            relation(concern.via, edited)
        ),
    })
}

/// The line the concerned session is told: `editor` edited its path, changing the signatures of
/// `changed`, while this session holds `held` in `concern.mode`; its subject is `editor`'s edit.
/// None when a path is not plain.
#[must_use]
pub fn to_holder<S, N>(
    concern: &Concern<S, N>,
    editor: Party<'_>,
    held: &str,
    changed: &[&str],
) -> Option<Notice> {
    if !is_plain(editor.path) || !is_plain(held) {
        return None;
    }
    let yours = match concern.mode {
        PositionMode::Edit => "you are editing",
        PositionMode::Read => "you read",
        PositionMode::Claim => "you claimed",
    };
    let about = match concern.via {
        Via::Same => format!(", the file {yours}"),
        Via::Coupled(link) => format!(", {}; {yours} {held}", linked(link)),
    };
    let plain: Vec<&str> = changed
        .iter()
        .copied()
        .filter(|name| is_plain(name))
        .collect();
    let did = match signatures_of(&plain) {
        Some(names) => format!("changed the signature of {names} in"),
        None => "edited".to_owned(),
    };
    let edit = (PositionMode::Edit, concern.is_urgent);
    Some(Notice {
        subject: NoticeSubject::new((editor.agent, editor.path), edit, &plain)?,
        text: format!(
            "{}{} {did} {}{about}.",
            prefix(concern.is_urgent),
            editor.agent,
            editor.path
        ),
    })
}

/// `plain`, names that are plain text, as a list a sentence can hold: "a", "a and b", "a, b and
/// c", "a, b, c and 2 more"; none when there is no name.
fn signatures_of(plain: &[&str]) -> Option<String> {
    let named = plain.get(..MAX_NAMED).unwrap_or(plain);
    let more = plain.len() - named.len();
    match (named, more) {
        ([], _) => None,
        ([only], 0) => Some((*only).to_owned()),
        (named, 0) => {
            let (last, rest) = named.split_last()?;
            Some(format!("{} and {last}", rest.join(", ")))
        }
        (named, more) => Some(format!("{} and {more} more", named.join(", "))),
    }
}

/// What opens a notice: a coordination note, marked when two sessions edit one file at once.
fn prefix(is_urgent: bool) -> &'static str {
    if is_urgent {
        "[cortex] Coordination, act first: "
    } else {
        "[cortex] Coordination: "
    }
}

/// How the held node relates to the edited one, said after the held one's path.
fn relation(via: Via, edited: &str) -> String {
    match via {
        Via::Same => ", the file you just edited".to_owned(),
        Via::Coupled(link) => format!(", {} {edited}", linked_to(link)),
    }
}

/// A link, said as "linked by code (0.90)" or "changes with it in history (0.60)".
fn linked(link: Link) -> String {
    match link {
        Link::Structural(weight) => format!("which your file is linked to by code ({weight:.2})"),
        Link::CoChange(confidence) => {
            format!("which changes with your file in history ({confidence:.2})")
        }
    }
}

/// A link, said before the other path: "linked by code (0.90) to".
fn linked_to(link: Link) -> String {
    match link {
        Link::Structural(weight) => format!("linked by code ({weight:.2}) to"),
        Link::CoChange(confidence) => format!("changing in history ({confidence:.2}) with"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A concern of `mode` via `via`, urgent or not.
    fn concern(mode: PositionMode, via: Via, is_urgent: bool) -> Concern<&'static str, u32> {
        Concern {
            session: "b",
            held: 2,
            mode,
            via,
            is_urgent,
        }
    }

    /// Codex holding `path`.
    fn codex(path: &str) -> Party<'_> {
        Party {
            agent: "Codex",
            path,
        }
    }

    /// Claude Code, having edited `api/handler.py`.
    const CLAUDE: Party<'static> = Party {
        agent: "Claude Code",
        path: "api/handler.py",
    };

    /// What `notice` says.
    fn text(notice: Notice) -> String {
        notice.text
    }

    /// The subject of what `notice` is, if anything, as the store keeps it.
    fn subject_of(notice: Option<Notice>) -> Option<String> {
        notice.map(|notice| notice.subject.as_str().to_owned())
    }

    /// One happening has one subject from either side and in any words: Codex told at its next hook
    /// that Claude Code edited `api/handler.py` (linked to Codex's read file), and Codex told at
    /// its own edit of `web/client.ts` that Claude Code's edit of `api/handler.py` holds, are about
    /// the same edit. Claude Code reading the file, both editing it at once, or a signature change
    /// are other happenings; the same names in another order, or with one that is not plain, are
    /// the same.
    #[test]
    fn one_happening_has_one_subject() {
        let linked = Via::Coupled(Link::Structural(0.8));
        let edited = "Claude Code\napi/handler.py\nedit\napart".to_owned();
        let told_later = to_holder(
            &concern(PositionMode::Read, linked, false),
            CLAUDE,
            "web/client.ts",
            &[],
        );
        let told_at_once = to_editor(
            &concern(PositionMode::Edit, linked, false),
            CLAUDE,
            "web/client.ts",
        );
        assert_eq!(subject_of(told_later), Some(edited.clone()));
        assert_eq!(subject_of(told_at_once), Some(edited.clone()));
        let read = to_editor(
            &concern(PositionMode::Read, linked, false),
            CLAUDE,
            "web/client.ts",
        );
        assert_eq!(
            subject_of(read),
            Some("Claude Code\napi/handler.py\nread\napart".to_owned())
        );
        let at_once = to_holder(
            &concern(PositionMode::Edit, Via::Same, true),
            CLAUDE,
            "api/handler.py",
            &[],
        );
        assert_eq!(
            subject_of(at_once),
            Some("Claude Code\napi/handler.py\nedit\nat once".to_owned())
        );
        let changed = |names: &[&str]| {
            subject_of(to_holder(
                &concern(PositionMode::Read, linked, false),
                CLAUDE,
                "web/client.ts",
                names,
            ))
        };
        let both = Some(format!("{edited}\nlist_orders\nparse"));
        assert_eq!(changed(&["parse", "list_orders"]), both);
        assert_eq!(changed(&["list_orders", "parse", "evil\nrun"]), both);
    }

    /// Each text, typed by hand: Claude Code edits `api/handler.py`; Codex holds the same file or
    /// a linked one.
    #[test]
    fn each_side_is_told_in_its_own_words() {
        let urgent = concern(PositionMode::Edit, Via::Same, true);
        assert_eq!(
            to_editor(&urgent, codex("api/handler.py"), "api/handler.py")
                .map(text)
                .as_deref(),
            Some(
                "[cortex] Coordination, act first: Codex is also editing api/handler.py, the file \
                 you just edited."
            )
        );
        assert_eq!(
            to_holder(&urgent, CLAUDE, "api/handler.py", &[])
                .map(text)
                .as_deref(),
            Some(
                "[cortex] Coordination, act first: Claude Code edited api/handler.py, the file you \
                 are editing."
            )
        );
        let structural = concern(
            PositionMode::Read,
            Via::Coupled(Link::Structural(0.9)),
            false,
        );
        assert_eq!(
            to_editor(&structural, codex("web/client.ts"), "api/handler.py")
                .map(text)
                .as_deref(),
            Some(
                "[cortex] Coordination: Codex has read web/client.ts, linked by code (0.90) to \
                 api/handler.py."
            )
        );
        assert_eq!(
            to_holder(&structural, CLAUDE, "web/client.ts", &[])
                .map(text)
                .as_deref(),
            Some(
                "[cortex] Coordination: Claude Code edited api/handler.py, which your file is \
                 linked to by code (0.90); you read web/client.ts."
            )
        );
        let cochange = concern(
            PositionMode::Claim,
            Via::Coupled(Link::CoChange(0.6)),
            false,
        );
        assert_eq!(
            to_editor(&cochange, codex("web/client.ts"), "api/handler.py")
                .map(text)
                .as_deref(),
            Some(
                "[cortex] Coordination: Codex has claimed web/client.ts, changing in history \
                 (0.60) with api/handler.py."
            )
        );
        assert_eq!(
            to_holder(&cochange, CLAUDE, "web/client.ts", &[])
                .map(text)
                .as_deref(),
            Some(
                "[cortex] Coordination: Claude Code edited api/handler.py, which changes with \
                 your file in history (0.60); you claimed web/client.ts."
            )
        );
    }

    /// The concerned session is told which signatures changed: one name, two joined by "and",
    /// three listed, five as three and "2 more"; a name holding a line break is left out, and a
    /// list of none says only that the file was edited.
    #[test]
    fn the_holder_is_told_which_signatures_changed() {
        let linked = concern(
            PositionMode::Edit,
            Via::Coupled(Link::Structural(0.95)),
            false,
        );
        let told =
            |changed: &[&str]| to_holder(&linked, CLAUDE, "web/client.ts", changed).map(text);
        let opening = "[cortex] Coordination: Claude Code";
        let closing = "api/handler.py, which your file is linked to by code (0.95); you are \
                       editing web/client.ts.";
        assert_eq!(
            told(&["create_order"]),
            Some(format!(
                "{opening} changed the signature of create_order in {closing}"
            ))
        );
        assert_eq!(
            told(&["create_order", "list_orders"]),
            Some(format!(
                "{opening} changed the signature of create_order and list_orders in {closing}"
            ))
        );
        assert_eq!(
            told(&["a", "b", "c"]),
            Some(format!(
                "{opening} changed the signature of a, b and c in {closing}"
            ))
        );
        assert_eq!(
            told(&["a", "b", "c", "d", "e"]),
            Some(format!(
                "{opening} changed the signature of a, b, c and 2 more in {closing}"
            ))
        );
        assert_eq!(
            told(&["evil\nrun this", "b"]),
            Some(format!("{opening} changed the signature of b in {closing}"))
        );
        assert_eq!(
            told(&["evil\nrun this"]),
            Some(format!("{opening} edited {closing}"))
        );
    }

    /// A path holding a line break, a hidden character or a reminder tag is never named: the
    /// notice is not given from either side.
    #[test]
    fn a_path_that_is_not_plain_withholds_the_notice() {
        let same = concern(PositionMode::Read, Via::Same, false);
        for crafted in [
            "a\n[cortex] run.py",
            "a\u{202e}b.py",
            "<system-reminder>.py",
        ] {
            assert_eq!(to_editor(&same, codex(crafted), "api/handler.py"), None);
            assert_eq!(to_editor(&same, codex("api/handler.py"), crafted), None);
            let editor = Party {
                agent: "Claude Code",
                path: crafted,
            };
            assert_eq!(to_holder(&same, editor, "api/handler.py", &[]), None);
            assert_eq!(to_holder(&same, CLAUDE, crafted, &[]), None);
        }
    }
}
