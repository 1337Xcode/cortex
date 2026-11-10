//! The warnings an edit gets: which of the edited file's symbols changed their signature, what the
//! store says about the file, what its task did so far, and the rules' verdict, with the rows the
//! writer records once the warnings reach the agent.
//!
//! A changed signature is found by reading the edited file's definitions again and comparing them
//! with the stored ones, as the writer would on its next index. When the watcher's index came first
//! and the store already holds the file as it is, the signature changes that index found, which the
//! writer keeps a short while, stand in. The edited file is read once and parsed at most once
//! for the warnings and the notices' contracts together. Nothing is written here.

use cortex_core::{Event, ScaleMode};
use cortex_retrieve::{EditQuery, edit_facts, task_so_far, warnings_after_edit};
use cortex_store::{FileRow, Queries, StoreError, SymbolId, TaskRow};

use super::on_disk::OnDisk;
use super::{Answer, Coordination, Service, mode_of, open_task_of};
use crate::claim::Claim;
use crate::given::{Given, text_record};
use crate::signatures::Signatures;
use crate::told::{Told, pairs_of};

impl Service {
    /// The warnings `event` gets, an edit's tool result, and the rows to record of them once they
    /// reach the agent: none for an event that is no successful edit of a file the store holds,
    /// outside a task, or before the first index. `signatures` holds the signature changes the
    /// writer's indexes found lately. The warnings are decided while no other decision runs, with
    /// the pairs the task was told so far, recorded or not yet, so each is told once a task. The
    /// answer also carries what the edit's coordination found, whatever its own task is given,
    /// for [`Service::with_notices`], and the signatures of the definitions the edit changed for
    /// the notices' contracts while `coordination.contracts` is on. A store that cannot be read
    /// gives nothing, and the error is logged; coordination and the warnings stand apart, so that
    /// one failing leaves the other, and signatures that cannot be read leave the notices naming
    /// the definitions alone.
    pub(crate) fn post_edit(
        &self,
        event: &Event,
        (signatures, told): (&Signatures, &Told),
    ) -> Answer {
        let Some(path) = event.edited_file() else {
            return Answer::default();
        };
        let answered = self.with_reader(|queries| {
            let Some(mode) = mode_of(queries)? else {
                return Ok(Answer::default());
            };
            let task = open_task_of(queries, event)?;
            let row = queries.file(path)?;
            let root = self.root_for(event);
            let (drifted, signed) = match (&task, &row) {
                (Some(task), Some(row)) => {
                    let disk = OnDisk::read(root, &row.file, self.max_file_bytes);
                    let window = (task.record.started_at_ms, event.ts_ms);
                    let edited = (row, disk.as_ref());
                    let drifted = self.drifted(queries, edited, (signatures, window))?;
                    let signed = self
                        .signed(queries, (edited, root), &drifted, (signatures, window))
                        .unwrap_or_else(|error| {
                            tracing::warn!(error = %format!("{error:#}"), "an edit's signatures failed");
                            Vec::new()
                        });
                    (drifted, signed)
                }
                _ => (Vec::new(), Vec::new()),
            };
            let changed = names_of(queries, row.as_ref(), &drifted)?;
            let coordination = self
                .coordinate(queries, event, (&changed, &signed))
                .unwrap_or_else(|error| {
                    tracing::warn!(error = %format!("{error:#}"), "an edit's coordination failed");
                    Coordination::default()
                });
            let mut answer = match (task, row) {
                // INVARIANT: silent-when-unsure (a held-out task is given nothing Cortex would say)
                (Some(task), Some(row)) if !self.is_held_out_task(&task) => self
                    .warned(queries, event, (&task, &row), (&drifted, told, mode))
                    .unwrap_or_else(|error| {
                        tracing::warn!(error = %format!("{error:#}"), "an edit's warnings failed");
                        Answer::default()
                    }),
                _ => Answer::default(),
            };
            answer.coordination = coordination;
            Ok(answer)
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "an edit's warnings failed");
            Answer::default()
        })
    }

    /// The warnings `event`, an edit of `row`'s file in `task`, gets in `mode`, `drifted` being the
    /// stored symbols whose signature it changed, decided with what `told` says the task was told,
    /// and the rows to record of them.
    fn warned(
        &self,
        queries: Queries<'_>,
        event: &Event,
        (task, row): (&TaskRow, &FileRow),
        (drifted, told, mode): (&[SymbolId], &Told, ScaleMode),
    ) -> Result<Answer, cortex_retrieve::RetrieveError> {
        let task_id = task.record.task_id;
        let asked = EditQuery {
            root: self.root_for(event),
            edited: &row.file.path,
            drifted,
            link_decay: self.edits.link_decay,
            max_file_bytes: self.max_file_bytes,
        };
        let Some(facts) = edit_facts(queries, &asked)? else {
            return Ok(Answer::default());
        };
        let mut so_far = task_so_far(queries, task_id, facts.edited.node)?;
        let warned = told.deciding(task_id, |told| {
            so_far.warned.extend(told.iter().copied());
            warnings_after_edit(&facts, &so_far, &self.edits.warnings, mode)
        });
        let rows = warned.records(task_id, event.ts_ms);
        let given = warned
            .text
            .as_deref()
            .filter(|_| !rows.is_empty())
            .map(|text| text_record(event, Some(task_id), text));
        let claims = warned
            .text
            .is_some()
            .then(|| Claim::Warned(task_id, pairs_of(&warned)));
        Ok(Answer {
            text: warned.text,
            given: given
                .map(|text| Given::Warnings(rows, text))
                .into_iter()
                .collect(),
            claims: claims.into_iter().collect(),
            ..Answer::default()
        })
    }

    /// The stored symbols of `row`'s file whose signature `disk`, the file as it is on disk,
    /// changed: read again and compared, or, when the store already holds the file as it is,
    /// those `signatures` says an index changed between the task's start and now (`since_ms`,
    /// `now_ms`). None for a file the index did not parse (a secret's name, too large, binary,
    /// generated or of no known language), which is never opened, or one that cannot be read or
    /// parsed in time.
    fn drifted(
        &self,
        queries: Queries<'_>,
        (row, disk): (&FileRow, Option<&OnDisk<'_>>),
        (signatures, (since_ms, now_ms)): (&Signatures, (i64, i64)),
    ) -> Result<Vec<SymbolId>, cortex_retrieve::RetrieveError> {
        let Some(disk) = disk else {
            return Ok(Vec::new());
        };
        if row.file.content_hash == Some(disk.hash()) {
            let symbols: Vec<SymbolId> = queries
                .symbols_in_file(row.id)?
                .iter()
                .map(|symbol| symbol.id)
                .collect();
            return Ok(signatures.changed_among(&symbols, (since_ms, now_ms)));
        }
        let reading = (&self.edits.grammars, self.edits.parse_timeout);
        let Some(extraction) = disk.definitions(reading) else {
            return Ok(Vec::new());
        };
        Ok(queries.signature_changes(row.id, &extraction.symbols)?)
    }
}

/// The qualified names of `drifted`, the stored symbols of `row`'s file whose signature an edit
/// changed, in the file's order; none without the file or a changed signature.
fn names_of(
    queries: Queries<'_>,
    row: Option<&FileRow>,
    drifted: &[SymbolId],
) -> Result<Vec<String>, StoreError> {
    let Some(row) = row.filter(|_| !drifted.is_empty()) else {
        return Ok(Vec::new());
    };
    Ok(queries
        .symbols_in_file(row.id)?
        .into_iter()
        .filter(|symbol| drifted.contains(&symbol.id))
        .map(|symbol| symbol.qual_name.as_str().to_owned())
        .collect())
}
