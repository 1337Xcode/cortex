//! The kinds of warning Cortex gives after an edit, with the names storage keeps them by.

use crate::named::named_kind;

named_kind! {
    /// What a warning after an edit points out.
    WarningKind {
        /// A file that usually changes with the edited one has not been touched in the task.
        CoChange => "cochange",
        /// A symbol's signature changed and code in other files still calls it.
        CallSite => "call_site",
        /// The edited file has tests the task has not touched.
        Test => "test",
        /// The edited lines came, before the edit, from a commit that names an issue or touched a
        /// test: a note of where they came from, naming no other file.
        Origin => "origin",
    }
}

impl WarningKind {
    /// Whether a warning of this kind names a file the task has yet to read or edit, so that one
    /// left unread counts as open and its follow-up as its precision. An origin note names only the
    /// file just edited and asks nothing of the task.
    #[must_use]
    pub fn expects_follow_up(self) -> bool {
        self != Self::Origin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names storage keeps, which never change once written.
    #[test]
    fn names_are_fixed() {
        let names: Vec<&str> = WarningKind::ALL.iter().map(|kind| kind.name()).collect();
        assert_eq!(names, ["cochange", "call_site", "test", "origin"]);
        assert_eq!(
            WarningKind::from_name("call_site"),
            Some(WarningKind::CallSite)
        );
        assert_eq!(WarningKind::from_name("coordination"), None);
    }

    /// Every kind but the origin note asks the task to read or edit the file it names.
    #[test]
    fn only_the_origin_note_expects_no_follow_up() {
        let expecting: Vec<WarningKind> = WarningKind::ALL
            .iter()
            .copied()
            .filter(|kind| kind.expects_follow_up())
            .collect();
        assert_eq!(
            expecting,
            [
                WarningKind::CoChange,
                WarningKind::CallSite,
                WarningKind::Test
            ]
        );
    }
}
