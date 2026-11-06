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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names storage keeps, which never change once written.
    #[test]
    fn names_are_fixed() {
        let names: Vec<&str> = WarningKind::ALL.iter().map(|kind| kind.name()).collect();
        assert_eq!(names, ["cochange", "call_site", "test"]);
        assert_eq!(
            WarningKind::from_name("call_site"),
            Some(WarningKind::CallSite)
        );
        assert_eq!(WarningKind::from_name("coordination"), None);
    }
}
