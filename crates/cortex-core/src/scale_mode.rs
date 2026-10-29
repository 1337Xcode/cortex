//! How much Cortex does for a repository of a given size.

use std::fmt;

/// The scale mode: how fully a repository is indexed and how much is injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScaleMode {
    /// A few files: record what agents do, inject almost nothing.
    Tiny,
    /// A small project: full index, a high bar for injecting.
    Small,
    /// Most repositories: full index with the file watcher.
    Medium,
    /// Very large repositories: inventory first, symbols parsed lazily.
    Huge,
}

impl ScaleMode {
    /// Every mode, smallest first.
    pub const ALL: [Self; 4] = [Self::Tiny, Self::Small, Self::Medium, Self::Huge];

    /// The stored and configured name: `tiny`, `small`, `medium` or `huge`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Huge => "huge",
        }
    }

    /// The mode with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }
}

impl fmt::Display for ScaleMode {
    /// The mode's name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each name reads back to its mode; other words are no mode.
    #[test]
    fn names_read_back() {
        for (mode, name) in [
            (ScaleMode::Tiny, "tiny"),
            (ScaleMode::Small, "small"),
            (ScaleMode::Medium, "medium"),
            (ScaleMode::Huge, "huge"),
        ] {
            assert_eq!(mode.name(), name);
            assert_eq!(ScaleMode::from_name(name), Some(mode));
        }
        assert_eq!(ScaleMode::from_name("auto"), None);
    }
}
