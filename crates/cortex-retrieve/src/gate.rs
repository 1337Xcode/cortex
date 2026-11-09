//! The gate: whether what retrieval found is sure enough to hand an agent.
//!
//! Silence is the answer unless a rule clears. Until a model calibrated on the repository's own
//! injections exists, the rule is the cold-start one: the signals agree, at least two independent
//! kinds of evidence supporting one of the top three files while the prompt's words match some file
//! strongly, or three kinds while they match some file at all; or the session is plainly continuing
//! its own work. The more independent evidence agrees, the less the prompt's wording has to: a
//! prompt that names a change in its own words ("stop creating the stats file") matches the code
//! weakly though the code it means is plain from the graph. A missing signal lowers nothing into
//! a yes; it only leaves the gate shut.

use cortex_core::GateFeatures;

/// How many independent kinds of evidence must support one top file for them to agree.
pub const AGREEING_KINDS: u8 = 2;

/// The top lexical score the cold-start rule asks for, before any raise the self-check made: half
/// of the prompt's searchable words, weighed by how rare each is, matched by one file.
pub const LEXICAL_BAR: f64 = 0.5;

/// How many independent kinds of evidence make strong agreement, which needs only
/// [`STRONG_LEXICAL_BAR`] of the prompt's words.
pub const STRONG_AGREEING_KINDS: u8 = 3;

/// The top lexical score strong agreement asks for, before any raise: a tenth of the prompt's
/// searchable words, weighed by rarity, matched by one file, so a prompt matching nothing gets
/// nothing. On the evaluation suite, the work orders three kinds agreed on while the prompt
/// matched between 0.14 and 0.29 were each used.
pub const STRONG_LEXICAL_BAR: f64 = 0.1;

/// What the gate says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Hand it over: the rule that cleared.
    Inject(Rule),
    /// Say nothing: why not.
    Silent(Shut),
}

/// The rule that let a work order through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Independent evidence agreed and the prompt matched strongly.
    Agreement,
    /// The session is continuing its own work.
    Continuity,
}

/// Why the gate stayed shut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shut {
    /// No top file had enough independent kinds of evidence behind it.
    TooFewKinds,
    /// The prompt's words matched no file strongly enough.
    WeakMatch,
    /// The spreading ran out of its time before it finished: what it ranked would hang on how fast
    /// the machine was, so it is not shown.
    OutOfTime,
}

/// Whether to hand over what `features` describe, the cold-start rule's lexical bars raised by
/// `raise` (the self-check's). A raise only ever lifts the bars: one below zero counts as none. The
/// continuity rule needs no lexical bar while there is no raise; once the self-check raises one,
/// the prompt's best match must reach the raise there too, so work orders for continuing work that
/// go unused are held back like the others.
#[must_use]
pub fn decide(features: &GateFeatures, raise: f64) -> Verdict {
    let raise = raise.max(0.0);
    let is_agreed = features.agreeing_kinds >= AGREEING_KINDS;
    let is_strong = features.top_lexical >= LEXICAL_BAR + raise;
    let is_strongly_agreed = features.agreeing_kinds >= STRONG_AGREEING_KINDS
        && features.top_lexical >= STRONG_LEXICAL_BAR + raise;
    if (is_agreed && is_strong) || is_strongly_agreed {
        return Verdict::Inject(Rule::Agreement);
    }
    if features.is_continuing && features.top_lexical >= raise {
        return Verdict::Inject(Rule::Continuity);
    }
    // INVARIANT: silent-when-unsure (no rule cleared, so nothing is injected: there is no
    // fallback, default span or best-effort answer below the gate)
    Verdict::Silent(if is_agreed {
        Shut::WeakMatch
    } else {
        Shut::TooFewKinds
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate's features with nothing behind them.
    fn none() -> GateFeatures {
        GateFeatures {
            top_lexical: 0.0,
            top_mass: 0.0,
            agreeing_kinds: 0,
            supporting_kinds: 0,
            similar_task: 0.0,
            recent_precision: None,
            is_continuing: false,
        }
    }

    /// The cold-start rule, case by case: two kinds agreeing on a file with the prompt matching at
    /// the bar or above clears; one kind, or a match below the bar, does not, whatever else is
    /// strong; a continuing session clears alone; nothing clears nothing. A raise of 0.1 lifts the
    /// bar to 0.6, so a match of 0.55 that cleared no longer does; a raise below zero lowers
    /// nothing, so a match of 0 or 0.499 still does not clear. A continuing session with no match
    /// clears with no raise and with one below zero, but not under a raise of 0.1; with a match of
    /// 0.1 it clears under that raise, with 0.05 it does not. Three kinds agreeing clear with a
    /// match of 0.1 or more (0.14, as on the suite), not 0.09; a raise of 0.1 lifts that bar to 0.2,
    /// so 0.15 no longer clears and 0.25 does.
    #[test]
    fn the_cold_start_rule_decides_case_by_case() {
        let agreed = |top_lexical| GateFeatures {
            top_lexical,
            agreeing_kinds: 2,
            supporting_kinds: 3,
            ..none()
        };
        let strongly = |top_lexical| GateFeatures {
            top_lexical,
            agreeing_kinds: 3,
            supporting_kinds: 3,
            ..none()
        };
        let continuing = |top_lexical| GateFeatures {
            top_lexical,
            is_continuing: true,
            ..none()
        };
        let cases = [
            (agreed(0.5), 0.0, Verdict::Inject(Rule::Agreement)),
            (agreed(0.9), 0.0, Verdict::Inject(Rule::Agreement)),
            (agreed(0.499), 0.0, Verdict::Silent(Shut::WeakMatch)),
            (agreed(0.55), 0.1, Verdict::Silent(Shut::WeakMatch)),
            (agreed(0.6), 0.1, Verdict::Inject(Rule::Agreement)),
            (agreed(0.0), -0.5, Verdict::Silent(Shut::WeakMatch)),
            (agreed(0.499), -0.1, Verdict::Silent(Shut::WeakMatch)),
            (agreed(0.3), 0.0, Verdict::Silent(Shut::WeakMatch)),
            (strongly(0.1), 0.0, Verdict::Inject(Rule::Agreement)),
            (strongly(0.14), 0.0, Verdict::Inject(Rule::Agreement)),
            (strongly(0.09), 0.0, Verdict::Silent(Shut::WeakMatch)),
            (strongly(0.0), -0.5, Verdict::Silent(Shut::WeakMatch)),
            (strongly(0.15), 0.1, Verdict::Silent(Shut::WeakMatch)),
            (strongly(0.25), 0.1, Verdict::Inject(Rule::Agreement)),
            (
                GateFeatures {
                    top_lexical: 1.0,
                    agreeing_kinds: 1,
                    supporting_kinds: 4,
                    top_mass: 0.9,
                    similar_task: 0.9,
                    recent_precision: Some(1.0),
                    ..none()
                },
                0.0,
                Verdict::Silent(Shut::TooFewKinds),
            ),
            (
                GateFeatures {
                    is_continuing: true,
                    ..none()
                },
                0.0,
                Verdict::Inject(Rule::Continuity),
            ),
            (continuing(0.0), -0.5, Verdict::Inject(Rule::Continuity)),
            (continuing(0.0), 0.1, Verdict::Silent(Shut::TooFewKinds)),
            (continuing(0.1), 0.1, Verdict::Inject(Rule::Continuity)),
            (continuing(0.05), 0.1, Verdict::Silent(Shut::TooFewKinds)),
            (none(), 0.0, Verdict::Silent(Shut::TooFewKinds)),
        ];
        for (features, raise, expected) in cases {
            assert_eq!(
                decide(&features, raise),
                expected,
                "{features:?} raised {raise}"
            );
        }
    }
}
