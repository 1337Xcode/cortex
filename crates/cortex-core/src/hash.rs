//! BLAKE3 hashes at the two sizes storage uses: 32 bytes for file contents, 16 bytes for symbols
//! and fact anchors.

use std::fmt;

/// The BLAKE3 hash of a file's bytes, 32 bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash([u8; 32]);

/// The first 16 bytes of a BLAKE3 hash, for symbol signatures and bodies and for fact anchors.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShortHash([u8; 16]);

impl ContentHash {
    /// The hash of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// The hash as stored: 32 raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// A hash read back from storage.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Its first 16 bytes: the short hash of the same bytes, as [`ShortHash::of`] gives it.
    #[must_use]
    pub fn short(&self) -> ShortHash {
        ShortHash(self.0.first_chunk().copied().unwrap_or_default())
    }
}

impl ShortHash {
    /// The first 16 bytes of the hash of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        let mut short = [0_u8; 16];
        short.copy_from_slice(&blake3::hash(bytes).as_bytes()[..16]);
        Self(short)
    }

    /// The hash as stored: 16 raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// A hash read back from storage.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// The hash written as 32 lower-case hexadecimal characters, as `Display` writes it; `None`
    /// for any other text.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 32 {
            return None;
        }
        let mut bytes = [0_u8; 16];
        for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
            let digit = |c: u8| match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                _ => None,
            };
            let [high, low] = pair else {
                return None;
            };
            *byte = (digit(*high)? << 4) | digit(*low)?;
        }
        Some(Self(bytes))
    }
}

impl serde::Serialize for ShortHash {
    /// Its 32 lower-case hexadecimal characters.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for ShortHash {
    /// Read from its 32 lower-case hexadecimal characters, and from nothing else.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Self::from_hex(&text).ok_or_else(|| {
            serde::de::Error::custom("not a short hash of 32 hexadecimal characters")
        })
    }
}

/// Writes bytes as lower-case hexadecimal.
fn write_hex(formatter: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    bytes
        .iter()
        .try_for_each(|byte| write!(formatter, "{byte:02x}"))
}

impl fmt::Display for ContentHash {
    /// Lower-case hexadecimal, 64 characters.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

impl fmt::Debug for ContentHash {
    /// The same hexadecimal as `Display`, inside the type name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ContentHash({self})")
    }
}

impl fmt::Display for ShortHash {
    /// Lower-case hexadecimal, 32 characters.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

impl fmt::Debug for ShortHash {
    /// The same hexadecimal as `Display`, inside the type name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ShortHash({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The official BLAKE3 test vectors (BLAKE3-team/BLAKE3, `test_vectors/test_vectors.json`):
    /// inputs are the repeating bytes 0, 1, ..., 250, 0, 1, ...; the hash is the first 32 bytes.
    #[test]
    fn hashes_match_the_published_vectors() {
        let pattern: Vec<u8> = (0..1023_u32)
            .map(|index| u8::try_from(index % 251).unwrap_or_default())
            .collect();
        let cases = [
            (
                0,
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            ),
            (
                1,
                "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
            ),
            (
                1023,
                "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
            ),
        ];
        for (length, expected) in cases {
            let input = &pattern[..length];
            assert_eq!(ContentHash::of(input).to_string(), expected, "{length}");
            assert_eq!(
                ShortHash::of(input).to_string(),
                &expected[..32],
                "{length}"
            );
        }
    }

    /// The short form of a file's hash is the short hash of the same bytes.
    #[test]
    fn a_content_hash_shortens_to_the_short_hash() {
        for input in [&b""[..], b"fn main() {}", &[0xff_u8; 300]] {
            assert_eq!(ContentHash::of(input).short(), ShortHash::of(input));
        }
    }

    /// A short hash reads back from the hexadecimal `Display` writes and from nothing else: the
    /// first half of the published hash of no bytes reads as the short hash of no bytes; the same
    /// in upper case, a character short, a character over, or with a character outside `0-9a-f`
    /// reads as nothing.
    #[test]
    fn a_short_hash_reads_back_from_its_hexadecimal() {
        let hex = "af1349b9f5f9a1a6a0404dea36dcc949";
        assert_eq!(ShortHash::from_hex(hex), Some(ShortHash::of(b"")));
        assert_eq!(ShortHash::of(b"").to_string(), hex);
        let refused = [
            hex.to_uppercase(),
            hex[..31].to_owned(),
            format!("{hex}0"),
            format!("g{}", &hex[1..]),
        ];
        for text in refused {
            assert_eq!(ShortHash::from_hex(&text), None, "{text}");
        }
    }

    /// A short hash is written in stored facts as its hexadecimal, and read back from that alone.
    #[test]
    fn a_short_hash_is_stored_as_its_hexadecimal() -> Result<(), serde_json::Error> {
        let short = ShortHash::of(b"");
        let json = serde_json::to_string(&short)?;
        assert_eq!(json, r#""af1349b9f5f9a1a6a0404dea36dcc949""#);
        assert_eq!(serde_json::from_str::<ShortHash>(&json)?, short);
        assert!(
            serde_json::from_str::<ShortHash>(r#""AF1349B9F5F9A1A6A0404DEA36DCC949""#).is_err()
        );
        assert!(serde_json::from_str::<ShortHash>("16").is_err());
        Ok(())
    }

    /// Stored bytes read back give the same hash.
    #[test]
    fn bytes_round_trip() {
        let content = ContentHash::of(b"fn main() {}");
        assert_eq!(ContentHash::from_bytes(*content.as_bytes()), content);
        let short = ShortHash::of(b"fn main() {}");
        assert_eq!(ShortHash::from_bytes(*short.as_bytes()), short);
    }
}
