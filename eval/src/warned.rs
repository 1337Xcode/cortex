//! The warnings Cortex gave a run's tasks after their edits, by kind, and how many were resolved:
//! the file a warning named was read or edited later in its task. The share of the labelled
//! warnings that were resolved is the warnings' precision.

use std::collections::BTreeMap;

use cortex_core::WarningKind;
use serde::{Deserialize, Serialize};

/// How many warnings of one kind were given, labelled at their task's end, and resolved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Warned {
    /// The files warnings of this kind named.
    pub(crate) given: u64,
    /// Of those, the ones labelled when their task ended.
    pub(crate) labelled: u64,
    /// Of those, the ones read or edited after the warning.
    pub(crate) resolved: u64,
}

impl Warned {
    /// The sum of `self` and `other`.
    #[must_use]
    pub(crate) fn add(self, other: Self) -> Self {
        Self {
            given: self.given + other.given,
            labelled: self.labelled + other.labelled,
            resolved: self.resolved + other.resolved,
        }
    }
}

/// The warnings of a run, by the name of their kind.
pub(crate) type Warnings = BTreeMap<String, Warned>;

/// The warnings of `rows`, each its kind and its label (none until its task ended), by kind.
pub(crate) fn warnings_of(rows: impl IntoIterator<Item = (WarningKind, Option<bool>)>) -> Warnings {
    let mut warnings = Warnings::new();
    for (kind, resolved) in rows {
        let held = warnings.entry(kind.name().to_owned()).or_default();
        *held = held.add(Warned {
            given: 1,
            labelled: u64::from(resolved.is_some()),
            resolved: u64::from(resolved == Some(true)),
        });
    }
    warnings
}

/// `warnings` and `more` added together, kind by kind.
pub(crate) fn add_warnings(mut warnings: Warnings, more: &Warnings) -> Warnings {
    for (kind, warned) in more {
        let held = warnings.entry(kind.clone()).or_default();
        *held = held.add(*warned);
    }
    warnings
}

/// All the kinds of `warnings` together.
#[must_use]
pub(crate) fn all_kinds(warnings: &Warnings) -> Warned {
    warnings
        .values()
        .copied()
        .fold(Warned::default(), Warned::add)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counted by kind, worked by hand: two call-site warnings, one resolved and one not, and a
    /// test warning whose task has not ended give call sites 2 given, 2 labelled, 1 resolved, and
    /// tests 1 given, 0 labelled; added to another run's resolved co-change warning, the total is
    /// 4 given, 3 labelled, 2 resolved.
    #[test]
    fn warnings_are_counted_by_kind() {
        let warnings = warnings_of([
            (WarningKind::CallSite, Some(true)),
            (WarningKind::CallSite, Some(false)),
            (WarningKind::Test, None),
        ]);
        let warned = |given, labelled, resolved| Warned {
            given,
            labelled,
            resolved,
        };
        assert_eq!(
            warnings,
            Warnings::from([
                ("call_site".to_owned(), warned(2, 2, 1)),
                ("test".to_owned(), warned(1, 0, 0)),
            ])
        );
        let both = add_warnings(
            warnings,
            &warnings_of([(WarningKind::CoChange, Some(true))]),
        );
        assert_eq!(both.get("cochange"), Some(&warned(1, 1, 1)));
        assert_eq!(all_kinds(&both), warned(4, 3, 2));
    }
}
