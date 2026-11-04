//! A hasher for node numbers, which spreading keys what each node holds by. The standard library's
//! default hasher defends against keys picked to collide, and with it spreading took a third to a
//! half longer in the benchmarks. Node numbers are the graph's own, 0 up in id order, and a
//! spreading's pushes are capped and its deadline looked at as it goes, so they need no such
//! defence. A number is spread over the whole hash by one multiplication, then the high half is
//! folded into the low, because a hash table picks a bucket by the low bits: without the fold,
//! numbers on a regular stride (every 1,024th, say) all land in a few buckets.

use std::hash::{BuildHasherDefault, Hasher};

/// The multiplier: 2^64 over the golden ratio, made odd, whose multiples of consecutive numbers
/// fall far apart.
const MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;

/// What makes a [`NodeHasher`] for each hash a map keyed by node numbers takes.
pub(crate) type NodeHashing = BuildHasherDefault<NodeHasher>;

/// The hasher: the hash of what was written so far.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NodeHasher(u64);

impl NodeHasher {
    /// Folds `value` into the hash.
    fn mix(&mut self, value: u64) {
        let spread = (self.0 ^ value).wrapping_mul(MULTIPLIER);
        self.0 = spread ^ (spread >> 32);
    }
}

impl Hasher for NodeHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.mix(u64::from(*byte));
        }
    }

    fn write_u32(&mut self, node: u32) {
        self.mix(u64::from(node));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::hash::BuildHasher;

    use super::*;

    /// The ten low bits of node `node`'s hash: its bucket in a table of 1,024.
    fn bucket(node: u32) -> u64 {
        NodeHashing::default().hash_one(node) & 1023
    }

    /// Numbers on a stride of 1,024 spread over a table of 1,024 buckets. Without the fold every
    /// one would land in bucket 0, since a multiple of 1,024 times an odd number has ten low zero
    /// bits. Dropped at random, 1,024 numbers fill 1,024 (1 - (1023/1024)^1024), about 648, buckets
    /// on average, give or take 13; at least half the table is asked.
    #[test]
    fn numbers_on_a_stride_spread_over_the_buckets() {
        let buckets: BTreeSet<u64> = (0..1024).map(|at| bucket(at * 1024)).collect();
        assert!(buckets.len() >= 512, "{} buckets", buckets.len());
    }

    /// The same number always hashes the same, and two numbers apart differ.
    #[test]
    fn a_number_always_hashes_the_same() {
        let hashing = NodeHashing::default();
        assert_eq!(hashing.hash_one(7_u32), hashing.hash_one(7_u32));
        assert_ne!(hashing.hash_one(7_u32), hashing.hash_one(8_u32));
    }
}
