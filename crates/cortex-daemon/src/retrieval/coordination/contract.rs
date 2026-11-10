//! Interface contracts in coordination notices, as the daemon works them out, while
//! `coordination.contracts` is on.
//!
//! At an edit, each definition whose signature it changed is found in the edited file as read for
//! the edit's warnings, for its signature after. Its signature before is the text of the header
//! whose hash the store held before the change: the stored hash when the edit's hook comes before
//! the index of the file, or the one the index's change recorded when it came first. The text is
//! taken from the headers the index read since the daemon started ([`Headers`]), or else from the
//! file as HEAD's commit holds it, parsed; a text whose hash is not the vouched one is never said.
//! A notice queued for a session holding a file that calls some of these definitions keeps its
//! contract in memory beside it.
//!
//! At that session's next hook one contract at most is checked against the files on disk: each
//! signature after must still be the edited file's (read again by its hash, parsed only when it
//! changed), and the lines that call it come from the index only while the held file is the one it
//! read. A notice whose contract the files bear out, and that fits the token cap, is given in the
//! contract's words; any other is given in the words that name the definitions alone.

use std::cell::OnceCell;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cortex_coord::{Called, Holding, Party, Signature, contract_to_holder};
use cortex_core::{CoordinationConfig, Identifier, RefKind, RepoPath, ShortHash, estimate_tokens};
use cortex_git::GixHistory;
use cortex_index::{Extraction, Grammars};
use cortex_retrieve::{CheckedFile, STRUCTURAL_FLOOR};
use cortex_store::{FileId, FileRow, NoticeId, NoticeRow, Queries, StoreError, SymbolId};

use super::super::Service;
use super::super::on_disk::OnDisk;
use crate::contracts::{Changed, Contract, Contracts, Definitions, NoticeKey};
use crate::headers::{signature_at, signatures_in};
use crate::signatures::Signatures;

/// A definition whose signature an edit changed, with its signatures when both can be said.
pub(crate) type Signed = (SymbolId, Option<Changed>);

/// What contracts are worked out with.
#[derive(Clone, Copy)]
pub(super) struct Rendering<'r> {
    /// The `coordination.*` settings: `coordination.contracts` and `coordination.max_tokens`.
    pub(super) config: &'r CoordinationConfig,
    /// Every language's grammar, to read a file's definitions again.
    pub(super) grammars: &'r Arc<Grammars>,
    /// `index.parse_timeout_ms`: how long reading one file's definitions may take.
    pub(super) parse_timeout: Duration,
    /// `index.max_file_bytes`: the most bytes of a file read.
    pub(super) max_file_bytes: u64,
    /// The contracts kept, and the edited files' definitions as read lately.
    pub(super) contracts: &'r Contracts,
}

impl Rendering<'_> {
    /// What a file's definitions are read with.
    fn reading(&self) -> (&Arc<Grammars>, Duration) {
        (self.grammars, self.parse_timeout)
    }
}

/// Where a queued notice goes and what it is about, for the contract kept beside it.
pub(super) struct Queued<'q> {
    /// The notice the contract belongs to.
    pub(super) key: NoticeKey,
    /// How the concerned session holds its file.
    pub(super) holding: Holding,
    /// The editing agent, as the notice names it.
    pub(super) editor: &'q str,
    /// The working tree the edit was made in, and the edited file.
    pub(super) edited: (&'q Path, &'q RepoPath),
    /// The held file's row and path.
    pub(super) held: (FileId, &'q RepoPath),
}

/// The edited file of an edit's answer: its row, the file as read on disk if the index parses it,
/// and the working tree it is in.
pub(in crate::retrieval) type Edited<'e> = ((&'e FileRow, Option<&'e OnDisk<'e>>), &'e Path);

impl Service {
    /// What this service works contracts out with.
    pub(super) fn rendering(&self) -> Rendering<'_> {
        Rendering {
            config: &self.coordination,
            grammars: &self.edits.grammars,
            parse_timeout: self.edits.parse_timeout,
            max_file_bytes: self.max_file_bytes,
            contracts: &self.contracts,
        }
    }

    /// [`signatures_of`] with this service's settings.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub(in crate::retrieval) fn signed(
        &self,
        queries: Queries<'_>,
        edited: Edited<'_>,
        drifted: &[SymbolId],
        changes: (&Signatures, (i64, i64)),
    ) -> Result<Vec<Signed>, StoreError> {
        signatures_of(queries, edited, drifted, (changes, self.rendering()))
    }
}

/// The definitions `drifted` of the edited file whose signature an edit changed, each with its
/// signature before and after when both can be said: after as `edited` reads on disk, before as
/// the header whose hash the store held before the change, from `signatures`' headers or HEAD's
/// commit. The index's changes in `signatures` within `window` give that hash when the index came
/// before the edit's hook. None while `coordination.contracts` is off.
///
/// # Errors
///
/// The store's errors.
pub(super) fn signatures_of(
    queries: Queries<'_>,
    ((row, disk), root): Edited<'_>,
    drifted: &[SymbolId],
    ((signatures, window), rendering): ((&Signatures, (i64, i64)), Rendering<'_>),
) -> Result<Vec<Signed>, StoreError> {
    if !rendering.config.is_contracts || drifted.is_empty() {
        return Ok(Vec::new());
    }
    let read = disk.and_then(|disk| Some((disk, disk.definitions(rendering.reading())?)));
    let Some((disk, extraction)) = read else {
        return Ok(drifted.iter().map(|symbol| (*symbol, None)).collect());
    };
    let definitions = definitions_of(extraction);
    rendering
        .contracts
        .keep_read(&row.file.path, disk.hash(), Arc::clone(&definitions));
    let stored = queries.symbols_in_file(row.id)?;
    let committed = OnceCell::new();
    let committed = || committed.get_or_init(|| committed_signatures(root, row, rendering));
    Ok(drifted
        .iter()
        .map(|symbol| {
            let changed = stored
                .iter()
                .find(|stored| stored.id == *symbol)
                .and_then(|stored| {
                    let position = position_of(extraction, &stored.qual_name)?;
                    let now = extraction.symbols.get(position)?;
                    let before_hash = if stored.sig_hash == now.sig_hash {
                        signatures.before_of(*symbol, window)?
                    } else {
                        stored.sig_hash
                    };
                    // A signature before is said only from a header whose hash the store held for
                    // the definition before the change: the hash vouches for the text.
                    let before = signatures.headers.said(before_hash).or_else(|| {
                        committed()
                            .iter()
                            .find(|(sig_hash, _)| *sig_hash == before_hash)
                            .map(|(_, signature)| signature.clone())
                    })?;
                    Some(Changed {
                        symbol: *symbol,
                        qual_name: stored.qual_name.clone(),
                        name: stored.name.clone(),
                        before,
                        after: signature_at(extraction, position, disk.bytes())?,
                        after_hash: now.sig_hash,
                    })
                });
            (*symbol, changed)
        })
        .collect())
}

/// The signatures of `row`'s file as HEAD's commit in the working tree at `root` holds it, each
/// with its header's hash; none in a folder git does not hold, for a file the commit does not
/// hold, or one that cannot be read. Only git's object store is read.
fn committed_signatures(
    root: &Path,
    row: &FileRow,
    rendering: Rendering<'_>,
) -> Vec<(ShortHash, Signature)> {
    let Some(language) = row.file.language else {
        return Vec::new();
    };
    let committed = GixHistory::open(root)
        .and_then(|history| history.committed(&row.file.path, rendering.max_file_bytes));
    let bytes = match committed {
        Ok(Some(bytes)) => bytes,
        Ok(None) | Err(cortex_git::GitError::NoRepository(_)) => return Vec::new(),
        Err(error) => {
            tracing::warn!(error = %error, "a committed file could not be read for its signatures");
            return Vec::new();
        }
    };
    let (grammars, timeout) = rendering.reading();
    let mut extractor = cortex_index::Extractor::new(Arc::clone(grammars), timeout);
    match extractor.extract(&row.file.path, language, &bytes) {
        Ok(extraction) => signatures_in(&extraction, &bytes),
        Err(_) => Vec::new(),
    }
}

/// `waiting`, the notices waiting for `session`, oldest first, the first whose contract is kept,
/// and not among `given`, said in its words when the files on disk bear it out and it fits
/// `coordination.max_tokens`, the held file read in the working tree at `held_root`; every other
/// notice as the store keeps it, naming the definitions alone. While `coordination.contracts` is
/// off, `waiting` as it is. A store that cannot be read leaves the notice as it is, and the error
/// is logged.
pub(super) fn rendered(
    queries: Queries<'_>,
    (session, held_root): (&Identifier, &Path),
    (waiting, given): (Vec<NoticeRow>, &[NoticeId]),
    rendering: Rendering<'_>,
) -> Vec<NoticeRow> {
    if !rendering.config.is_contracts {
        return waiting;
    }
    let max_tokens = usize::try_from(rendering.config.max_tokens).unwrap_or(usize::MAX);
    let mut is_checked = false;
    let mut rendered = Vec::with_capacity(waiting.len());
    for mut notice in waiting {
        let contract = notice
            .about
            .clone()
            .filter(|_| !is_checked && !given.contains(&notice.id))
            .and_then(|about| {
                let key = (session.clone(), about, notice.created_ms);
                rendering.contracts.of(&key)
            });
        if let Some(contract) = contract {
            // INVARIANT: budgeted (one contract is checked against the files at each hook; the
            // notices after it name their definitions alone)
            is_checked = true;
            match said(queries, &contract, held_root, rendering) {
                // INVARIANT: true-at-render (a contract the files on disk no longer bear out is not
                // said; its notice names the definitions alone)
                Ok(Some(said)) if estimate_tokens(&format!("{said}\n")) <= max_tokens => {
                    notice.text = said;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "a notice's contract failed");
                }
            }
        }
        rendered.push(notice);
    }
    rendered
}

/// The words of `contract` as the files on disk bear it out, the held file read in the working
/// tree at `held_root`: none when they do not ([`borne_out`]), or the held file calls one of the
/// definitions no more.
///
/// # Errors
///
/// The store's errors.
fn said(
    queries: Queries<'_>,
    contract: &Contract,
    held_root: &Path,
    rendering: Rendering<'_>,
) -> Result<Option<String>, StoreError> {
    let Some(held) = borne_out(queries, contract, held_root, rendering)? else {
        return Ok(None);
    };
    let symbols: Vec<SymbolId> = contract
        .changed
        .iter()
        .map(|changed| changed.symbol)
        .collect();
    let lines = calls_in(queries, held, &symbols)?;
    let called: Vec<Called<'_>> = contract
        .changed
        .iter()
        .zip(&lines)
        .map(|(changed, lines)| Called {
            name: changed.name.as_str(),
            before: &changed.before,
            after: &changed.after,
            lines,
        })
        .collect();
    let editor = Party {
        agent: &contract.editor,
        path: contract.edited.as_str(),
    };
    Ok(contract_to_holder(
        contract.holding,
        editor,
        contract.held.as_str(),
        &called,
    ))
}

/// The held file's row when the files on disk bear `contract` out, the held file read in the
/// working tree at `held_root`: the edited file still gives each signature after as it was kept,
/// and the held file's bytes are the ones the index read, so that its stored lines are its lines.
/// None otherwise.
///
/// # Errors
///
/// The store's errors.
fn borne_out(
    queries: Queries<'_>,
    contract: &Contract,
    held_root: &Path,
    rendering: Rendering<'_>,
) -> Result<Option<FileId>, StoreError> {
    let Some(edited) = queries.file(&contract.edited)? else {
        return Ok(None);
    };
    let Some(definitions) = definitions_now(&contract.edited_root, &edited, rendering) else {
        return Ok(None);
    };
    // INVARIANT: true-at-render (a signature after is said only while the edited file on disk
    // still gives it)
    let is_as_kept = contract.changed.iter().all(|changed| {
        definitions.iter().any(|(qual_name, sig_hash)| {
            *qual_name == changed.qual_name && *sig_hash == changed.after_hash
        })
    });
    let Some(held) = queries.file(&contract.held)? else {
        return Ok(None);
    };
    let Some(stored) = held.file.content_hash else {
        return Ok(None);
    };
    // INVARIANT: true-at-render (a calling line is told only from a held file whose bytes are the
    // ones the index read)
    let is_held_as_read =
        CheckedFile::read(held_root, &contract.held, &stored, rendering.max_file_bytes).is_ok();
    Ok((is_as_kept && is_held_as_read).then_some(held.id))
}

/// What `edited`'s file, in the working tree at `root`, defines as it is on disk: read and hashed,
/// and parsed only when its bytes are not those it was last read from.
fn definitions_now(root: &Path, edited: &FileRow, rendering: Rendering<'_>) -> Option<Definitions> {
    let disk = OnDisk::read(root, &edited.file, rendering.max_file_bytes)?;
    if let Some(definitions) = rendering.contracts.read_of(&edited.file.path, disk.hash()) {
        return Some(definitions);
    }
    let definitions = definitions_of(disk.definitions(rendering.reading())?);
    rendering
        .contracts
        .keep_read(&edited.file.path, disk.hash(), Arc::clone(&definitions));
    Some(definitions)
}

/// The contract of the notice `queued` names, of the definitions in `signed` that its held file
/// calls: none when it calls none of them, or when one it calls has a signature that cannot be
/// said, and the notice names the definitions alone.
///
/// # Errors
///
/// The store's errors.
pub(super) fn contract_of(
    queries: Queries<'_>,
    queued: Queued<'_>,
    signed: &[Signed],
) -> Result<Option<Contract>, StoreError> {
    let symbols: Vec<SymbolId> = signed.iter().map(|(symbol, _)| *symbol).collect();
    let lines = calls_in(queries, queued.held.0, &symbols)?;
    let mut changed = Vec::new();
    for ((_, signatures), lines) in signed.iter().zip(&lines) {
        if lines.is_empty() {
            continue;
        }
        let Some(signatures) = signatures else {
            return Ok(None);
        };
        changed.push(signatures.clone());
    }
    if changed.is_empty() {
        return Ok(None);
    }
    let (edited_root, edited) = queued.edited;
    Ok(Some(Contract {
        key: queued.key,
        holding: queued.holding,
        editor: queued.editor.to_owned(),
        edited_root: edited_root.to_path_buf(),
        edited: edited.clone(),
        held: queued.held.1.clone(),
        changed,
    }))
}

/// What `extraction` defines: each qualified name one definition alone has, with its header's
/// hash, in the order of the definitions.
fn definitions_of(extraction: &Extraction) -> Definitions {
    let names: Vec<(Identifier, ShortHash)> = extraction
        .symbols
        .iter()
        .filter(|symbol| position_of(extraction, &symbol.qual_name).is_some())
        .map(|symbol| (symbol.qual_name.clone(), symbol.sig_hash))
        .collect();
    Arc::new(names)
}

/// Where in `extraction` the one definition named `qual_name` is; none when there is none, or more
/// than one, which could not be told apart.
fn position_of(extraction: &Extraction, qual_name: &Identifier) -> Option<usize> {
    let mut found = extraction
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.qual_name == *qual_name)
        .map(|(position, _)| position);
    let position = found.next()?;
    found.next().is_none().then_some(position)
}

/// For each of `symbols`, the lines of `file` that call it, by references tied firmly enough to be
/// stated, in order and each once; the file's references are read once.
///
/// # Errors
///
/// The store's errors.
fn calls_in(
    queries: Queries<'_>,
    file: FileId,
    symbols: &[SymbolId],
) -> Result<Vec<Vec<u32>>, StoreError> {
    let references = queries.refs_in_file(file)?;
    Ok(symbols
        .iter()
        .map(|symbol| {
            let mut lines: Vec<u32> = references
                .iter()
                .filter(|reference| {
                    reference
                        .target
                        .as_ref()
                        .is_some_and(|target| target.symbol == *symbol)
                        && reference.kind == RefKind::Call
                        && reference.confidence >= STRUCTURAL_FLOOR
                })
                .map(|reference| reference.line)
                .collect();
            lines.sort_unstable();
            lines.dedup();
            lines
        })
        .collect())
}

#[cfg(test)]
mod tests;
