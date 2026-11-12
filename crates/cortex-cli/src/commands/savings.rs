//! What Cortex has measured it saves, in one line: the holdout comparison's median input tokens per
//! successful task, Cortex on minus held out, with its 95% interval, or how far the two arms still
//! are from being compared. `cortex status` prints the long form and the Claude Code status line
//! the short one. No saving is stated without the comparison: until there is one, both say so.

use cortex_meter::Holdout;

use super::receipts::{grouped, quantity};

/// The savings line `cortex status` prints, starting `savings: `.
pub(crate) fn status_line(holdout: &Holdout) -> String {
    let needed = holdout.needed_per_arm;
    let (held_out, treated) = (holdout.arms.held_out, holdout.arms.treated);
    if !holdout.arms.is_enough() {
        return format!(
            "savings: not measured yet, {} of {needed} held-out and {} of {needed} treated tasks \
             so far",
            grouped(held_out),
            grouped(treated)
        );
    }
    let Some(comparison) = &holdout.comparison else {
        return format!(
            "savings: not measured yet, fewer than {needed} tasks of each arm have ended"
        );
    };
    let Some([input, ..]) = &comparison.per_success else {
        return format!(
            "savings: not measured yet, fewer than {needed} successful tasks of each arm have their \
             usage read"
        );
    };
    let [with_cortex, held] = comparison.per_success_tasks;
    format!(
        "savings: median input tokens per successful task, with Cortex minus held out: {} \
         ({} to {}, 95%), from {} and {} successful tasks",
        signed(input.difference),
        signed(input.low),
        signed(input.high),
        grouped(with_cortex),
        grouped(held)
    )
}

/// The savings part of the status line: the comparison's difference in input tokens per
/// successful task with its interval, or how many tasks each arm holds against what it needs.
pub(crate) fn short(holdout: &Holdout) -> String {
    let needed = holdout.needed_per_arm;
    let per_success = holdout
        .comparison
        .as_ref()
        .and_then(|comparison| comparison.per_success.as_ref());
    match per_success {
        Some([input, ..]) => format!(
            "input per task vs held out {} ({} to {})",
            signed(input.difference),
            signed(input.low),
            signed(input.high)
        ),
        None => format!(
            "savings not measured yet ({}/{needed} held out, {}/{needed} with Cortex)",
            grouped(holdout.arms.held_out),
            grouped(holdout.arms.treated)
        ),
    }
}

/// `value` as `quantity` writes it, with a `+` before a positive one.
fn signed(value: f64) -> String {
    let text = quantity(value);
    if value > 0.0 {
        format!("+{text}")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use cortex_meter::{Arms, Compared, Comparison};

    use super::*;

    /// A comparison whose success rates are equal and whose medians of input differ by `input`.
    fn compared(input: Compared) -> Comparison {
        let same = Compared {
            treated: 1.0,
            held_out: 1.0,
            difference: 0.0,
            low: 0.0,
            high: 0.0,
        };
        Comparison {
            tasks: [40, 31],
            success_rate: same,
            per_success_tasks: [38, 30],
            per_success: Some([input, same, same, same]),
        }
    }

    /// A holdout of `held_out` and `treated` tasks with `comparison`.
    fn holdout(held_out: u64, treated: u64, comparison: Option<Comparison>) -> Holdout {
        Holdout {
            arms: Arms { held_out, treated },
            needed_per_arm: 30,
            comparison,
        }
    }

    /// Below 30 a side, both forms count the tasks against 30 and state no saving; with a
    /// comparison they give the median difference in input tokens and its interval, signed, digits
    /// grouped: 1,550 with Cortex against 18,500 held out is -16,950.
    #[test]
    fn savings_are_stated_only_from_the_comparison() {
        let few = holdout(12, 1_200, None);
        assert_eq!(
            status_line(&few),
            "savings: not measured yet, 12 of 30 held-out and 1,200 of 30 treated tasks so far"
        );
        assert_eq!(
            short(&few),
            "savings not measured yet (12/30 held out, 1,200/30 with Cortex)"
        );
        let unended = holdout(31, 40, None);
        assert_eq!(
            status_line(&unended),
            "savings: not measured yet, fewer than 30 tasks of each arm have ended"
        );
        let input = Compared {
            treated: 1_550.0,
            held_out: 18_500.0,
            difference: -16_950.0,
            low: -18_000.0,
            high: -15_000.5,
        };
        let measured = holdout(31, 40, Some(compared(input)));
        assert_eq!(
            status_line(&measured),
            "savings: median input tokens per successful task, with Cortex minus held out: \
             -16,950 (-18,000 to -15,000.5, 95%), from 38 and 30 successful tasks"
        );
        assert_eq!(
            short(&measured),
            "input per task vs held out -16,950 (-18,000 to -15,000.5)"
        );
        let mut unread = compared(input);
        unread.per_success = None;
        let thin = holdout(31, 40, Some(unread));
        assert_eq!(
            status_line(&thin),
            "savings: not measured yet, fewer than 30 successful tasks of each arm have their \
             usage read"
        );
        assert_eq!(
            short(&thin),
            "savings not measured yet (31/30 held out, 40/30 with Cortex)"
        );
    }

    /// A difference above zero, Cortex costing more, is written with its sign.
    #[test]
    fn a_cost_is_signed_too() {
        let input = Compared {
            treated: 2_000.0,
            held_out: 1_000.0,
            difference: 1_000.0,
            low: -200.0,
            high: 2_100.0,
        };
        assert_eq!(
            short(&holdout(30, 30, Some(compared(input)))),
            "input per task vs held out +1,000 (-200 to +2,100)"
        );
    }
}
