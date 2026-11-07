//! Which other live sessions an edit concerns: those holding the edited node itself, or a node
//! coupled to it strongly enough (a structural link of 0.8 or more, or co-change of 0.5 or more).
//! Each such session is concerned once, by the strongest reason it has: the same node before a
//! coupled one, a structural link before co-change, a stronger link before a weaker. Two sessions
//! that both edited the same node within 10 minutes are urgent: both are told first.

use cortex_core::CoordinationConfig;

use crate::position::{Position, PositionMode, Timing, is_live};

/// How strongly two nodes are coupled, as the graph says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Link {
    /// A structural edge (a call, an import, a test) of this weight.
    Structural(f64),
    /// Co-change of this confidence.
    CoChange(f64),
}

/// The least weights that couple two nodes for a notice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// The least structural weight.
    pub structural: f64,
    /// The least co-change confidence.
    pub cochange: f64,
}

impl Thresholds {
    /// The least weights `config` sets.
    #[must_use]
    pub fn of(config: &CoordinationConfig) -> Self {
        Self {
            structural: config.structural_min_weight,
            cochange: config.cochange_min_confidence,
        }
    }
}

/// Why an edit concerns a session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Via {
    /// The session holds the edited node itself.
    Same,
    /// The session holds a node coupled to it.
    Coupled(Link),
}

/// One other session an edit concerns.
#[derive(Debug, Clone, PartialEq)]
pub struct Concern<S, N> {
    /// The session.
    pub session: S,
    /// The node it holds.
    pub held: N,
    /// What it did to that node.
    pub mode: PositionMode,
    /// Why the edit concerns it.
    pub via: Via,
    /// Whether it edited the same node within the same-file window: both are told first.
    pub is_urgent: bool,
}

/// The edit the concerns are about: who made it, on which node, when.
#[derive(Debug, Clone, Copy)]
pub struct EditAt<'e, S, N> {
    /// The session that edited.
    pub session: &'e S,
    /// The node it edited.
    pub node: N,
    /// When, in UTC milliseconds.
    pub at_ms: i64,
}

/// The sessions `edit` concerns, from the positions other sessions hold (`positions`), each
/// session's last event (`last_event_ms`), and the nodes coupled to the edited one (`links`):
/// urgent ones first, then by session. Positions of the editing session, of sessions no longer
/// live, and that no longer hold, concern nobody.
pub fn concerned<S, N>(
    edit: EditAt<'_, S, N>,
    positions: &[Position<S, N>],
    last_event_ms: &dyn Fn(&S) -> Option<i64>,
    (links, thresholds, timing): (&[(N, Link)], &Thresholds, &Timing),
) -> Vec<Concern<S, N>>
where
    S: Clone + Ord,
    N: Copy + Eq,
{
    let mut found: Vec<Concern<S, N>> = Vec::new();
    for position in positions {
        let is_other_live = &position.session != edit.session
            && last_event_ms(&position.session)
                .is_some_and(|last| is_live(last, edit.at_ms, timing));
        if !is_other_live || !position.holds_at(edit.at_ms, timing) {
            continue;
        }
        let Some(via) = reason(position.node, edit.node, links, thresholds) else {
            continue;
        };
        let is_urgent = via == Via::Same
            && position.mode == PositionMode::Edit
            && edit.at_ms - position.last_ms < timing.same_file_ms;
        let concern = Concern {
            session: position.session.clone(),
            held: position.node,
            mode: position.mode,
            via,
            is_urgent,
        };
        match found
            .iter_mut()
            .find(|kept| kept.session == concern.session)
        {
            Some(kept) if rank(&concern) > rank(kept) => *kept = concern,
            Some(_) => {}
            None => found.push(concern),
        }
    }
    found.sort_by(|a, b| {
        b.is_urgent
            .cmp(&a.is_urgent)
            .then_with(|| a.session.cmp(&b.session))
    });
    found
}

/// Why holding `held` makes an edit of `edited` a concern, if it does.
fn reason<N: Copy + Eq>(
    held: N,
    edited: N,
    links: &[(N, Link)],
    thresholds: &Thresholds,
) -> Option<Via> {
    if held == edited {
        return Some(Via::Same);
    }
    links
        .iter()
        .filter(|(node, _)| *node == held)
        .map(|(_, link)| *link)
        .filter(|link| match link {
            Link::Structural(weight) => *weight >= thresholds.structural,
            Link::CoChange(confidence) => *confidence >= thresholds.cochange,
        })
        .max_by(|a, b| link_rank(*a).total_cmp(&link_rank(*b)))
        .map(Via::Coupled)
}

/// A link's order among reasons: structural before co-change, then by weight.
fn link_rank(link: Link) -> f64 {
    match link {
        Link::Structural(weight) => 2.0 + weight,
        Link::CoChange(confidence) => 1.0 + confidence,
    }
}

/// A concern's order among one session's reasons: urgent, then the same node, then its link.
fn rank<S, N>(concern: &Concern<S, N>) -> (bool, f64) {
    let via = match concern.via {
        Via::Same => 4.0,
        Via::Coupled(link) => link_rank(link),
    };
    (concern.is_urgent, via)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minute in milliseconds.
    const MINUTE: i64 = 60_000;

    /// A position of `session` on `node`, last at `minute`.
    fn held(
        session: &'static str,
        node: u32,
        mode: PositionMode,
        minute: i64,
    ) -> Position<&'static str, u32> {
        Position {
            session,
            node,
            mode,
            last_ms: minute * MINUTE,
            task_ended_ms: None,
        }
    }

    /// Session `a` edits node 1 at minute 20. Worked by hand:
    /// - `x` edited node 1 at minute 15: the same node within 10 minutes, urgent.
    /// - `c` reads node 2 (minute 18), structurally linked to 1 at 0.9: concerned; its read of
    ///   node 3 (co-change 0.6) is a weaker reason for the same session, dropped.
    /// - `d` reads node 4, linked at 0.7 structurally and 0.4 by co-change: below both bars.
    /// - `e` edited node 1 at minute 0 (live by a later event at 12): the edit holds until 30, not
    ///   urgent (20 minutes apart).
    /// - `f` read node 1 at minute 5: the read held until 15, so no longer.
    /// - `g` holds node 2 but its last event was at minute 2: not live.
    /// - `h` claimed node 1 at minute 2, a claim that holds for 2 hours, but its last event was
    ///   at minute 2: not live, so its claim concerns nobody.
    /// - `a`'s own positions concern nobody.
    ///
    /// So: `x` first (urgent, though its name sorts last), then `c` and `e` by session.
    #[test]
    fn an_edit_concerns_the_live_sessions_holding_it_or_its_strong_links() {
        let positions = [
            held("a", 2, PositionMode::Read, 19),
            held("x", 1, PositionMode::Edit, 15),
            held("c", 3, PositionMode::Read, 18),
            held("c", 2, PositionMode::Read, 18),
            held("d", 4, PositionMode::Read, 18),
            held("e", 1, PositionMode::Edit, 0),
            held("f", 1, PositionMode::Read, 5),
            held("g", 2, PositionMode::Read, 2),
            held("h", 1, PositionMode::Claim, 2),
        ];
        let last_event = |session: &&str| match *session {
            "e" => Some(12 * MINUTE),
            "g" | "h" => Some(2 * MINUTE),
            _ => Some(19 * MINUTE),
        };
        let links = [
            (2, Link::Structural(0.9)),
            (3, Link::CoChange(0.6)),
            (4, Link::Structural(0.7)),
            (4, Link::CoChange(0.4)),
        ];
        let edit = EditAt {
            session: &"a",
            node: 1,
            at_ms: 20 * MINUTE,
        };
        let found = concerned(
            edit,
            &positions,
            &last_event,
            (
                &links,
                &Thresholds::of(&CoordinationConfig::default()),
                &Timing::of(&CoordinationConfig::default()),
            ),
        );
        assert_eq!(
            found,
            [
                Concern {
                    session: "x",
                    held: 1,
                    mode: PositionMode::Edit,
                    via: Via::Same,
                    is_urgent: true,
                },
                Concern {
                    session: "c",
                    held: 2,
                    mode: PositionMode::Read,
                    via: Via::Coupled(Link::Structural(0.9)),
                    is_urgent: false,
                },
                Concern {
                    session: "e",
                    held: 1,
                    mode: PositionMode::Edit,
                    via: Via::Same,
                    is_urgent: false,
                },
            ]
        );
    }
}
