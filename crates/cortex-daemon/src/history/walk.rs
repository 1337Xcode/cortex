//! The walk one step past the commits whose message is not worth showing, and the runs the cache
//! keeps. When a trivial, mass or bot commit owns lines of a span, the span is blamed once more at
//! HEAD with every such owner ignored (`--ignore-rev`), so that git maps each of their lines
//! through the change to the commit that changed it before, however the change moved lines; the
//! cache keeps that second blame's runs. One step only: a commit the second blame names is kept
//! as found, whatever its kind.
//!
//! A run git blamed on a boundary is dropped unless the boundary is known to be a root: blame
//! stopped there at the date limit, so the commit only stands for lines older than the history
//! read, and did not necessarily write them. A root commit wrote what it holds.

use std::collections::BTreeMap;

use cortex_git::blame::{Blame, BlameRun};
use cortex_store::{BlameRunRecord, CardKind};

/// What the walk needs to know of a commit that owns a run: what kind of change its card says it
/// is, and whether it has a parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Owner {
    /// The kind of change.
    pub(crate) kind: CardKind,
    /// Whether the commit has a parent: false for a root commit.
    pub(crate) has_parent: bool,
}

/// The commits to look through in `blame`, in id order: each owner of a run that `owners` knows
/// as a change other than a normal one and that is not a boundary.
pub(crate) fn ignored(blame: &Blame, owners: &BTreeMap<String, Owner>) -> Vec<String> {
    blame
        .commits
        .iter()
        .filter(|(oid, commit)| {
            !commit.is_boundary
                && owners
                    .get(*oid)
                    .is_some_and(|owner| owner.kind != CardKind::Normal)
        })
        .map(|(oid, _)| oid.clone())
        .collect()
}

/// The runs the cache keeps of `blame`: every run a boundary owns that is not known to be a root
/// dropped, and next runs of one commit whose lines run on in both files joined.
pub(crate) fn kept_runs(blame: &Blame, owners: &BTreeMap<String, Owner>) -> Vec<BlameRunRecord> {
    let mut kept: Vec<BlameRunRecord> = Vec::new();
    for run in &blame.runs {
        if is_cut(run, blame, owners) {
            continue;
        }
        join_or_push(&mut kept, record(run));
    }
    kept
}

/// Whether `run`, of `blame`, is owned by a boundary that is not known to be a root: a cut of the
/// history read rather than the commit that wrote the lines.
fn is_cut(run: &BlameRun, blame: &Blame, owners: &BTreeMap<String, Owner>) -> bool {
    let is_boundary = blame
        .commits
        .get(&run.oid)
        .is_some_and(|commit| commit.is_boundary);
    let is_root = owners.get(&run.oid).is_some_and(|owner| !owner.has_parent);
    is_boundary && !is_root
}

/// Adds `next` after the runs `kept`, joined to the last when it is the same commit's and its
/// lines follow the last's in both files.
fn join_or_push(kept: &mut Vec<BlameRunRecord>, next: BlameRunRecord) {
    if let Some(last) = kept.last_mut() {
        let source_end = last
            .source_start
            .saturating_add(last.end.saturating_sub(last.start));
        let runs_on = last.oid == next.oid
            && last.end.checked_add(1) == Some(next.start)
            && source_end.checked_add(1) == Some(next.source_start);
        if runs_on {
            last.end = next.end;
            return;
        }
    }
    kept.push(next);
}

/// `run` as the store keeps it.
fn record(run: &BlameRun) -> BlameRunRecord {
    BlameRunRecord {
        start: run.start,
        end: run.end,
        source_start: run.source_start,
        oid: run.oid.clone(),
    }
}

#[cfg(test)]
mod tests {
    use cortex_git::blame::BlamedCommit;

    use super::*;

    /// The id of the commit named by `letter`.
    fn oid(letter: char) -> String {
        letter.to_string().repeat(40)
    }

    /// The run of lines `start..=end`, from `source_start` of `letter`'s file.
    fn run(start: u32, end: u32, source_start: u32, letter: char) -> BlameRun {
        BlameRun {
            start,
            end,
            oid: oid(letter),
            source_start,
        }
    }

    /// The stored run of lines `start..=end`, from `source_start` of `letter`'s file.
    fn kept(start: u32, end: u32, source_start: u32, letter: char) -> BlameRunRecord {
        BlameRunRecord {
            start,
            end,
            source_start,
            oid: oid(letter),
        }
    }

    /// What porcelain says of `letter`: whether it is a boundary.
    fn commit(letter: char, is_boundary: bool) -> (String, BlamedCommit) {
        (
            oid(letter),
            BlamedCommit {
                previous: None,
                is_boundary,
            },
        )
    }

    /// Owners: `n` and `m` normal changes, `r` a normal root, `t` trivial, `x` a mass change, `b` a
    /// bot's, each with a parent but `r`.
    fn owners() -> BTreeMap<String, Owner> {
        [
            ('n', CardKind::Normal, true),
            ('m', CardKind::Normal, true),
            ('r', CardKind::Normal, false),
            ('t', CardKind::Trivial, true),
            ('x', CardKind::Mass, true),
            ('b', CardKind::Bot, true),
        ]
        .into_iter()
        .map(|(letter, kind, has_parent)| (oid(letter), Owner { kind, has_parent }))
        .collect()
    }

    /// Of the owners of a blame, the trivial, mass and bot commits that are not boundaries are
    /// looked through, in id order; a normal one, a boundary bot and a commit with no card are not.
    #[test]
    fn only_owners_of_other_kinds_are_looked_through() {
        let blame = Blame {
            runs: vec![
                run(1, 1, 1, 'n'),
                run(2, 2, 2, 'x'),
                run(3, 3, 3, 't'),
                run(4, 4, 4, 'b'),
                run(5, 5, 5, 'u'),
            ],
            commits: [
                commit('n', false),
                commit('x', false),
                commit('t', false),
                commit('b', true),
                commit('u', false),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(ignored(&blame, &owners()), [oid('t'), oid('x')]);
    }

    /// A run a boundary with a parent owns is a cut of the history read, and is dropped; a root's
    /// boundary is kept, and so is a run that is no boundary. A boundary whose owner has no card is
    /// not known to be a root, and is dropped. Worked by hand: `m`'s lines 4 and 5 from its 9 and
    /// 10 run on in both files and join.
    #[test]
    fn runs_cut_by_the_date_limit_are_dropped_and_neighbours_joined() {
        let blame = Blame {
            runs: vec![
                run(1, 1, 1, 'r'),
                run(2, 2, 2, 'n'),
                run(3, 3, 3, 'u'),
                run(4, 4, 9, 'm'),
                run(5, 5, 10, 'm'),
                run(6, 6, 12, 'm'),
            ],
            commits: [
                commit('r', true),
                commit('n', true),
                commit('u', true),
                commit('m', false),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(
            kept_runs(&blame, &owners()),
            [kept(1, 1, 1, 'r'), kept(4, 5, 9, 'm'), kept(6, 6, 12, 'm')]
        );
    }
}
