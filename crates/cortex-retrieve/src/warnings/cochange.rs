//! The co-change companion: a file that usually changes with the edited one, by git history or by
//! agents' past work, and that the task has neither read nor edited.

use cortex_core::{WarningKind, WarningsConfig};
use cortex_learn::MAX_LINK_WEIGHT;

use super::{EditFacts, NamedFile, TaskSoFar, Warning};
use crate::plain::is_plain;

/// The least weight, after fading, a co-access link needs before an edit that leaves its other
/// file untouched is worth a warning: about ten successful tasks that edited both files.
pub const MIN_LINK_WEIGHT: f64 = 1.0;

/// The fewest tasks that must have drawn a co-access link before it is worth a warning.
pub const MIN_LINK_TASKS: u32 = 3;

/// A warning for each partner of the edited file strong enough in history or past work and not
/// touched in the task, each partner once: by history when both say so.
pub(super) fn warnings(
    facts: &EditFacts,
    task: &TaskSoFar,
    config: &WarningsConfig,
) -> Vec<Warning> {
    let edited = &facts.edited;
    let is_open = |file: &NamedFile| {
        file.node != edited.node
            && !task.touched.contains(&file.node)
            && task.is_new(WarningKind::CoChange, edited.node, file.node)
            && is_plain(file.path.as_str())
    };
    let by_history = facts
        .partners
        .iter()
        .filter(|partner| {
            partner.confidence >= config.cochange_min_confidence
                && partner.co_changes >= config.cochange_min_count
                && is_open(&partner.file)
        })
        .map(|partner| {
            let line = format!(
                "Edits to {} usually come with {} ({} of {} changes); not touched yet.",
                edited.path, partner.file.path, partner.co_changes, facts.commits
            );
            warning(edited.node, &partner.file, partner.confidence, line)
        });
    let by_work = facts
        .linked
        .iter()
        .filter(|linked| {
            linked.weight >= MIN_LINK_WEIGHT
                && linked.tasks >= MIN_LINK_TASKS
                && is_open(&linked.file)
        })
        .map(|linked| {
            let line = format!(
                "Edits to {} usually come with {} (worked on together in {} earlier tasks); not \
                 touched yet.",
                edited.path, linked.file.path, linked.tasks
            );
            let strength = (linked.weight / MAX_LINK_WEIGHT).min(1.0);
            warning(edited.node, &linked.file, strength, line)
        });
    let mut warnings: Vec<Warning> = Vec::new();
    for found in by_history.chain(by_work) {
        if !warnings.iter().any(|kept| kept.targets == found.targets) {
            warnings.push(found);
        }
    }
    warnings
}

/// A co-change warning about `source` naming `partner`.
fn warning(
    source: cortex_store::NodeId,
    partner: &NamedFile,
    strength: f64,
    line: String,
) -> Warning {
    Warning {
        kind: WarningKind::CoChange,
        source,
        targets: vec![partner.node],
        strength,
        line,
    }
}
