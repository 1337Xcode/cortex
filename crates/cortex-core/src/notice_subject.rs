//! What a coordination notice is about, apart from its words. One happening told from either side
//! of an edit, or in other words, has one subject, and a session is told it once: the subject is
//! what the store keeps beside a notice and what the daemon compares before telling a line.

use crate::PositionMode;

/// What separates the parts of a subject: no plain text holds a line break, so no part does.
const PART_BREAK: &str = "\n";

/// What a notice is about: the agent as the notice names it, the file, what the agent did there,
/// whether it did so at once with the session told, and the signatures it changed there, sorted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoticeSubject(String);

impl NoticeSubject {
    /// The subject of a notice saying that `agent` holds `path` in `mode`, at once with the session
    /// told when `is_at_once`, having changed the signatures of `changed`. None when a part holds a
    /// line break, which would make two subjects one.
    #[must_use]
    pub fn new(
        (agent, path): (&str, &str),
        (mode, is_at_once): (PositionMode, bool),
        changed: &[&str],
    ) -> Option<Self> {
        let mut names = changed.to_vec();
        names.sort_unstable();
        names.dedup();
        let when = if is_at_once { "at once" } else { "apart" };
        let parts: Vec<&str> = [agent, path, mode.name(), when]
            .into_iter()
            .chain(names)
            .collect();
        if parts.iter().any(|part| part.contains(PART_BREAK)) {
            return None;
        }
        Some(Self(parts.join(PART_BREAK)))
    }

    /// A subject as the store kept it.
    #[must_use]
    pub fn from_stored(text: String) -> Self {
        Self(text)
    }

    /// The subject as the store keeps it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether a session told of this subject has nothing to learn from a notice about `later`:
    /// the same subject, or the same hold with fewer of the changed signatures, those listed first
    /// here. A notice that the agent edited a file says nothing new after one naming the
    /// signatures it changed there; one naming a signature not named before does.
    #[must_use]
    pub fn covers(&self, later: &Self) -> bool {
        self.0 == later.0
            || self
                .0
                .strip_prefix(&later.0)
                .is_some_and(|rest| rest.starts_with(PART_BREAK))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codex's edit of `calc/ops.py`, apart from the session told, changing `changed`.
    fn edit(changed: &[&str]) -> Option<NoticeSubject> {
        NoticeSubject::new(
            ("Codex", "calc/ops.py"),
            (PositionMode::Edit, false),
            changed,
        )
    }

    /// The parts, worked by hand: the agent, the path, the mode, whether at once, then the names
    /// sorted and each once.
    #[test]
    fn a_subject_is_its_parts_in_order() {
        assert_eq!(
            edit(&["parse", "add", "parse"])
                .as_ref()
                .map(NoticeSubject::as_str),
            Some("Codex\ncalc/ops.py\nedit\napart\nadd\nparse")
        );
        let at_once = NoticeSubject::new(("Claude Code", "a.py"), (PositionMode::Read, true), &[]);
        assert_eq!(
            at_once.as_ref().map(NoticeSubject::as_str),
            Some("Claude Code\na.py\nread\nat once")
        );
        assert_eq!(edit(&["evil\nname"]), None);
    }

    /// A subject covers itself and the same hold naming fewer signatures, those it lists first; not
    /// a hold naming one it does not, another mode, or a path that only starts like its own.
    #[test]
    fn a_subject_covers_what_says_no_more() -> Result<(), &'static str> {
        let both = edit(&["add", "parse"]).ok_or("both")?;
        let plain = edit(&[]).ok_or("plain")?;
        let add = edit(&["add"]).ok_or("add")?;
        let parse = edit(&["parse"]).ok_or("parse")?;
        assert!(both.covers(&both));
        assert!(both.covers(&plain));
        assert!(both.covers(&add));
        assert!(!both.covers(&parse));
        assert!(!plain.covers(&add));
        let read = NoticeSubject::new(("Codex", "calc/ops.py"), (PositionMode::Read, false), &[])
            .ok_or("read")?;
        assert!(!read.covers(&plain));
        let longer =
            NoticeSubject::new(("Codex", "calc/ops.pyc"), (PositionMode::Edit, false), &[])
                .ok_or("longer")?;
        assert!(!longer.covers(&plain));
        Ok(())
    }
}
