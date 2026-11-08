//! The positions each session holds, kept by the writer as it records each tool call: the node of
//! every file a session read, or edited, with the position that holds longer of the one kept and
//! the new one, the kept one held as its task's end allows. A call that failed changed nothing and
//! moves no position.

use cortex_coord::{Position, PositionMode, Timing};
use cortex_core::{Event, HookKind, ToolKind};
use cortex_store::{PositionRecord, PositionRow, StoreError, WriteTx};

/// Keeps the positions `event` gives its session in `transaction`: a read of a file, or an edit of
/// one or several; nothing for any other event.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn keep(
    transaction: &mut WriteTx<'_>,
    event: &Event,
    timing: &Timing,
) -> Result<(), StoreError> {
    let Some(mode) = mode_of(event) else {
        return Ok(());
    };
    let Some(target) = &event.tool_target else {
        return Ok(());
    };
    for path in target.paths() {
        let Some(node) = transaction.queries().file_node_at(path)? else {
            continue;
        };
        let kept = transaction.queries().position_of(&event.session_id, node)?;
        // The new position's task is open: the event belongs to it.
        let new = PositionRow {
            record: PositionRecord {
                session_id: event.session_id.clone(),
                node,
                mode,
                since_ms: event.ts_ms,
                last_ms: event.ts_ms,
            },
            task_ended_ms: None,
        };
        let longer = Position::longer_of(kept.as_ref().map(rule_of), rule_of(&new), timing);
        if kept.as_ref().is_some_and(|kept| rule_of(kept) == longer) {
            continue;
        }
        // The same mode taken again keeps when it was first taken.
        let since_ms = kept
            .filter(|kept| kept.record.mode == mode)
            .map_or(event.ts_ms, |kept| kept.record.since_ms);
        let new = new.record;
        transaction.keep_position(&PositionRecord { since_ms, ..new })?;
    }
    Ok(())
}

/// The mode of the position `event` gives: a successful read or edit after it ran.
fn mode_of(event: &Event) -> Option<PositionMode> {
    if event.hook != HookKind::PostTool
        || event
            .tool_result
            .as_ref()
            .is_some_and(|result| !result.is_success)
    {
        return None;
    }
    match event.tool? {
        ToolKind::Read => Some(PositionMode::Read),
        tool if tool.is_edit() => Some(PositionMode::Edit),
        _ => None,
    }
}

/// `row` as the rules read a position.
fn rule_of(row: &PositionRow) -> Position<&str, i64> {
    Position {
        session: row.record.session_id.as_str(),
        node: row.record.node.get(),
        mode: row.record.mode,
        last_ms: row.record.last_ms,
        task_ended_ms: row.task_ended_ms,
    }
}

#[cfg(test)]
mod tests;
