//! Versions and how they compare: the protocol's, which decides whether two processes can talk,
//! and Cortex's own, which decides which of two installed versions keeps the daemon.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// This build's Cortex version, as written in its manifest.
pub const CORTEX_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A protocol version. Two processes talk when their major versions match; a minor version only
/// adds fields that a reader of an earlier one ignores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protocol {
    /// Changes when a message changes in a way an earlier reader cannot read.
    pub major: u32,
    /// Changes when a message gains a field an earlier reader can ignore.
    pub minor: u32,
}

impl Protocol {
    /// The protocol this build speaks.
    pub const CURRENT: Self = Self { major: 1, minor: 0 };

    /// Whether a process speaking this protocol can talk to one speaking `other`.
    #[must_use]
    pub fn is_compatible_with(self, other: Self) -> bool {
        self.major == other.major
    }
}

/// A Cortex version: major, minor and patch numbers and an optional pre-release tag such as `dev`
/// or `rc.1`, ordered as semantic versioning orders them. Build metadata after `+` is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CortexVersion {
    /// Major, minor and patch.
    numbers: [u64; 3],
    /// The pre-release identifiers, empty for a release.
    pre: Vec<PreRelease>,
}

/// One dot-separated identifier of a pre-release tag.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PreRelease {
    /// Digits only: compared as a number, and before any text.
    Number(u64),
    /// Anything else: compared as text, byte by byte.
    Text(String),
}

/// Why a text is not a Cortex version.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{text:?} is not a version of the form major.minor.patch[-pre-release]")]
pub struct VersionError {
    /// The text, cut to its first 40 characters.
    text: String,
}

impl CortexVersion {
    /// The version written as `text`, such as `2.0.0` or `2.0.0-dev`.
    ///
    /// # Errors
    ///
    /// The text is not three dot-separated numbers, optionally followed by `-` and dot-separated
    /// identifiers of letters, digits and `-`.
    pub fn parse(text: &str) -> Result<Self, VersionError> {
        let refused = || VersionError {
            text: text.chars().take(40).collect(),
        };
        let without_build = text.split_once('+').map_or(text, |(version, _)| version);
        let (core, pre) = match without_build.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (without_build, None),
        };
        let numbers: Vec<u64> = core
            .split('.')
            .map(|part| parse_number(part).ok_or_else(refused))
            .collect::<Result<_, _>>()?;
        let numbers: [u64; 3] = numbers.try_into().map_err(|_| refused())?;
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre
                .split('.')
                .map(|part| parse_identifier(part).ok_or_else(refused))
                .collect::<Result<_, _>>()?,
        };
        Ok(Self { numbers, pre })
    }

    /// Whether this version comes after `other`.
    #[must_use]
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self > other
    }
}

/// `part` as a number: digits only, with no leading zero unless it is zero itself.
fn parse_number(part: &str) -> Option<u64> {
    let is_digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if !is_digits || (part.len() > 1 && part.starts_with('0')) {
        return None;
    }
    part.parse().ok()
}

/// `part` as a pre-release identifier: a number, or letters, digits and `-`.
fn parse_identifier(part: &str) -> Option<PreRelease> {
    if let Some(number) = parse_number(part) {
        return Some(PreRelease::Number(number));
    }
    let is_text = !part.is_empty()
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    is_text.then(|| PreRelease::Text(part.to_owned()))
}

impl Ord for CortexVersion {
    /// Numbers first; then a release comes after any of its pre-releases, and pre-releases compare
    /// identifier by identifier, a shorter list first when all it has are equal.
    fn cmp(&self, other: &Self) -> Ordering {
        self.numbers.cmp(&other.numbers).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            }
        })
    }
}

impl PartialOrd for CortexVersion {
    /// See [`Ord`].
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordering example semantic versioning publishes, each version before the next, then
    /// Cortex's own development version before its release and both after the version before.
    #[test]
    fn versions_order_as_semantic_versioning_says() -> Result<(), VersionError> {
        let published = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.9.9",
            "2.0.0-dev",
            "2.0.0",
            "2.0.1",
        ];
        let parsed = published
            .iter()
            .map(|text| CortexVersion::parse(text))
            .collect::<Result<Vec<_>, _>>()?;
        for pair in parsed.windows(2) {
            if let [earlier, later] = pair {
                assert!(later.is_newer_than(earlier), "{later:?} after {earlier:?}");
                assert!(!earlier.is_newer_than(later));
            }
        }
        let built = CortexVersion::parse("2.0.0+build.5")?;
        assert_eq!(built.cmp(&CortexVersion::parse("2.0.0")?), Ordering::Equal);
        Ok(())
    }

    /// This build's own version reads.
    #[test]
    fn this_build_s_version_reads() {
        assert!(CortexVersion::parse(CORTEX_VERSION).is_ok());
    }

    /// Texts that are not versions are refused.
    #[test]
    fn other_texts_are_refused() {
        for text in [
            "",
            "2",
            "2.0",
            "2.0.0.0",
            "02.0.0",
            "2.0.x",
            "2.0.0-",
            "2.0.0-a..b",
            "v2.0.0",
        ] {
            assert!(CortexVersion::parse(text).is_err(), "{text:?}");
        }
    }

    /// Two protocols talk when their majors match, whatever their minors.
    #[test]
    fn protocols_talk_when_their_majors_match() {
        let current = Protocol::CURRENT;
        let newer_minor = Protocol {
            major: current.major,
            minor: current.minor + 1,
        };
        let newer_major = Protocol {
            major: current.major + 1,
            minor: 0,
        };
        assert!(current.is_compatible_with(newer_minor));
        assert!(!current.is_compatible_with(newer_major));
    }
}
