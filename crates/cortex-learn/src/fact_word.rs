//! The user's word on one fact: it holds for the code as it is now (confirmed: active, its anchors
//! stamped anew), it no longer holds (retired), or it is corrected (a new fact of the user's on the
//! same anchors supersedes it, the old one kept as history). A fact another one corrects is not
//! changed: the correction is the one to change.

use cortex_core::{FactSource, FactStatus};
use cortex_store::{FactId, FactRecord, FactRow, NodeId, Queries, WriteTx};

use crate::facts::{FactError, anchors_now};

/// Confirms fact `id` against the code as it is now, at `now_ms` in `transaction`: active, its
/// anchors stamped with the hashes they hold now, whatever its status was.
///
/// # Errors
///
/// [`FactError::Unknown`] for no such fact, [`FactError::Superseded`] for one another fact
/// corrects, [`FactError::Unanchorable`] when an anchor's node holds no hash now (its file is gone
/// or no longer read), and the store's errors.
pub fn confirm_fact(
    transaction: &mut WriteTx<'_>,
    id: FactId,
    now_ms: i64,
) -> Result<(), FactError> {
    let row = current(transaction.queries(), id)?;
    let anchors = anchors_now(transaction.queries(), &nodes_of(&row))?;
    transaction.restamp_fact(id, &anchors)?;
    transaction.set_fact_status(id, FactStatus::Active, now_ms)?;
    Ok(())
}

/// Retires fact `id` at `now_ms` in `transaction`: it is never injected again.
///
/// # Errors
///
/// [`FactError::Unknown`], [`FactError::Superseded`] and the store's errors.
pub fn retire_fact(
    transaction: &mut WriteTx<'_>,
    id: FactId,
    now_ms: i64,
) -> Result<(), FactError> {
    current(transaction.queries(), id)?;
    transaction.set_fact_status(id, FactStatus::Retired, now_ms)?;
    Ok(())
}

/// Corrects fact `id` with `text` at `now_ms` in `transaction`: a new active fact of the user's, of
/// the same kind, on the same anchors stamped with their hashes now, supersedes it. The new fact.
///
/// # Errors
///
/// [`FactError::Unknown`], [`FactError::Superseded`], [`FactError::Unanchorable`] and the store's
/// errors, text that is too long among them.
pub fn edit_fact(
    transaction: &mut WriteTx<'_>,
    id: FactId,
    text: &str,
    now_ms: i64,
) -> Result<FactId, FactError> {
    let row = current(transaction.queries(), id)?;
    let anchors = anchors_now(transaction.queries(), &nodes_of(&row))?;
    let new = transaction.add_fact(&FactRecord {
        text: text.to_owned(),
        kind: row.record.kind,
        source: FactSource::User,
        status: FactSource::User.first_status(),
        created_at_ms: now_ms,
        task: None,
        anchors,
    })?;
    transaction.supersede_fact(id, new)?;
    Ok(new)
}

/// Fact `id`, when it exists and no other fact supersedes it.
fn current(queries: Queries<'_>, id: FactId) -> Result<FactRow, FactError> {
    let row = queries.fact(id)?.ok_or(FactError::Unknown { fact: id })?;
    match row.superseded_by {
        Some(by) => Err(FactError::Superseded { fact: id, by }),
        None => Ok(row),
    }
}

/// The nodes `row`'s anchors are on, in node order.
fn nodes_of(row: &FactRow) -> Vec<NodeId> {
    row.record
        .anchors
        .iter()
        .map(|anchor| anchor.node)
        .collect()
}
