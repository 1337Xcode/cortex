//! The names whose definitions came or went in an index, counted per language family, so that the
//! references going by those names elsewhere are tied again only where what a name finds may now
//! differ.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Identifier, Language};

use crate::resolve::family;

/// How many definitions of each name came or went, per language family.
#[derive(Debug, Default, Clone)]
pub(crate) struct NameChanges {
    /// Definitions that came (counted up) or went (counted down), by name and family.
    counted: BTreeMap<(Identifier, Language), i64>,
    /// Names whose family is not known: those of a file whose language changed, whose definitions
    /// may have moved from one family to another.
    moved: BTreeSet<Identifier>,
}

impl NameChanges {
    /// A definition of `name` came, in a file of `language`.
    pub(crate) fn came(&mut self, name: &Identifier, language: Option<Language>) {
        self.count(name, language, 1);
    }

    /// A definition of `name` went, from a file of `language`.
    pub(crate) fn went(&mut self, name: &Identifier, language: Option<Language>) {
        self.count(name, language, -1);
    }

    /// Definitions of `name` came, went or moved between families in ways not counted.
    pub(crate) fn moved(&mut self, name: &Identifier) {
        self.moved.insert(name.clone());
    }

    /// Adds `other`'s changes.
    pub(crate) fn extend(&mut self, other: NameChanges) {
        for ((name, family), delta) in other.counted {
            *self.counted.entry((name, family)).or_default() += delta;
        }
        self.moved.extend(other.moved);
    }

    /// Whether no definition came, went or moved.
    pub(crate) fn is_empty(&self) -> bool {
        self.counted.is_empty() && self.moved.is_empty()
    }

    /// Every name whose definitions changed.
    pub(crate) fn names(&self) -> BTreeSet<&Identifier> {
        self.counted
            .keys()
            .map(|(name, _)| name)
            .chain(&self.moved)
            .collect()
    }

    /// Each name and family whose definitions changed in number, with by how many, and the names
    /// whose family is not known.
    pub(crate) fn counted(
        &self,
    ) -> (
        &BTreeMap<(Identifier, Language), i64>,
        &BTreeSet<Identifier>,
    ) {
        (&self.counted, &self.moved)
    }

    /// Counts `delta` definitions of `name` in the family of `language`; a file in no language
    /// defines nothing.
    fn count(&mut self, name: &Identifier, language: Option<Language>, delta: i64) {
        if let Some(language) = language {
            *self
                .counted
                .entry((name.clone(), family(language)))
                .or_default() += delta;
        }
    }
}

/// Whether references going by a name may now find something else in a family where the name has
/// `after` definitions, `delta` of them having come (or gone, when negative): unless the family has
/// held two or more both before and after, which leaves every such reference unresolved either
/// way. One definition replaced by another counts as a change, since the one found differs.
pub(crate) fn may_find_otherwise(after: i64, delta: i64) -> bool {
    let before = after - delta;
    before < 2 || after < 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two or more definitions before and after leave nothing to tie again; any count of one or
    /// none, before or after, does: none to one, one to two, two to one, and one replaced by
    /// another (one before and after, a definition gone and another come).
    #[test]
    fn only_counts_reaching_one_or_none_matter() {
        let cases = [
            (5_001, 1, false),
            (4_999, -1, false),
            (2, 0, false),
            (1, 1, true),
            (2, 1, true),
            (1, -1, true),
            (1, 0, true),
            (0, -1, true),
            (3, 2, true),
        ];
        for (after, delta, expected) in cases {
            assert_eq!(
                may_find_otherwise(after, delta),
                expected,
                "{after} {delta}"
            );
        }
    }

    /// Changes are counted per family, a file in no language counts nothing, and joined changes
    /// add up.
    #[test]
    fn changes_add_up_per_family() -> Result<(), cortex_core::IdentifierError> {
        let run = Identifier::new("run")?;
        let mut changes = NameChanges::default();
        changes.came(&run, Some(Language::TypeScript));
        changes.came(&run, Some(Language::JavaScript));
        changes.went(&run, Some(Language::Python));
        changes.came(&run, None);
        let mut more = NameChanges::default();
        more.went(&run, Some(Language::Tsx));
        more.moved(&Identifier::new("go")?);
        changes.extend(more);
        let (counted, moved) = changes.counted();
        let counts: Vec<(Language, i64)> = counted
            .iter()
            .map(|((_, family), delta)| (*family, *delta))
            .collect();
        assert_eq!(counts, [(Language::TypeScript, 1), (Language::Python, -1)]);
        assert_eq!(moved.len(), 1);
        assert_eq!(changes.names().len(), 2);
        Ok(())
    }
}
