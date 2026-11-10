//! The names a fact is stored with: what kind of rule it is, where it came from, and where it
//! stands. A fact is one short project rule anchored to the files or symbols it is about; whether
//! it is injected depends on its status alone.

use crate::named::named_kind;

/// The most characters a fact's text may hold: one short rule, never a passage of code.
pub const MAX_FACT_CHARS: usize = 240;

/// The most files and symbols one fact may be about: a rule about more than this is about a part of
/// the code a person names better by a folder's files, or about the whole repository.
pub const MAX_FACT_ANCHORS: usize = 16;

named_kind! {
    /// What kind of rule a fact is.
    FactKind {
        /// How things are done here.
        Convention => "convention",
        /// A choice made, with its reason.
        Decision => "decision",
        /// What must always hold.
        Invariant => "invariant",
        /// A mistake made before, and how to avoid it.
        Pitfall => "pitfall",
        /// Who or what owns a part.
        Ownership => "ownership",
    }
}

named_kind! {
    /// Who wrote a fact.
    FactSource {
        /// The user, with `cortex note`.
        User => "user",
        /// An agent, with the `cortex_note` tool.
        Agent => "agent",
        /// A teammate's export, imported.
        Import => "import",
    }
}

named_kind! {
    /// Where a fact stands, which alone decides whether it is injected.
    FactStatus {
        /// Written by an agent or an import and not yet confirmed: never injected.
        Candidate => "candidate",
        /// Written or confirmed by the user, or asserted again in a second task: injected.
        Active => "active",
        /// The code an anchor names changed since: never injected until confirmed again.
        Suspect => "suspect",
        /// No longer held: never injected.
        Retired => "retired",
    }
}

impl FactStatus {
    /// Whether a fact in this status may reach an agent.
    #[must_use]
    pub fn is_injected(self) -> bool {
        self == Self::Active
    }
}

impl FactSource {
    /// The status a new fact from this source starts in: the user's are active at once, every
    /// other source's wait for a confirmation.
    #[must_use]
    pub fn first_status(self) -> FactStatus {
        match self {
            Self::User => FactStatus::Active,
            Self::Agent | Self::Import => FactStatus::Candidate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user's fact is active from the start, an agent's or an import's a candidate; only an
    /// active fact is injected.
    #[test]
    fn only_the_user_s_facts_start_active_and_only_active_ones_are_injected() {
        assert_eq!(
            FactSource::ALL
                .iter()
                .map(|source| source.first_status())
                .collect::<Vec<_>>(),
            [
                FactStatus::Active,
                FactStatus::Candidate,
                FactStatus::Candidate
            ]
        );
        assert_eq!(
            FactStatus::ALL
                .iter()
                .filter(|status| status.is_injected())
                .collect::<Vec<_>>(),
            [&FactStatus::Active]
        );
    }
}
