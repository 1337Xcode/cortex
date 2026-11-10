//! Strings that look random, which is what a credential of no known shape looks like. A word is
//! read as runs of letters, digits, `+`, `/`, `_` and `-` (the characters of base64 and of
//! paths); a run is judged when it has [`MIN_RUN_CHARS`] or more and holds both a digit and a
//! letter, and redacted when its Shannon entropy per character is above [`MIN_ENTROPY_SHARE`] of
//! the most a run of its length can have.
//!
//! The digit is asked for because names chosen by people seldom hold one and random strings
//! almost always do: without it, 228 of 12,538 runs of 20 characters or more in real test file
//! paths passed the bound; with it, 26 of 33,046 runs in those paths and test names together.
//!
//! A run of hex digits alone is judged against the 16 characters it is drawn from, so a git
//! commit id of 40 characters is redacted. That is deliberate: hex is also the shape of many
//! services' keys and of older GitHub tokens, which cannot be told from a commit id, and a commit
//! id in a check command is rare, so redacting one costs little.

use super::detectors::{Finding, run_end};

/// The least characters a run must have to be judged: in fewer, random and chosen text cannot be
/// told apart, and a credential of fewer is the other detectors' to find by its name or shape.
const MIN_RUN_CHARS: usize = 20;

/// The characters a run is drawn from when it is not hex alone: base64's 64.
const ALPHABET_CHARS: usize = 64;

/// The characters a run of hex digits alone is drawn from.
const HEX_ALPHABET_CHARS: usize = 16;

/// The share of the most entropy a run can have above which it looks random. A run of `n`
/// characters drawn from `a` can have at most `log2(min(n, a))` bits per character, so a longer
/// run can have more: a fixed bound in bits would treat long paths as random (real test paths
/// reach 4.6 bits) or let short tokens through (a random 20-character token averages 4.02).
///
/// Measured: of 33,046 runs of 20 characters or more in the test file paths and test names of 35
/// open-source repositories, 1,152 hold both a digit and a letter and are not hex alone, and 26 of
/// those are above 0.80 (`test_127001_replaced` is one, at 0.85); of random strings of 20 to 40
/// characters, 97% or more of those in base62 and base64 and 91% to 98% of those in lower-case
/// letters and digits are above it, the rest lacking a digit or repeating characters by chance.
///
/// Worked at 20 characters, where the bound is 0.80 * log2 20 = 3.4575 bits:
/// `gh12gh12jk34jk34mn56` (eight characters twice and four once: 8 * 0.1 * log2 10 + 4 * 0.05 *
/// log2 20 = 3.5219 bits) is redacted, and `gh12jk34mngh12jk34mn` (ten characters twice: log2 10 =
/// 3.3219 bits) is kept. A 40-character commit id, with a bound of 0.80 * log2 16 = 3.2 bits, is
/// above it in all but about two cases in ten thousand of random ones
/// (`24b19f9c3a1e5d7f0b2c4e6a8d0f1b3c5e7a9d0f`: 3.9210 bits).
const MIN_ENTROPY_SHARE: f64 = 0.80;

/// The runs of `word` that look random.
pub(super) fn find(word: &str) -> Vec<Finding> {
    let bytes = word.as_bytes();
    let mut found = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let end = run_end(bytes, start, is_run_byte);
        if bytes.get(start..end).is_some_and(looks_random) {
            found.push(Finding::Secret(start..end));
        }
        // The byte at `end` ends the run, or begins none: the next run starts after it.
        start = end + 1;
    }
    found
}

/// Whether `run` looks random.
fn looks_random(run: &[u8]) -> bool {
    let is_judged = run.len() >= MIN_RUN_CHARS
        && run.iter().any(u8::is_ascii_digit)
        && run.iter().any(u8::is_ascii_alphabetic);
    if !is_judged {
        return false;
    }
    let alphabet = if run.iter().all(u8::is_ascii_hexdigit) {
        HEX_ALPHABET_CHARS
    } else {
        ALPHABET_CHARS
    };
    entropy_bits(run) > MIN_ENTROPY_SHARE * log2_of(run.len().min(alphabet))
}

/// The Shannon entropy of `run` in bits per character.
fn entropy_bits(run: &[u8]) -> f64 {
    let mut counts = [0_u32; 256];
    for &byte in run {
        if let Some(count) = counts.get_mut(usize::from(byte)) {
            *count += 1;
        }
    }
    let total = f64::from(u32::try_from(run.len()).unwrap_or(u32::MAX));
    counts
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let share = f64::from(count) / total;
            -share * share.log2()
        })
        .sum()
}

/// The base-2 logarithm of `n`.
fn log2_of(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX)).log2()
}

/// A byte of a run: a letter, digit, `+`, `/`, `_` or `-`.
fn is_run_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'_' | b'-')
}
