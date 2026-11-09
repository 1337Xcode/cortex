//! How alike two sets are: the Jaccard index, what they share over what they hold between them.
//! Clustering tasks by the files they changed, matching a prompt's words to past tasks and merging
//! facts that say the same thing all measure likeness this one way.

use std::collections::BTreeSet;

/// What `left` and `right` share over what they hold between them, from 0 for sets with nothing in
/// common to 1 for equal sets; 0 for two empty sets.
///
/// `{a, b, c}` and `{b, c, d}` share 2 of the 4 they hold: 0.5.
#[must_use]
pub fn jaccard<T: Ord>(left: &BTreeSet<T>, right: &BTreeSet<T>) -> f64 {
    let both = left.intersection(right).count();
    let either = left.len() + right.len() - both;
    if either == 0 {
        return 0.0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "sets this large are far beyond any count a float loses whole numbers at"
    )]
    let share = both as f64 / either as f64;
    share
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked by hand: 2 shared of 4 held; equal sets; nothing shared; two empty sets.
    #[test]
    fn likeness_is_shared_over_held() {
        let set = |items: &[&'static str]| items.iter().copied().collect::<BTreeSet<_>>();
        assert!((jaccard(&set(&["a", "b", "c"]), &set(&["b", "c", "d"])) - 0.5).abs() < 1e-12);
        assert!((jaccard(&set(&["a"]), &set(&["a"])) - 1.0).abs() < 1e-12);
        assert!(jaccard(&set(&["a"]), &set(&["b"])).abs() < 1e-12);
        assert!(jaccard::<&str>(&set(&[]), &set(&[])).abs() < 1e-12);
    }
}
