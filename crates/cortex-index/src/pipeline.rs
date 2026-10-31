//! Indexing a whole repository: the walk, then every file read, hashed and parsed, then written to
//! the store in path order.
//!
//! Files are prepared in batches. While one batch is written in one transaction, the next is read
//! and parsed on several threads, each with a parser of its own. A batch is written in path order
//! however its threads finish, so the same tree gets the same row ids whatever the number of
//! threads, as long as no file runs out of time.
//!
//! A file that may hold secrets, one over the size limit and one that is not text are written as
//! the walk found them, without being read. Every other file is read whole, hashed and its lines
//! counted, and parsed when Cortex parses its language and neither a tool nor a third party wrote
//! it. A file whose reading or parsing fails is skipped: it is reported, and what the store held
//! for it stays. A stored file the walk no longer finds is removed, unless the walk could not see
//! where it was.
//!
//! A parsed file's imports are resolved as it is prepared, against every file the walk found or
//! the store keeps. Once every file is written, the names its imports bind are tied to their
//! definitions and its import edges are written. A skipped file keeps the edges and the ties
//! through imports it had.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use cortex_core::{ContentHash, FileFlags, Language, ModeSetting, RepoPath, ScaleMode};
use cortex_store::{
    ChangeKind, FileIndex, FileRecord, FileRow, StoreError, SymbolChange, SymbolId, Writer,
};
use thiserror::Error;

use crate::bindings::Bindings;
use crate::discover::{
    DiscoverError, DiscoverOptions, DiscoveredFile, Discovery, Problem, ProblemKind, discover,
};
use crate::extract::{ExtractError, Extractor, LeftOut};
use crate::file_edges;
use crate::file_imports::{self, FileImports};
use crate::folders;
use crate::grammar::Grammars;
use crate::imports::Project;
use crate::incremental;
use crate::resolve;
use crate::safe_read;
use crate::scale::{self, RepoSize};
use crate::scip::{self, ScipOutcome};
use crate::symbol_edges;
use crate::test_targets;

/// How many files are prepared together and written in one transaction. The results of the batch
/// being written and of the one being prepared are all the parsed results held in memory at once,
/// and a commit costs little next to the work in a batch of this size.
const FILES_PER_BATCH: usize = 256;

/// Flags that keep a file from being read: it may hold secrets, it is over the size limit, or it
/// is not text.
const UNREAD: [FileFlags; 3] = [
    FileFlags::SECRET_PATTERN,
    FileFlags::TOO_LARGE,
    FileFlags::BINARY,
];

/// Flags that keep a file from being parsed: a tool or a third party wrote it.
const UNPARSED: [FileFlags; 2] = [FileFlags::GENERATED, FileFlags::VENDORED];

/// What [`index_repository`] needs besides the root, the grammars and the writer.
#[derive(Debug, Clone, Copy)]
pub struct IndexOptions<'a> {
    /// How the repository is walked. Its size limit also bounds what is read.
    pub discover: DiscoverOptions<'a>,
    /// How long parsing one file and reading its definitions may take
    /// (`index.parse_timeout_ms`).
    pub parse_timeout: Duration,
    /// How many files are read and parsed at once.
    pub threads: NonZeroUsize,
    /// The time in UTC milliseconds, recorded as each written file's indexing time.
    pub now_ms: i64,
    /// The scale mode, chosen from the repository's size or pinned (`mode`).
    pub mode: ModeSetting,
}

/// What an index wrote, and what it could not do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexReport {
    /// How many files were written.
    pub files: u64,
    /// How many of them were parsed for their definitions and references.
    pub parsed: u64,
    /// How many files the walk found as stored, kept without being written again; one whose
    /// contents hashed as stored had only its time written.
    pub unchanged: u64,
    /// How many stored files were removed because the walk no longer finds them.
    pub removed: u64,
    /// What the walk could not do.
    pub walk_problems: Vec<Problem>,
    /// Files the walk found that were skipped, in path order, with the reason.
    pub problems: Vec<(RepoPath, FileProblem)>,
    /// Files written although their queries found names that were left out, in path order, with
    /// how many of each kind.
    pub left_out: Vec<(RepoPath, LeftOut)>,
    /// The definitions of written files whose signature or body changed, in the order written.
    pub changed_symbols: Vec<SymbolChange>,
    /// What became of the repository's SCIP index.
    pub scip: ScipOutcome,
    /// The repository's size, as the scale modes count it.
    pub size: RepoSize,
    /// The scale mode recorded for it.
    pub mode: ScaleMode,
}

/// Why a file the walk found was skipped. A skipped file is not written: what the store held for
/// it stays until an index reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileProblem {
    /// It could not be read as the walk found it: it is gone, locked, no longer a regular file,
    /// or now over the size limit.
    Unreadable,
    /// Parsing it and reading its definitions took longer than allowed.
    TimedOut,
    /// Reading it stopped with a panic: a fault in Cortex or in a grammar, caught so that it
    /// costs this file only.
    Panicked,
}

/// Why an index stopped. The batches committed before it stay written.
#[derive(Debug, Error)]
pub enum IndexError {
    /// The walk could not run.
    #[error(transparent)]
    Discover(#[from] DiscoverError),
    /// The store refused a write.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A language's grammar could not be used.
    #[error(transparent)]
    Grammar(ExtractError),
    /// A thread reading files stopped with a panic.
    #[error("a thread reading files stopped unexpectedly")]
    WorkerPanicked,
}

/// What became of one file prepared for the index.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// It is ready to write.
    Ready(Box<Prepared>),
    /// Its contents are as stored: only its new modification time, in nanoseconds, needs writing.
    Touched(RepoPath, i64),
    /// Nothing to write: the file at this path was skipped, for this reason.
    Skipped(RepoPath, FileProblem),
}

impl Outcome {
    /// A file ready to write without being parsed: it defines, uses and imports nothing.
    fn unparsed(file: FileRecord) -> Self {
        Self::Ready(Box::new(Prepared {
            index: file_only(file),
            left_out: LeftOut::default(),
            imported: FileImports::default(),
        }))
    }
}

/// A file ready to write.
#[derive(Debug, PartialEq, Eq)]
struct Prepared {
    /// Its rows: with its definitions and references when it was parsed.
    index: FileIndex,
    /// What its queries found but left out.
    left_out: LeftOut,
    /// What its imports name: files, and the names they bind.
    imported: FileImports,
}

/// What is left to do with a file once it has been read as far as its flags allow.
#[derive(Debug, PartialEq, Eq)]
enum Read {
    /// Nothing: this is what became of it.
    Done(Outcome),
    /// Parsing `source`, its contents, in `language`.
    Parse {
        /// The file, with its size, lines and hash.
        record: FileRecord,
        /// Its language.
        language: Language,
        /// Its contents.
        source: Vec<u8>,
    },
}

/// What every thread that prepares files needs.
struct Context<'a> {
    /// The repository root.
    root: &'a Path,
    /// Every language's compiled rules.
    grammars: &'a Arc<Grammars>,
    /// What import resolution knows of the repository.
    project: &'a Project,
    /// The stored files, by path, whose contents a file read is compared with: one that hashes as
    /// stored is not parsed. `None` when every file read is parsed.
    known: Option<&'a BTreeMap<RepoPath, FileRow>>,
    /// The size above which a file is not read.
    max_file_bytes: u64,
    /// How long one file may take to parse and read.
    parse_timeout: Duration,
    /// How many threads prepare a batch.
    threads: NonZeroUsize,
    /// The indexing time in UTC milliseconds.
    now_ms: i64,
}

/// Indexes the repository at `root` into `writer`'s store: walks it, removes the stored files the
/// walk no longer finds, reads, parses and writes every file that changed since the last index
/// (every file the first time, or after a Cortex of another version), reads the files around the
/// changes again for what their imports bind, ties each reference to the definition its file's
/// imports or its name settle, joins files and definitions in the graph, gives each folder holding
/// stored files its node and containment edges, and records the scale mode.
///
/// # Errors
///
/// [`IndexError::Discover`] when the walk cannot start, [`IndexError::Store`] when a write fails,
/// [`IndexError::Grammar`] when a language's grammar cannot be used, and
/// [`IndexError::WorkerPanicked`]. Problems with single files and folders are reported in the
/// [`IndexReport`] instead.
pub fn index_repository(
    root: &Path,
    grammars: &Arc<Grammars>,
    writer: &mut Writer,
    options: &IndexOptions<'_>,
) -> Result<IndexReport, IndexError> {
    let discovery = discover(root, &options.discover)?;
    let known = incremental::known_files(writer.queries())?;
    let is_whole = known.is_empty();
    let missing = missing_files(writer, &discovery)?;
    let importers_of_gone = if is_whole {
        BTreeSet::new()
    } else {
        incremental::importers(writer.queries(), &missing.iter().cloned().collect())?
    };
    remove_files(writer, &missing)?;
    let project = load_project(root, writer, &discovery, options.discover.max_file_bytes)?;
    let context = Context {
        root,
        grammars,
        project: &project,
        known: Some(&known),
        max_file_bytes: options.discover.max_file_bytes,
        parse_timeout: options.parse_timeout,
        threads: options.threads,
        now_ms: options.now_ms,
    };
    let candidates: Vec<DiscoveredFile> = discovery
        .files
        .iter()
        .filter(|file| {
            known
                .get(&file.path)
                .is_none_or(|row| !incremental::is_unchanged(file, row))
        })
        .cloned()
        .collect();
    let mut report = IndexReport {
        files: 0,
        parsed: 0,
        unchanged: count(discovery.files.len() - candidates.len()),
        removed: count(missing.len()),
        walk_problems: discovery.problems.clone(),
        problems: Vec::new(),
        left_out: Vec::new(),
        changed_symbols: Vec::new(),
        scip: ScipOutcome::Absent,
        size: RepoSize {
            source_files: 0,
            lines: 0,
        },
        mode: ScaleMode::Tiny,
    };
    let written = write_files(writer, &candidates, &context, &mut report)?;
    let around = if is_whole {
        Vec::new()
    } else {
        files_around(writer, &discovery, &written, importers_of_gone)?
    };
    let again = Context {
        known: None,
        ..context
    };
    let read_again = read_again(&around, &again)?;
    link(writer, root, (written, read_again), is_whole, &mut report)?;
    folders::write_folders(writer)?;
    (report.mode, report.size) = scale::settle_mode(writer, options.mode)?;
    let mut transaction = writer.transaction()?;
    transaction.set_cortex_version()?;
    transaction.commit()?;
    Ok(report)
}

/// `items` as a count.
fn count(items: usize) -> u64 {
    u64::try_from(items).unwrap_or(u64::MAX)
}

/// A file this index wrote: what its imports bind, and the ids of its definitions.
struct Written {
    /// Where it is.
    path: RepoPath,
    /// What its imports name and bind.
    imported: FileImports,
    /// Its definitions' rows.
    symbols: Vec<SymbolId>,
}

/// The files around this index's changes to read again for what their imports bind: those whose
/// imports name a `written` file, `importers_of_gone`, whose imports named a removed one, and the
/// files the written files and these reach through their imports; only files the walk found and
/// this index did not write.
fn files_around(
    writer: &Writer,
    discovery: &Discovery,
    written: &[Written],
    importers_of_gone: BTreeSet<RepoPath>,
) -> Result<Vec<DiscoveredFile>, StoreError> {
    let changed: BTreeSet<RepoPath> = written.iter().map(|file| file.path.clone()).collect();
    let mut around = importers_of_gone;
    around.extend(incremental::importers(writer.queries(), &changed)?);
    let from: BTreeSet<RepoPath> = changed.iter().chain(&around).cloned().collect();
    around.extend(incremental::reached(writer.queries(), &from)?);
    Ok(discovery
        .files
        .iter()
        .filter(|file| around.contains(&file.path) && !changed.contains(&file.path))
        .cloned()
        .collect())
}

/// Reads and parses `files` again for what their imports bind, without writing them; a file that
/// cannot be read or parsed now is left out.
fn read_again(
    files: &[DiscoveredFile],
    context: &Context<'_>,
) -> Result<Vec<(RepoPath, FileImports)>, IndexError> {
    let mut imports = Vec::new();
    for batch in files.chunks(FILES_PER_BATCH) {
        for outcome in prepare_batch(batch, context)? {
            if let Outcome::Ready(prepared) = outcome {
                imports.push((prepared.index.file.path, prepared.imported));
            }
        }
    }
    Ok(imports)
}

/// Ties every reference and writes the graph's edges once files are written: `files` holds the
/// written files and those read again for what their imports bind. With `is_whole`, every file
/// was written and every definition's edges are written again; otherwise only those of the
/// written files' definitions and of definitions whose references' targets changed.
fn link(
    writer: &mut Writer,
    root: &Path,
    (written, read_again): (Vec<Written>, Vec<(RepoPath, FileImports)>),
    is_whole: bool,
    report: &mut IndexReport,
) -> Result<(), IndexError> {
    let (scip, scip_ties) = scip::read_index(root, writer.queries())?;
    report.scip = scip;
    let mut affected: BTreeSet<SymbolId> = BTreeSet::new();
    let mut files = Vec::new();
    for file in written {
        affected.extend(file.symbols);
        files.push((file.path, file.imported));
    }
    files.extend(read_again);
    let mut counts = Vec::new();
    let mut bindings = BTreeMap::new();
    let mut test_targets = BTreeMap::new();
    for (path, imported) in files {
        if let Some(target) = imported.test_target {
            test_targets.insert(path.clone(), target);
        }
        counts.push((path.clone(), imported.counts));
        bindings.insert(path, imported.bindings);
    }
    let pass = resolve::resolve_names(writer, &bindings, &scip_ties)?;
    affected.extend(pass.changed_sources);
    let only = (!is_whole).then_some(&affected);
    symbol_edges::write_symbol_edges(writer, &pass.ties, only)?;
    file_edges::write_import_edges(writer, &counts, &test_targets)?;
    Ok(())
}

/// What import resolution knows of the repository at `root`: every file the walk found or the
/// store keeps. A project file is read only when the walk found it and its flags let it be read:
/// one that may hold secrets is never opened.
fn load_project(
    root: &Path,
    writer: &Writer,
    discovery: &Discovery,
    max_file_bytes: u64,
) -> Result<Project, StoreError> {
    let stored = writer.queries().file_paths()?;
    let found = discovery.files.iter().map(|file| file.path.clone());
    let read = |path: &RepoPath| {
        let position = discovery
            .files
            .binary_search_by(|file| file.path.cmp(path))
            .ok()?;
        let file = discovery.files.get(position)?;
        if has_any(file.flags, &UNREAD) {
            return None;
        }
        let bytes = safe_read::read_whole(&path.to_native(root), max_file_bytes).ok()?;
        String::from_utf8(bytes).ok()
    };
    Ok(Project::load(stored.into_iter().chain(found), &read))
}

/// Every stored file the walk did not find, except those it could not see: below a folder it
/// could not list or did not enter, or a file it could not inspect.
fn missing_files(writer: &Writer, discovery: &Discovery) -> Result<Vec<RepoPath>, StoreError> {
    let unseen: BTreeSet<&RepoPath> = discovery
        .problems
        .iter()
        .filter(|problem| hides_files(problem.kind))
        .filter_map(|problem| problem.path.as_ref())
        .collect();
    Ok(writer
        .queries()
        .file_paths()?
        .into_iter()
        .filter(|path| {
            let found = discovery
                .files
                .binary_search_by(|file| file.path.cmp(path))
                .is_ok();
            !found && !is_unseen(path, &unseen)
        })
        .collect())
}

/// Removes the stored files at `paths`.
fn remove_files(writer: &mut Writer, paths: &[RepoPath]) -> Result<(), StoreError> {
    for batch in paths.chunks(FILES_PER_BATCH) {
        let mut transaction = writer.transaction()?;
        for path in batch {
            transaction.remove_file(path)?;
        }
        transaction.commit()?;
    }
    Ok(())
}

/// Whether a walk problem of this kind means the walk did not see the files at its path or below
/// it.
fn hides_files(kind: ProblemKind) -> bool {
    matches!(
        kind,
        ProblemKind::Unreadable | ProblemKind::UnlistedEntry | ProblemKind::TooDeep
    )
}

/// Whether `path`, or a folder above it, is in `unseen`.
fn is_unseen(path: &RepoPath, unseen: &BTreeSet<&RepoPath>) -> bool {
    std::iter::successors(Some(path.clone()), RepoPath::parent).any(|at| unseen.contains(&at))
}

/// Prepares the files in batches and writes each batch in one transaction while the next one is
/// prepared. Returns each written file, in path order.
fn write_files(
    writer: &mut Writer,
    files: &[DiscoveredFile],
    context: &Context<'_>,
    report: &mut IndexReport,
) -> Result<Vec<Written>, IndexError> {
    thread::scope(|scope| {
        let mut written = Vec::new();
        let mut batches = files.chunks(FILES_PER_BATCH);
        let mut next = batches
            .next()
            .map(|batch| scope.spawn(move || prepare_batch(batch, context)));
        while let Some(preparing) = next.take() {
            let outcomes = preparing.join().map_err(|_| IndexError::WorkerPanicked)??;
            next = batches
                .next()
                .map(|batch| scope.spawn(move || prepare_batch(batch, context)));
            written.extend(write_batch(writer, outcomes, report)?);
        }
        Ok(written)
    })
}

/// Writes one batch's files in one transaction, in order, and counts them in `report`, with the
/// definitions whose signature or body changed. A file whose contents are as stored gets only its
/// new time. Returns each written file.
fn write_batch(
    writer: &mut Writer,
    outcomes: Vec<Outcome>,
    report: &mut IndexReport,
) -> Result<Vec<Written>, StoreError> {
    let mut written = Vec::new();
    let mut transaction = writer.transaction()?;
    for outcome in outcomes {
        match outcome {
            Outcome::Ready(prepared) => {
                let change = transaction.replace_file(&prepared.index)?;
                report.files += 1;
                report.parsed += u64::from(prepared.index.file.flags.contains(FileFlags::PARSED));
                let changed = change.changes.into_iter().filter(|symbol| {
                    matches!(symbol.kind, ChangeKind::Signature | ChangeKind::Body)
                });
                report.changed_symbols.extend(changed);
                let path = prepared.index.file.path;
                if !prepared.left_out.is_empty() {
                    report.left_out.push((path.clone(), prepared.left_out));
                }
                written.push(Written {
                    path,
                    imported: prepared.imported,
                    symbols: change.symbols,
                });
            }
            Outcome::Touched(path, mtime_ns) => {
                transaction.set_file_mtime(&path, mtime_ns)?;
                report.unchanged += 1;
            }
            Outcome::Skipped(path, problem) => report.problems.push((path, problem)),
        }
    }
    transaction.commit()?;
    Ok(written)
}

/// Prepares `batch` on the context's threads, each with an extractor of its own, and returns what
/// became of each file in the batch's order.
fn prepare_batch(
    batch: &[DiscoveredFile],
    context: &Context<'_>,
) -> Result<Vec<Outcome>, IndexError> {
    let next = AtomicUsize::new(0);
    let workers = context.threads.get().min(batch.len());
    let finished: Vec<_> = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| scope.spawn(|| prepare_in_turn(batch, &next, context, prepare)))
            .collect();
        handles
            .into_iter()
            .map(thread::ScopedJoinHandle::join)
            .collect()
    });
    let mut outcomes = Vec::with_capacity(batch.len());
    for worker in finished {
        let prepared = worker
            .map_err(|_| IndexError::WorkerPanicked)?
            .map_err(IndexError::Grammar)?;
        outcomes.extend(prepared);
    }
    // INVARIANT: deterministic. Threads take files in turn and finish in any order; the batch is
    // written in path order.
    outcomes.sort_unstable_by_key(|(position, _)| *position);
    Ok(outcomes.into_iter().map(|(_, outcome)| outcome).collect())
}

/// How one file is prepared: [`prepare`], or in tests a stand-in that fails on purpose.
type PrepareFile =
    fn(&DiscoveredFile, &Context<'_>, &mut Extractor) -> Result<Outcome, ExtractError>;

/// Prepares files of `batch` one at a time with `prepare_file`, each at the next position `next`
/// hands out, until none is left. Returns each outcome with its position.
fn prepare_in_turn(
    batch: &[DiscoveredFile],
    next: &AtomicUsize,
    context: &Context<'_>,
    prepare_file: PrepareFile,
) -> Result<Vec<(usize, Outcome)>, ExtractError> {
    let new_extractor = || Extractor::new(Arc::clone(context.grammars), context.parse_timeout);
    let mut extractor = new_extractor();
    let mut prepared = Vec::new();
    loop {
        let position = next.fetch_add(1, Ordering::Relaxed);
        let Some(file) = batch.get(position) else {
            return Ok(prepared);
        };
        let outcome = guarded(&file.path, || prepare_file(file, context, &mut extractor))?;
        if matches!(outcome, Outcome::Skipped(_, FileProblem::Panicked)) {
            // The panic may have left the parser or the query cursor part of the way through.
            extractor = new_extractor();
        }
        prepared.push((position, outcome));
    }
}

/// Runs `prepare` for the file at `path`, turning a panic into skipping the file, so that one
/// file cannot stop the index.
fn guarded(
    path: &RepoPath,
    prepare: impl FnOnce() -> Result<Outcome, ExtractError>,
) -> Result<Outcome, ExtractError> {
    // Unwind safe as used: after a panic the caller replaces the extractor `prepare` borrowed.
    panic::catch_unwind(AssertUnwindSafe(prepare))
        .unwrap_or_else(|_| Ok(Outcome::Skipped(path.clone(), FileProblem::Panicked)))
}

/// Reads, hashes and parses one file as its flags allow, or skips it. Only a grammar that cannot
/// be used is an error.
fn prepare(
    file: &DiscoveredFile,
    context: &Context<'_>,
    extractor: &mut Extractor,
) -> Result<Outcome, ExtractError> {
    let (mut record, language, source) =
        match read(file, context.root, context.max_file_bytes, context.now_ms) {
            Read::Done(Outcome::Ready(prepared)) if is_as_stored(&prepared.index.file, context) => {
                let file = prepared.index.file;
                return Ok(Outcome::Touched(file.path, file.mtime_ns));
            }
            Read::Done(outcome) => return Ok(outcome),
            Read::Parse { record, .. } if is_as_stored(&record, context) => {
                return Ok(Outcome::Touched(record.path, record.mtime_ns));
            }
            Read::Parse {
                record,
                language,
                source,
            } => (record, language, source),
        };
    match extractor.extract(&record.path, language, &source) {
        Ok(extraction) => {
            record.flags = record.flags.with(FileFlags::PARSED);
            let (counts, targets) = file_imports::resolve_imports(
                context.project,
                &record.path,
                language,
                &extraction.imports,
            );
            let names = file_imports::module_names(context.project, language, &extraction.imports);
            let spec = context.grammars.get(language).spec;
            let bindings = Bindings::of(&extraction, &targets, &names, spec.closed_scopes);
            let test_target = test_targets::base_name_target(
                &record.path,
                spec.test_files,
                &counts,
                context.project,
            );
            Ok(Outcome::Ready(Box::new(Prepared {
                index: FileIndex {
                    file: record,
                    symbols: extraction.symbols,
                    refs: extraction.refs,
                },
                left_out: extraction.left_out,
                imported: FileImports {
                    counts,
                    bindings,
                    test_target,
                },
            })))
        }
        Err(ExtractError::TimedOut { .. }) => {
            Ok(Outcome::Skipped(record.path, FileProblem::TimedOut))
        }
        Err(error) => Err(error),
    }
}

/// Whether the file `record` describes, just read, holds what the store holds for it: the same
/// contents by hash, language and flags.
fn is_as_stored(record: &FileRecord, context: &Context<'_>) -> bool {
    let Some(row) = context.known.and_then(|known| known.get(&record.path)) else {
        return false;
    };
    let stored = &row.file;
    record.content_hash.is_some()
        && record.content_hash == stored.content_hash
        && record.language == stored.language
        && record.flags == incremental::unparsed(stored.flags)
}

/// Reads `file`, found under `root`, as far as its flags allow, recording `now_ms` as its indexing
/// time: not at all when it may hold secrets, is over the size limit or is not text; otherwise
/// whole, to hash it and count its lines, and then it is left to parse when Cortex parses its
/// language and neither a tool nor a third party wrote it.
fn read(file: &DiscoveredFile, root: &Path, max_file_bytes: u64, now_ms: i64) -> Read {
    let mut record = FileRecord {
        path: file.path.clone(),
        language: file.language,
        size_bytes: file.size_bytes,
        line_count: None,
        content_hash: None,
        mtime_ns: file.mtime_ns,
        flags: file.flags,
        indexed_at_ms: now_ms,
    };
    if has_any(file.flags, &UNREAD) {
        return Read::Done(Outcome::unparsed(record));
    }
    let Some(source) = read_contents(&mut record, root, max_file_bytes) else {
        return Read::Done(Outcome::Skipped(record.path, FileProblem::Unreadable));
    };
    match file.language.filter(|_| !has_any(file.flags, &UNPARSED)) {
        Some(language) => Read::Parse {
            record,
            language,
            source,
        },
        None => Read::Done(Outcome::unparsed(record)),
    }
}

/// Reads the whole file `record` describes, and records its size, line count and hash; `None`
/// when it cannot be read as the walk found it.
fn read_contents(record: &mut FileRecord, root: &Path, max_file_bytes: u64) -> Option<Vec<u8>> {
    let source = safe_read::read_whole(&record.path.to_native(root), max_file_bytes).ok()?;
    record.size_bytes = u64::try_from(source.len()).unwrap_or(u64::MAX);
    record.line_count = Some(count_lines(&source));
    record.content_hash = Some(ContentHash::of(&source));
    Some(source)
}

/// How many lines `source` has: one per line feed, and one more for text after the last.
#[allow(
    clippy::naive_bytecount,
    reason = "a file read here is at most `index.max_file_bytes` long, so one pass over it costs \
              little next to parsing it, and a counting crate is not worth a dependency"
)]
fn count_lines(source: &[u8]) -> u64 {
    let feeds = source.iter().filter(|byte| **byte == b'\n').count();
    let has_open_line = source.last().is_some_and(|byte| *byte != b'\n');
    u64::try_from(feeds)
        .unwrap_or(u64::MAX)
        .saturating_add(u64::from(has_open_line))
}

/// Whether `flags` holds any of `wanted`.
fn has_any(flags: FileFlags, wanted: &[FileFlags]) -> bool {
    wanted.iter().any(|flag| flags.contains(*flag))
}

/// The rows of a file written without definitions or references.
fn file_only(file: FileRecord) -> FileIndex {
    FileIndex {
        file,
        symbols: Vec::new(),
        refs: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines are counted as an editor shows them: a final line feed ends the last line rather
    /// than starting another, and a carriage return is part of its line.
    #[test]
    fn lines_are_counted_as_editors_show_them() {
        let cases: [(&[u8], u64); 7] = [
            (b"", 0),
            (b"a", 1),
            (b"a\n", 1),
            (b"a\nb", 2),
            (b"\n", 1),
            (b"\n\n", 2),
            (b"a\r\nb\r\n", 2),
        ];
        for (source, expected) in cases {
            assert_eq!(count_lines(source), expected, "{source:?}");
        }
    }

    /// A path the walk could not see is one at or below a path it reported, compared by whole
    /// folder names; a problem at the root hides every path.
    #[test]
    fn unseen_paths_are_at_or_below_a_problem() -> Result<(), cortex_core::PathError> {
        let folder = RepoPath::new("src/a")?;
        let unseen = BTreeSet::from([&folder]);
        let cases = [
            ("src/a", true),
            ("src/a/b.py", true),
            ("src/a/deep/c.py", true),
            ("src/ab.py", false),
            ("src/b.py", false),
            ("a.py", false),
        ];
        for (path, expected) in cases {
            assert_eq!(
                is_unseen(&RepoPath::new(path)?, &unseen),
                expected,
                "{path}"
            );
        }
        let root = RepoPath::root();
        let everything = BTreeSet::from([&root]);
        assert!(is_unseen(&RepoPath::new("a.py")?, &everything));
        Ok(())
    }

    /// A file is read whole and its size, lines and hash recorded; a file gone since the walk, a
    /// folder in its place and a file grown past the limit cannot be read and leave the record
    /// as it was.
    #[test]
    fn contents_are_read_as_the_walk_found_them() -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::tempdir()?;
        std::fs::write(repo.path().join("a.py"), "x = 1\ny = 2\n")?;
        std::fs::write(repo.path().join("big.py"), "x = 1\n".repeat(10))?;
        std::fs::create_dir(repo.path().join("folder.py"))?;
        let record = |path: &str| -> Result<FileRecord, cortex_core::PathError> {
            Ok(FileRecord {
                path: RepoPath::new(path)?,
                language: None,
                size_bytes: 3,
                line_count: None,
                content_hash: None,
                mtime_ns: 7,
                flags: FileFlags::empty(),
                indexed_at_ms: 9,
            })
        };
        let mut whole = record("a.py")?;
        let source = read_contents(&mut whole, repo.path(), 50);
        assert_eq!(source.as_deref(), Some(&b"x = 1\ny = 2\n"[..]));
        assert_eq!(
            (whole.size_bytes, whole.line_count, whole.content_hash),
            (12, Some(2), Some(ContentHash::of(b"x = 1\ny = 2\n")))
        );
        for path in ["gone.py", "folder.py", "big.py"] {
            let mut unread = record(path)?;
            assert_eq!(read_contents(&mut unread, repo.path(), 50), None, "{path}");
            assert_eq!(unread, record(path)?, "{path}");
        }
        Ok(())
    }

    /// A file is read only as far as its flags allow. One that may hold secrets, is over the size
    /// limit or is not text is never opened: the file here does not exist, so opening it would
    /// make it unreadable. Generated, vendored and other text is read but not parsed; source
    /// Cortex parses is left to parse; a file gone since the walk is unreadable.
    #[test]
    fn files_are_read_as_far_as_their_flags_allow() -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::tempdir()?;
        for name in ["a.py", "gen.py", "lib.py", "notes.txt"] {
            std::fs::write(repo.path().join(name), "x = 1\n")?;
        }
        let found =
            |path: &str, language, flags| -> Result<DiscoveredFile, cortex_core::PathError> {
                Ok(DiscoveredFile {
                    path: RepoPath::new(path)?,
                    language,
                    size_bytes: 6,
                    mtime_ns: 7,
                    flags,
                })
            };
        let record = |file: &DiscoveredFile, is_read: bool| FileRecord {
            path: file.path.clone(),
            language: file.language,
            size_bytes: 6,
            line_count: is_read.then_some(1),
            content_hash: is_read.then(|| ContentHash::of(b"x = 1\n")),
            mtime_ns: 7,
            flags: file.flags,
            indexed_at_ms: 9,
        };
        let python = Some(Language::Python);
        let root = repo.path();
        for flags in [
            FileFlags::SECRET_PATTERN,
            FileFlags::TOO_LARGE,
            FileFlags::BINARY,
        ] {
            let file = found("missing.py", python, flags)?;
            let expected = Read::Done(Outcome::unparsed(record(&file, false)));
            assert_eq!(read(&file, root, 50, 9), expected, "{flags:?}");
        }
        for file in [
            found("gen.py", python, FileFlags::GENERATED)?,
            found("lib.py", python, FileFlags::VENDORED)?,
            found("notes.txt", None, FileFlags::empty())?,
        ] {
            let expected = Read::Done(Outcome::unparsed(record(&file, true)));
            assert_eq!(read(&file, root, 50, 9), expected, "{}", file.path.as_str());
        }
        let source = found("a.py", python, FileFlags::empty())?;
        let expected = Read::Parse {
            record: record(&source, true),
            language: Language::Python,
            source: b"x = 1\n".to_vec(),
        };
        assert_eq!(read(&source, root, 50, 9), expected);
        let gone = found("gone.py", python, FileFlags::empty())?;
        let expected = Read::Done(Outcome::Skipped(gone.path.clone(), FileProblem::Unreadable));
        assert_eq!(read(&gone, root, 50, 9), expected);
        Ok(())
    }

    /// A panic while a file is prepared skips that file instead of stopping the thread; what
    /// `prepare` returns otherwise passes through, an error included.
    #[test]
    fn a_panic_skips_the_file() -> Result<(), cortex_core::PathError> {
        let path = RepoPath::new("src/a.py")?;
        #[allow(
            clippy::panic,
            reason = "the panic is what the guard is tested against"
        )]
        let skipped = guarded(&path, || panic!("a fault while reading"));
        assert_eq!(
            skipped,
            Ok(Outcome::Skipped(path.clone(), FileProblem::Panicked))
        );
        let ready = || {
            let file = FileRecord {
                path: path.clone(),
                language: None,
                size_bytes: 1,
                line_count: Some(1),
                content_hash: None,
                mtime_ns: 1,
                flags: FileFlags::empty(),
                indexed_at_ms: 1,
            };
            Outcome::unparsed(file)
        };
        assert_eq!(guarded(&path, || Ok(ready())), Ok(ready()));
        let refused = || ExtractError::Grammar {
            language: Language::Python,
            message: "refused".to_owned(),
        };
        assert_eq!(guarded(&path, || Err(refused())), Err(refused()));
        Ok(())
    }

    /// Prepares the file as usual, except that it panics on `b.py`.
    fn panics_on_b(
        file: &DiscoveredFile,
        context: &Context<'_>,
        extractor: &mut Extractor,
    ) -> Result<Outcome, ExtractError> {
        // The failed assertion is the panic the worker is tested against.
        assert!(file.path.as_str() != "b.py", "a fault while reading b.py");
        prepare(file, context, extractor)
    }

    /// A file whose preparation panics costs that file only: the thread reports it with the
    /// reason and goes on, and parses the file after it as usual.
    #[test]
    fn a_panicking_file_costs_that_file_only() -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::tempdir()?;
        let names = ["a.py", "b.py", "c.py"];
        for name in names {
            std::fs::write(repo.path().join(name), "def f():\n    pass\n")?;
        }
        let grammars = Arc::new(Grammars::load()?);
        let context = Context {
            root: repo.path(),
            grammars: &grammars,
            project: &Project::load(Vec::new(), &|_| None),
            known: None,
            max_file_bytes: 1000,
            parse_timeout: Duration::from_secs(60),
            threads: NonZeroUsize::MIN,
            now_ms: 9,
        };
        let batch = names
            .into_iter()
            .map(|name| {
                Ok(DiscoveredFile {
                    path: RepoPath::new(name)?,
                    language: Some(Language::Python),
                    size_bytes: 18,
                    mtime_ns: 7,
                    flags: FileFlags::empty(),
                })
            })
            .collect::<Result<Vec<_>, cortex_core::PathError>>()?;
        let prepared = prepare_in_turn(&batch, &AtomicUsize::new(0), &context, panics_on_b)?;
        let summary: Vec<(usize, &str, Option<FileProblem>, Vec<&str>)> = prepared
            .iter()
            .map(|(position, outcome)| match outcome {
                Outcome::Ready(prepared) => {
                    let index = &prepared.index;
                    let names = index.symbols.iter().map(|s| s.qual_name.as_str());
                    (*position, index.file.path.as_str(), None, names.collect())
                }
                Outcome::Skipped(path, problem) => {
                    (*position, path.as_str(), Some(*problem), Vec::new())
                }
                Outcome::Touched(path, _) => (*position, path.as_str(), None, Vec::new()),
            })
            .collect();
        assert_eq!(
            summary,
            [
                (0, "a.py", None, vec!["f"]),
                (1, "b.py", Some(FileProblem::Panicked), vec![]),
                (2, "c.py", None, vec!["f"]),
            ]
        );
        Ok(())
    }

    /// A problem with a folder, one of its entries or a file means the walk did not see what is
    /// there; a problem with a rules file or a name that cannot be stored does not.
    #[test]
    fn only_what_the_walk_could_not_see_is_hidden() {
        let cases = [
            (ProblemKind::Unreadable, true),
            (ProblemKind::UnlistedEntry, true),
            (ProblemKind::TooDeep, true),
            (ProblemKind::UnstorableName, false),
            (ProblemKind::SkippedRulesFile, false),
            (ProblemKind::BadRulesLine { line: 1 }, false),
            (ProblemKind::MoreBadRulesLines { count: 1 }, false),
        ];
        for (kind, expected) in cases {
            assert_eq!(hides_files(kind), expected, "{kind:?}");
        }
    }

    /// The report of an index that has done nothing yet.
    fn empty_report() -> IndexReport {
        IndexReport {
            files: 0,
            parsed: 0,
            unchanged: 0,
            removed: 0,
            walk_problems: Vec::new(),
            problems: Vec::new(),
            left_out: Vec::new(),
            changed_symbols: Vec::new(),
            scip: ScipOutcome::Absent,
            size: RepoSize {
                source_files: 0,
                lines: 0,
            },
            mode: ScaleMode::Tiny,
        }
    }

    /// A batch is written in one transaction and counted: a file ready to write is written, with
    /// what its queries left out reported when anything was, and returned with the files its
    /// imports name; a skipped file is reported and leaves what the store held for it, or
    /// nothing; a file whose contents are as stored gets its new time and counts as unchanged.
    #[test]
    fn a_batch_writes_what_was_read_and_reports_the_rest() -> Result<(), Box<dyn std::error::Error>>
    {
        let repo = tempfile::tempdir()?;
        let mut writer = Writer::open(&cortex_store::WriterOptions {
            folder: &repo.path().join(".cortex"),
            repo_id: &cortex_core::RepoId::of(repo.path()),
            mmap_bytes: 0,
            now_ms: 1,
        })?;
        let record = |path: &str, flags| -> Result<FileRecord, cortex_core::PathError> {
            Ok(FileRecord {
                path: RepoPath::new(path)?,
                language: None,
                size_bytes: 1,
                line_count: Some(1),
                content_hash: None,
                mtime_ns: 1,
                flags,
                indexed_at_ms: 1,
            })
        };
        let kept = record("kept.txt", FileFlags::empty())?;
        let slow = record("slow.txt", FileFlags::PARSED)?;
        let mut transaction = writer.transaction()?;
        transaction.replace_file(&file_only(kept.clone()))?;
        transaction.replace_file(&file_only(slow.clone()))?;
        transaction.commit()?;
        let mut report = empty_report();
        let skipped = |path: &str, problem| -> Result<Outcome, cortex_core::PathError> {
            Ok(Outcome::Skipped(RepoPath::new(path)?, problem))
        };
        let some_left_out = LeftOut {
            definitions: 2,
            references: 0,
            imports: 1,
        };
        let some_imported = || -> Result<FileImports, cortex_core::PathError> {
            Ok(FileImports {
                counts: [(RepoPath::new("a.txt")?, 2)].into(),
                bindings: crate::bindings::Bindings::default(),
                test_target: Some(RepoPath::new("a.txt")?),
            })
        };
        let outcomes = vec![
            Outcome::Ready(Box::new(Prepared {
                index: file_only(record("a.txt", FileFlags::PARSED)?),
                left_out: LeftOut::default(),
                imported: FileImports::default(),
            })),
            Outcome::Ready(Box::new(Prepared {
                index: file_only(record("b.txt", FileFlags::PARSED)?),
                left_out: some_left_out,
                imported: some_imported()?,
            })),
            skipped("kept.txt", FileProblem::Unreadable)?,
            skipped("new.txt", FileProblem::Panicked)?,
            skipped("slow.txt", FileProblem::TimedOut)?,
            Outcome::Touched(RepoPath::new("a.txt")?, 99),
        ];
        let written = write_batch(&mut writer, outcomes, &mut report)?;
        let imports: Vec<(RepoPath, FileImports)> = written
            .into_iter()
            .map(|file| (file.path, file.imported))
            .collect();
        assert_eq!(report.unchanged, 1);
        let touched = writer.queries().file(&RepoPath::new("a.txt")?)?;
        assert_eq!(touched.map(|row| row.file.mtime_ns), Some(99));
        assert_eq!(
            imports,
            [
                (RepoPath::new("a.txt")?, FileImports::default()),
                (RepoPath::new("b.txt")?, some_imported()?)
            ]
        );
        assert_eq!((report.files, report.parsed), (2, 2));
        assert_eq!(report.left_out, [(RepoPath::new("b.txt")?, some_left_out)]);
        assert_eq!(
            report.problems,
            [
                (RepoPath::new("kept.txt")?, FileProblem::Unreadable),
                (RepoPath::new("new.txt")?, FileProblem::Panicked),
                (RepoPath::new("slow.txt")?, FileProblem::TimedOut),
            ]
        );
        let queries = writer.queries();
        let paths = ["a.txt", "b.txt", "kept.txt", "slow.txt"].map(RepoPath::new);
        assert_eq!(
            queries.file_paths()?,
            paths.into_iter().collect::<Result<Vec<_>, _>>()?
        );
        for unchanged in [kept, slow] {
            let stored = queries.file(&unchanged.path)?;
            assert_eq!(stored.map(|row| row.file), Some(unchanged));
        }
        Ok(())
    }
}
