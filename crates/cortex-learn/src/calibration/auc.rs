//! How well a model ranks: the area under the ROC curve, the chance that a used injection drawn at
//! random scores above an unused one drawn at random, ties counting half.

/// The area under the ROC curve of `scored`, each a score and whether it was used, from 0 to 1, by
/// the Mann-Whitney statistic: rank every score from 1 up, give scores that tie the average of
/// their ranks, sum the ranks of the used ones, take away the least that sum could be, and divide
/// by the number of pairs of one used and one unused. None unless both kinds are there.
#[must_use]
pub fn auc(scored: &[(f64, bool)]) -> Option<f64> {
    let used = scored.iter().filter(|(_, is_used)| *is_used).count();
    let unused = scored.len() - used;
    if used == 0 || unused == 0 {
        return None;
    }
    let mut order: Vec<&(f64, bool)> = scored.iter().collect();
    order.sort_by(|left, right| left.0.total_cmp(&right.0));
    // Ranks are doubled so that the average of a run of ties, (first + last) / 2, stays whole.
    let mut used_ranks_doubled: u64 = 0;
    let mut ranked = 0;
    for ties in order.chunk_by(|left, right| left.0.total_cmp(&right.0).is_eq()) {
        let shared_rank_doubled = to_u64(ranked + 1) + to_u64(ranked + ties.len());
        let used_in_run = ties.iter().filter(|item| item.1).count();
        used_ranks_doubled += shared_rank_doubled * to_u64(used_in_run);
        ranked += ties.len();
    }
    let (used, unused) = (to_f64(used), to_f64(unused));
    let least_doubled = used * (used + 1.0);
    let excess = (to_f64_u64(used_ranks_doubled) - least_doubled) / 2.0;
    Some(excess / (used * unused))
}

/// `count` as a `u64`; a slice's length always fits.
fn to_u64(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// `count` as an `f64`, exact for any count of injections Cortex keeps (at most a few thousand).
fn to_f64(count: usize) -> f64 {
    to_f64_u64(to_u64(count))
}

/// `value` as an `f64`, exact below 2^53.
fn to_f64_u64(value: u64) -> f64 {
    // Exact for every value below 2^53, far above the sums of ranks of a few thousand scores.
    #[allow(clippy::cast_precision_loss)]
    let exact = value as f64;
    exact
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scores with whether each was used, and the area they should give.
    type Case<'a> = (&'a [(f64, bool)], Option<f64>);

    /// Worked by hand, pair by pair. Unused at 0.1 and 0.4, used at 0.35 and 0.8: of the four
    /// pairs, 0.35 beats 0.1, loses to 0.4, and 0.8 beats both, so 3 of 4, 0.75. With ties, unused
    /// at 0.2 and 0.5, used at 0.5 and 0.9: 0.5 beats 0.2 and ties 0.5 (a half), 0.9 beats both,
    /// so 3.5 of 4, 0.875. All tied: 0.5. Perfectly ranked: 1; perfectly wrong: 0. One kind
    /// alone has no pairs.
    #[test]
    fn the_area_counts_pairs_ranked_right_with_ties_as_half() {
        let cases: [Case<'_>; 7] = [
            (
                &[(0.1, false), (0.4, false), (0.35, true), (0.8, true)],
                Some(0.75),
            ),
            (
                &[(0.5, true), (0.2, false), (0.9, true), (0.5, false)],
                Some(0.875),
            ),
            (&[(0.3, true), (0.3, false), (0.3, false)], Some(0.5)),
            (&[(0.1, false), (0.2, false), (0.7, true)], Some(1.0)),
            (&[(0.9, false), (0.2, true), (0.1, true)], Some(0.0)),
            (&[(0.4, true), (0.6, true)], None),
            (&[], None),
        ];
        for (scored, expected) in cases {
            assert_eq!(auc(scored), expected, "{scored:?}");
        }
    }
}
