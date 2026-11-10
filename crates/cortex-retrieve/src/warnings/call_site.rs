//! Call-site drift: a symbol whose signature the edit changed, still called from files the task has
//! neither read nor edited (those are in the agent's context already), by references tied firmly
//! enough to be stated.

use std::fmt::Write as _;

use cortex_core::{WarningKind, WarningsConfig};
use cortex_store::NodeId;

use super::{Drift, EditFacts, TaskSoFar, Warning};
use crate::support::STRUCTURAL_FLOOR;
use cortex_core::is_plain;

/// The most call sites one warning lists; the count says how many there are.
pub const MAX_LISTED_CALL_SITES: usize = 5;

/// A warning for each changed symbol still called from a file the task has neither read nor
/// edited, naming those files not named for that symbol before in the task.
pub(super) fn warnings(facts: &EditFacts, task: &TaskSoFar, _: &WarningsConfig) -> Vec<Warning> {
    facts
        .drifts
        .iter()
        .filter_map(|drift| warning(facts, task, drift))
        .collect()
}

/// The warning for `drift`, if any call site is left to tell of.
fn warning(facts: &EditFacts, task: &TaskSoFar, drift: &Drift) -> Option<Warning> {
    if !is_plain(&drift.name) {
        return None;
    }
    let sites: Vec<_> = drift
        .call_sites
        .iter()
        .filter(|site| {
            let file = site.file.node;
            site.confidence >= STRUCTURAL_FLOOR
                && file != facts.edited.node
                && !task.touched.contains(&file)
                && task.is_new(WarningKind::CallSite, drift.symbol, file)
                && is_plain(site.file.path.as_str())
        })
        .collect();
    let listed = sites
        .get(..sites.len().min(MAX_LISTED_CALL_SITES))
        .unwrap_or_default();
    let strength = listed
        .iter()
        .map(|site| site.confidence)
        .max_by(f64::total_cmp)?;
    let mut targets: Vec<NodeId> = Vec::new();
    for site in listed {
        if !targets.contains(&site.file.node) {
            targets.push(site.file.node);
        }
    }
    let at_least = if drift.unchecked > 0 { "at least " } else { "" };
    let count = if sites.len() == 1 {
        format!("{at_least}1 call site")
    } else {
        format!("{at_least}{} call sites", sites.len())
    };
    // The index knows a file refers to the symbol, not whether the call still fits: the line says
    // what to look at, never that the call is broken.
    let mut line = format!("{}'s signature changed; check {count}: ", drift.name);
    for (at, site) in listed.iter().enumerate() {
        let comma = if at == 0 { "" } else { ", " };
        let _ = write!(line, "{comma}{}:{}", site.file.path, site.line);
    }
    if sites.len() > MAX_LISTED_CALL_SITES {
        line.push_str(", ...");
    }
    Some(Warning {
        kind: WarningKind::CallSite,
        source: drift.symbol,
        targets,
        strength,
        line,
    })
}
