//! The links the map draws between files, of three kinds, each read from what the store holds:
//! files that changed in the same commits (the miner's co-change edges), files agents used
//! together in tasks (the learner's co-access edges, faded to now), and files named in the same
//! work order (counted from the work orders that reached an agent).
//!
//! A link joins two files whichever way the store keeps it. Only the strongest stay: of each
//! kind, a file's [`KEPT_PER_FILE`] strongest links, and a link is kept when it is among those of
//! either end.

use std::collections::BTreeMap;

use cortex_core::EdgeType;
use cortex_git::cochange::partner_rank;
use cortex_learn::{FADED_OUT_BELOW, MAX_LINK_WEIGHT, decayed};
use cortex_retrieve::GraphSettings;
use cortex_store::{NodeId, Queries, StoreError};

use super::listing::Listing;

/// How many links of one kind a file keeps: its strongest. Eight lets a hub show its neighbours
/// without hiding the rest of the map under lines.
pub(super) const KEPT_PER_FILE: usize = 8;

/// What a link is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    /// The files changed in the same commits.
    CoChange,
    /// Agents used the files together in tasks.
    UsedTogether,
    /// A work order named both files.
    WorkOrder,
}

impl Kind {
    /// The name the API gives the kind.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::CoChange => "co_change",
            Self::UsedTogether => "used_together",
            Self::WorkOrder => "work_order",
        }
    }
}

/// One link between two files, by id, the smaller first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Link {
    /// The file with the smaller id.
    pub(super) from: usize,
    /// The file with the larger id.
    pub(super) to: usize,
    /// What the link is made of.
    pub(super) kind: Kind,
    /// How strong it is, from 0 to 1.
    pub(super) weight: f64,
    /// For a co-change link, the commits that changed both files; 0 for the other kinds.
    pub(super) commits: u64,
}

/// A link's strength and commits, by its two files' ids with the smaller first.
type Pairs = BTreeMap<(usize, usize), (f64, u64)>;

/// The strongest links of each kind, sorted by kind, then by the file with the smaller id, then by
/// the other.
///
/// # Errors
///
/// SQLite errors and stored values that cannot be read back.
pub(super) fn read(
    queries: Queries<'_>,
    listing: &Listing,
    (settings, latest_seq): (&GraphSettings, u64),
) -> Result<Vec<Link>, StoreError> {
    let mut links = Vec::new();
    for (kind, pairs) in [
        (Kind::CoChange, co_change(queries, listing)?),
        (
            Kind::UsedTogether,
            used_together(queries, listing, (settings, latest_seq))?,
        ),
        (Kind::WorkOrder, work_orders(queries, listing)?),
    ] {
        links.extend(strongest(kind, &pairs, listing.files().len()));
    }
    // INVARIANT: deterministic (links come in the order of kind, then file ids, never in the
    // order a map or a thread happened to give them)
    links.sort_by_key(|link| (link.kind, link.from, link.to));
    Ok(links)
}

/// The strongest co-change link of all, in the order [`read`] keeps links by: the pair the summary
/// names. Being the strongest of both its files, it is always among the links [`read`] gives, so
/// the summary never names a pair the map does not draw. None when no two listed files changed
/// together.
///
/// # Errors
///
/// SQLite errors and stored values that cannot be read back.
pub(super) fn strongest_co_change(
    queries: Queries<'_>,
    listing: &Listing,
) -> Result<Option<Link>, StoreError> {
    let pairs = co_change(queries, listing)?;
    Ok(pairs
        .iter()
        .map(|(&(from, to), &(weight, commits))| Link {
            from,
            to,
            kind: Kind::CoChange,
            weight,
            commits,
        })
        .min_by(|a, b| stronger(Some(a), Some(b))))
}

/// The pair of ids `from` and `to` as a key, the smaller first; none for a file with itself.
fn pair_of(from: usize, to: usize) -> Option<(usize, usize)> {
    match from.cmp(&to) {
        std::cmp::Ordering::Less => Some((from, to)),
        std::cmp::Ordering::Greater => Some((to, from)),
        std::cmp::Ordering::Equal => None,
    }
}

/// Adds the edge from `src` to `dst` of strength `weight` and `commits` to `pairs`, keeping the
/// stronger of the two directions a pair may have.
fn add_edge(
    pairs: &mut Pairs,
    listing: &Listing,
    (src, dst): (NodeId, NodeId),
    (weight, commits): (f64, u64),
) {
    let (Some(from), Some(to)) = (listing.id_of_node(src), listing.id_of_node(dst)) else {
        return;
    };
    let Some(key) = pair_of(from, to) else {
        return;
    };
    if !weight.is_finite() {
        return;
    }
    let weight = weight.clamp(0.0, 1.0);
    let entry = pairs.entry(key).or_insert((weight, commits));
    if weight > entry.0 {
        *entry = (weight, commits);
    }
}

/// The co-change pairs: each edge's confidence, and the commits that changed both files.
fn co_change(queries: Queries<'_>, listing: &Listing) -> Result<Pairs, StoreError> {
    let mut pairs = Pairs::new();
    queries.each_edge_of_type(EdgeType::CoChange, |row| {
        add_edge(
            &mut pairs,
            listing,
            (row.src, row.edge.dst),
            (row.edge.weight, u64::from(row.edge.count)),
        );
        Ok(())
    })?;
    Ok(pairs)
}

/// The used-together pairs: each link's weight brought forward to the latest task, over the most
/// a link can weigh; a link that faded out is none.
fn used_together(
    queries: Queries<'_>,
    listing: &Listing,
    (settings, latest_seq): (&GraphSettings, u64),
) -> Result<Pairs, StoreError> {
    let mut pairs = Pairs::new();
    queries.each_edge_of_type(EdgeType::CoAccess, |row| {
        let last_seq = u64::try_from(row.edge.last_seq).unwrap_or(0);
        let now = decayed(row.edge.weight, last_seq, latest_seq, settings.link_decay);
        if now < FADED_OUT_BELOW {
            return Ok(());
        }
        add_edge(
            &mut pairs,
            listing,
            (row.src, row.edge.dst),
            (now / MAX_LINK_WEIGHT, 0),
        );
        Ok(())
    })?;
    Ok(pairs)
}

/// The work-order pairs: how many work orders named both files, over the most any pair was named
/// together.
fn work_orders(queries: Queries<'_>, listing: &Listing) -> Result<Pairs, StoreError> {
    let mut counts: BTreeMap<(usize, usize), u64> = BTreeMap::new();
    for order in queries.delivered_work_orders()? {
        let mut named: Vec<usize> = order
            .record
            .files
            .iter()
            .filter_map(|file| listing.id_of_node(file.node))
            .collect();
        named.sort_unstable();
        named.dedup();
        for (at, from) in named.iter().enumerate() {
            for to in named.iter().skip(at + 1) {
                *counts.entry((*from, *to)).or_insert(0) += 1;
            }
        }
    }
    let most = counts.values().copied().max().unwrap_or(0);
    Ok(counts
        .into_iter()
        .map(|(key, count)| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "counts of work orders stay far below 2^52"
            )]
            let weight = count as f64 / most as f64;
            (key, (weight, 0))
        })
        .collect())
}

/// The links of `kind` among `pairs` that are among the [`KEPT_PER_FILE`] strongest of either of
/// their files, as links, ranked as [`stronger`] orders them.
fn strongest(kind: Kind, pairs: &Pairs, file_count: usize) -> Vec<Link> {
    let mut links: Vec<Link> = pairs
        .iter()
        .map(|(&(from, to), &(weight, commits))| Link {
            from,
            to,
            kind,
            weight,
            commits,
        })
        .collect();
    let mut ends: Vec<Vec<usize>> = vec![Vec::new(); file_count];
    for (at, link) in links.iter().enumerate() {
        for end in [link.from, link.to] {
            if let Some(list) = ends.get_mut(end) {
                list.push(at);
            }
        }
    }
    let mut is_kept = vec![false; links.len()];
    for list in &mut ends {
        list.sort_by(|a, b| stronger(links.get(*a), links.get(*b)));
        for at in list.iter().take(KEPT_PER_FILE) {
            if let Some(slot) = is_kept.get_mut(*at) {
                *slot = true;
            }
        }
    }
    let mut kept = is_kept.into_iter();
    links.retain(|_| kept.next().unwrap_or(false));
    links
}

/// How strong `link` is, for ranking: a co-change link by the miner's own partner rank, its
/// weight times `ln(1 + commits)`, so that two files that changed together in 62 commits outrank
/// two whose only 2 commits agree; a link of another kind by its weight.
fn rank_of(link: &Link) -> f64 {
    match link.kind {
        Kind::CoChange => {
            partner_rank(link.weight, u32::try_from(link.commits).unwrap_or(u32::MAX))
        }
        Kind::UsedTogether | Kind::WorkOrder => link.weight,
    }
}

/// How two links order from strongest: by [`rank_of`], then commits, both larger first, then by
/// the files' ids, smaller first. A link that is missing sorts last, which never happens.
fn stronger(a: Option<&Link>, b: Option<&Link>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => rank_of(b)
            .total_cmp(&rank_of(a))
            .then(b.commits.cmp(&a.commits))
            .then((a.from, a.to).cmp(&(b.from, b.to))),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten files all linked to each other at weights that grow with their ids, by hand: the link
    /// of files `a` and `b` (`a` < `b`) weighs `0.01 * (a + b + 1)`, so each file's weakest link
    /// is the one to the lowest id other than itself. Each file keeps 8 of its 9 links, so a link
    /// goes only when it is the weakest of both its ends: files 0 and 1, whose link (0.02) is the
    /// weakest of each; every other file's weakest link, to file 0, is among the strongest 8 of
    /// file 0's. 45 links less that one are 44, and the one left out is 0 and 1's.
    #[test]
    fn a_link_stays_when_it_is_among_the_eight_strongest_of_either_end() {
        let mut pairs = Pairs::new();
        for from in 0..10_u32 {
            for to in (from + 1)..10 {
                let weight = 0.01 * f64::from(from + to + 1);
                pairs.insert((from as usize, to as usize), (weight, 2));
            }
        }
        let kept = strongest(Kind::CoChange, &pairs, 10);
        assert_eq!(kept.len(), 44);
        assert!(!kept.iter().any(|link| (link.from, link.to) == (0, 1)));
        assert!(kept.iter().any(|link| (link.from, link.to) == (0, 2)));
        let keys: Vec<(usize, usize)> = kept.iter().map(|link| (link.from, link.to)).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }

    /// A link that is weak for one end stays when it is strong for the other: file 0 is linked to
    /// ten leaves, the lowest id the weakest, and each leaf has only that link, so all 10 stay, the
    /// two weakest of the hub's among them.
    #[test]
    fn a_link_weak_for_one_end_stays_for_the_other() {
        let mut pairs = Pairs::new();
        for leaf in 1..=10_u32 {
            pairs.insert((0, leaf as usize), (0.01 * f64::from(leaf), 2));
        }
        let kept = strongest(Kind::CoChange, &pairs, 11);
        assert_eq!(kept.len(), 10);
    }

    /// Co-change links rank by weight times `ln(1 + commits)`, then commits, then ids, never by
    /// chance: (0, 2) at 0.5 of 6 commits ranks 0.5 ln 7 = 0.973; (0, 1) and (0, 3) at 0.5 of 5
    /// rank 0.5 ln 6 = 0.896 each, the lower id first; (0, 4) weighs most, 0.6, but over 1 commit
    /// ranks 0.6 ln 2 = 0.416, last.
    #[test]
    fn equal_weights_are_ordered_by_commits_then_ids() {
        let link = |to, weight, commits| Link {
            from: 0,
            to,
            kind: Kind::CoChange,
            weight,
            commits,
        };
        let mut links = [
            link(1, 0.5, 5),
            link(2, 0.5, 6),
            link(3, 0.5, 5),
            link(4, 0.6, 1),
        ];
        links.sort_by(|a, b| stronger(Some(a), Some(b)));
        let order: Vec<usize> = links.iter().map(|link| link.to).collect();
        assert_eq!(order, [2, 1, 3, 4]);
    }

    /// A file's most frequent partner is kept though small files' every commit agrees with it:
    /// file 0 shares 60 commits with file 1 at confidence 0.75 (rank 0.75 ln 61 = 3.083) and 2 of
    /// 2 with each of files 2 to 10 at 1.0 (rank ln 3 = 1.099). Of file 0's ten links it keeps
    /// eight, file 1's first; the nine small links each stay too, as each is its leaf's only link.
    #[test]
    fn a_frequent_partner_outranks_a_perfect_pair_of_two_commits() {
        let mut pairs = Pairs::new();
        pairs.insert((0, 1), (0.75, 60));
        for leaf in 2..=10_usize {
            pairs.insert((0, leaf), (1.0, 2));
        }
        let links: Vec<Link> = pairs
            .iter()
            .map(|(&(from, to), &(weight, commits))| Link {
                from,
                to,
                kind: Kind::CoChange,
                weight,
                commits,
            })
            .collect();
        let best = links
            .iter()
            .min_by(|a, b| stronger(Some(a), Some(b)))
            .map(|link| (link.from, link.to));
        assert_eq!(best, Some((0, 1)));
        let kept = strongest(Kind::CoChange, &pairs, 11);
        assert_eq!(kept.len(), 10);
        assert!(kept.iter().any(|link| (link.from, link.to) == (0, 1)));
    }
}
