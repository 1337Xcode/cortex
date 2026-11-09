//! The warnings an edit gets: which of the edited file's symbols changed their signature, what the
//! store says about the file, what its task did so far, and the rules' verdict, with the rows the
//! writer records once the warnings reach the agent.
//!
//! A changed signature is found by reading the edited file's definitions again and comparing them
//! with the stored ones, as the writer would on its next index. When the watcher's index came first
//! and the store already holds the file as it is, the signature changes that index found, which the
//! writer keeps a short while, stand in. Nothing is written here.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use cortex_core::{ContentHash, Event, FileFlags, ScaleMode};
use cortex_index::{Extractor, open_regular};
use cortex_retrieve::{EditQuery, edit_facts, task_so_far, warnings_after_edit};
use cortex_store::{FileRecord, FileRow, Queries, StoreError, SymbolId, TaskRow};

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
    /// for [`Service::with_notices`]. A store that cannot be read gives nothing, and the error is
    /// logged; coordination and the warnings stand apart, so that one failing leaves the other.
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
            let drifted = match (&task, &row) {
                (Some(task), Some(row)) => {
                    let since = (signatures, (task.record.started_at_ms, event.ts_ms));
                    self.drifted(queries, row, since)?
                }
                _ => Vec::new(),
            };
            let changed = names_of(queries, row.as_ref(), &drifted)?;
            let coordination = self
                .coordinate(queries, event, &changed)
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
            root: &self.root,
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

    /// The stored symbols of `row`'s file whose signature the file on disk changed: read again and
    /// compared, or, when the store already holds the file as it is, those `signatures` says an
    /// index changed between the task's start and now (`since_ms`, `now_ms`). None for a file the
    /// index did not parse (a secret's name, too large, binary, generated or of no known language),
    /// which is never opened here, or one that cannot be read or parsed in time.
    fn drifted(
        &self,
        queries: Queries<'_>,
        row: &FileRow,
        (signatures, (since_ms, now_ms)): (&Signatures, (i64, i64)),
    ) -> Result<Vec<SymbolId>, cortex_retrieve::RetrieveError> {
        let Some(language) = row.file.language else {
            return Ok(Vec::new());
        };
        let Some(bytes) = parsed_bytes(&self.root, &row.file, self.max_file_bytes) else {
            return Ok(Vec::new());
        };
        if row.file.content_hash == Some(ContentHash::of(&bytes)) {
            let symbols: Vec<SymbolId> = queries
                .symbols_in_file(row.id)?
                .iter()
                .map(|symbol| symbol.id)
                .collect();
            return Ok(signatures.changed_among(&symbols, (since_ms, now_ms)));
        }
        let mut extractor =
            Extractor::new(Arc::clone(&self.edits.grammars), self.edits.parse_timeout);
        let Ok(extraction) = extractor.extract(&row.file.path, language, &bytes) else {
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

/// The bytes of `file`, in the repository at `root`, when the index parsed it for its definitions:
/// one whose name matches a secret pattern, too large or binary was never read and is not opened
/// here, and a generated or vendored one was only listed. Read as the index reads files: never
/// through a link, never anything but a regular file, at most `max_bytes`; none otherwise.
fn parsed_bytes(root: &Path, file: &FileRecord, max_bytes: u64) -> Option<Vec<u8>> {
    let is_parsed =
        file.flags.contains(FileFlags::PARSED) && !file.flags.contains(FileFlags::SECRET_PATTERN);
    if !is_parsed {
        return None;
    }
    let file = open_regular(&file.path.to_native(root)).ok()?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (u64::try_from(bytes.len()).ok()? <= max_bytes).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use cortex_core::{Language, RepoPath};

    use super::*;

    /// The record of `path` as the index would store it with `flags`.
    fn record(path: &str, flags: FileFlags) -> Result<FileRecord, Box<dyn std::error::Error>> {
        Ok(FileRecord {
            path: RepoPath::new(path)?,
            language: Some(Language::Python),
            size_bytes: 6,
            line_count: Some(1),
            content_hash: None,
            mtime_ns: 1,
            flags,
            indexed_at_ms: 1,
        })
    }

    /// A parsed file's bytes are read, at most the cap; the same file flagged as a secret's, or
    /// never parsed, is not, though it is there to read.
    #[test]
    fn only_a_parsed_file_is_read() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        std::fs::write(root.path().join("app.py"), "x = 1\n")?;
        let parsed = FileFlags::PARSED;
        let read = |flags, max_bytes| -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
            Ok(parsed_bytes(
                root.path(),
                &record("app.py", flags)?,
                max_bytes,
            ))
        };
        assert_eq!(read(parsed, 6)?, Some(b"x = 1\n".to_vec()));
        assert_eq!(read(parsed, 5)?, None);
        assert_eq!(read(parsed.with(FileFlags::SECRET_PATTERN), 6)?, None);
        assert_eq!(read(FileFlags::empty(), 6)?, None);
        assert_eq!(read(FileFlags::GENERATED, 6)?, None);
        Ok(())
    }
}
