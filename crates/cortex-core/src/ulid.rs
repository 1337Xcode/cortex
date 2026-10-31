//! Sortable unique ids for events and tasks.

use std::fmt;

/// How many bytes of the seed's hash fill an id's 80 bits after the time.
const RANDOM_BYTES: usize = 10;

/// Characters in an id's text form.
const TEXT_CHARS: usize = 26;

/// A sortable unique id (a ULID): 48 bits of UTC milliseconds, then 80 bits from the BLAKE3 hash
/// of a seed. The caller's seed must differ between any two ids made in one millisecond (a
/// process id, a clock in nanoseconds and a counter do); the ids need to be unique, not
/// unpredictable, so no random number generator is involved. Written as 26 characters of
/// Crockford's base 32, which sort as the ids do.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ulid(ulid::Ulid);

/// Why a text is not an id.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{text:?} is not a ULID")]
pub struct UlidError {
    /// The text, cut to its first 40 characters.
    text: String,
}

impl Ulid {
    /// The id made at `timestamp_ms` from `seed`: its 80 bits after the time are the first 10
    /// bytes of the seed's BLAKE3 hash, read as a big-endian number. Times past the 48 bits an id
    /// holds keep their low 48 bits.
    #[must_use]
    pub fn from_seed(timestamp_ms: u64, seed: &[u8]) -> Self {
        let hash = blake3::hash(seed);
        let mut random = [0u8; 16];
        let (_, low) = random.split_at_mut(16 - RANDOM_BYTES);
        let (head, _) = hash.as_bytes().split_at(RANDOM_BYTES);
        low.copy_from_slice(head);
        Self(ulid::Ulid::from_parts(
            timestamp_ms,
            u128::from_be_bytes(random),
        ))
    }

    /// The id written as `text`.
    ///
    /// # Errors
    ///
    /// The text is not the one spelling of an id: 26 upper-case characters of Crockford's base
    /// 32, the first at most 7 so that the number fits in 128 bits.
    pub fn parse(text: &str) -> Result<Self, UlidError> {
        let refused = || UlidError {
            text: text.chars().take(40).collect(),
        };
        let id = ulid::Ulid::from_string(text)
            .map(Self)
            .map_err(|_| refused())?;
        // The decoder drops the bits a first character past 7 overflows, and reads lower case and
        // look-alike letters as digits: only the canonical spelling is an id's text.
        if id.to_string() != text {
            return Err(refused());
        }
        Ok(id)
    }

    /// The UTC milliseconds the id was made at.
    #[must_use]
    pub fn timestamp_ms(self) -> u64 {
        self.0.timestamp_ms()
    }
}

impl fmt::Display for Ulid {
    /// The 26 characters of Crockford's base 32.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buffer = [0u8; TEXT_CHARS];
        formatter.write_str(self.0.array_to_str(&mut buffer))
    }
}

impl fmt::Debug for Ulid {
    /// The text form, inside the type name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Ulid({self})")
    }
}

impl serde::Serialize for Ulid {
    /// The text form.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Ulid {
    /// An id from its text form; any other text is refused.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id of an empty seed at 1,700,000,000,000 ms, worked out outside Cortex: the time is
    /// 0x018BCFE56800, the first 10 bytes of the published BLAKE3 hash of no bytes are
    /// af1349b9f5f9a1a6a040; in Crockford's base 32 the 128 bits read 01HF7YAT00NW9MKEFNZ6GTD820.
    #[test]
    fn an_id_is_the_time_then_the_seed_s_hash() -> Result<(), UlidError> {
        let id = Ulid::from_seed(1_700_000_000_000, b"");
        assert_eq!(id.to_string(), "01HF7YAT00NW9MKEFNZ6GTD820");
        assert_eq!(id.timestamp_ms(), 1_700_000_000_000);
        assert_eq!(Ulid::parse("01HF7YAT00NW9MKEFNZ6GTD820")?, id);
        Ok(())
    }

    /// Ids sort by time first; one millisecond with two seeds gives two ids.
    #[test]
    fn ids_sort_by_time_and_differ_by_seed() {
        let early = Ulid::from_seed(1_000, b"b");
        let late = Ulid::from_seed(1_001, b"a");
        assert!(early < late);
        assert!(early.to_string() < late.to_string());
        assert_ne!(Ulid::from_seed(1_000, b"a"), Ulid::from_seed(1_000, b"b"));
    }

    /// Text that is not an id is refused: too short, a letter base 32 leaves out, a first
    /// character past 7, which would overflow 128 bits, and an id in lower case.
    #[test]
    fn other_text_is_refused() {
        for text in [
            "01HF7YAT00",
            "01HF7YAT00NW9MKEFNZ6GTD84U",
            "81HF7YAT00NW9MKEFNZ6GTD820",
            "01hf7yat00nw9mkefnz6gtd820",
        ] {
            assert!(Ulid::parse(text).is_err(), "{text}");
        }
    }
}
