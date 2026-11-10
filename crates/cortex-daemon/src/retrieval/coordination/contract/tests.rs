//! Interface contracts worked out and rendered against a repository indexed for the test: Codex
//! changes `add` in `calc/ops.py`, which `main.py` calls on its third line, and a session reading
//! `main.py` is told what changed while the files on disk bear it out, and otherwise which
//! definition changed, by name.

use std::num::NonZeroUsize;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use cortex_coord::{Holding, PositionMode};
use cortex_core::{CoordinationConfig, Identifier, ModeSetting, NoticeSubject, RepoPath};
use cortex_index::{
    DiscoverOptions, Extraction, Grammars, IndexOptions, OnChanged, OnParsed, index_paths,
    index_repository,
};
use cortex_store::{
    ChangeKind, FileRow, NoticeId, NoticeRecord, NoticeRow, SymbolChange, SymbolId, Writer,
};

use super::super::super::on_disk::OnDisk;
use super::{Queued, Rendering, Signed, contract_of, rendered, signatures_of};
use crate::contracts::Contracts;
use crate::signatures::Signatures;
use crate::test_store::{TestResult, writer};

/// `calc/ops.py` before the change.
const OPS: &str = "def add(a, b):\n    return a + b\n";

/// `calc/ops.py` once `add` takes a third number.
const OPS_THREE: &str = "def add(a, b, c=0):\n    return a + b + c\n";

/// `main.py`, calling `add` on its third line.
const MAIN: &str = "from calc.ops import add\n\nprint(add(1, 2))\n";

/// What a session reading `main.py` is told of the change, typed by hand.
const TOLD: &str = "[cortex] Coordination: Codex changed add in calc/ops.py from `(a, b)` to \
                    `(a, b, c=0)`. You read main.py, which calls it at line 3.";

/// What the store keeps of the notice, naming the definition alone.
const NAMES_ONLY: &str = "names only";

/// The session told.
const SESSION: &str = "b";

/// When the index's changes are found, in UTC milliseconds, and the window an edit looks at them
/// in.
const FOUND_MS: i64 = 1;

/// The window of an edit's task, around [`FOUND_MS`].
const WINDOW: (i64, i64) = (0, 10);

/// A repository indexed for the test, what its indexes left in memory, and its store's writer.
struct Repository {
    /// The folder holding it.
    folder: tempfile::TempDir,
    /// Every grammar.
    grammars: Arc<Grammars>,
    /// The signature changes and the signatures the indexes read.
    signatures: Signatures,
    /// The contracts kept and the files read for them.
    contracts: Contracts,
    /// The store's writer.
    writer: Writer,
}

impl Repository {
    /// `calc/ops.py` holding `ops` and `main.py` [`MAIN`], indexed whole; committed first with git
    /// when `is_committed`.
    fn with(ops: &str, is_committed: bool) -> TestResult<Self> {
        let folder = tempfile::tempdir()?;
        std::fs::create_dir_all(folder.path().join("calc"))?;
        std::fs::write(folder.path().join("calc/__init__.py"), "")?;
        std::fs::write(folder.path().join("calc/ops.py"), ops)?;
        std::fs::write(folder.path().join("main.py"), MAIN)?;
        if is_committed {
            commit(folder.path())?;
        }
        let mut repository = Self {
            writer: writer(folder.path())?,
            folder,
            grammars: Arc::new(Grammars::load()?),
            signatures: Signatures::new(),
            contracts: Contracts::default(),
        };
        repository.index(None)?;
        Ok(repository)
    }

    /// The repository's root.
    fn root(&self) -> &Path {
        self.folder.path()
    }

    /// Forgets what the indexes left in memory, as a daemon that started again would.
    fn restart(&mut self) {
        self.signatures = Signatures::new();
        self.contracts = Contracts::default();
    }

    /// Writes `text` to the file at `path`, and indexes it again when `is_indexed`.
    fn write(&mut self, path: &str, text: &str, is_indexed: bool) -> TestResult {
        std::fs::write(self.root().join(path), text)?;
        if is_indexed {
            self.index(Some(&[RepoPath::new(path)?]))?;
        }
        Ok(())
    }

    /// Indexes `paths`, or the whole repository, keeping the signatures read and the changes found
    /// as the daemon keeps them, found at [`FOUND_MS`].
    fn index(&mut self, paths: Option<&[RepoPath]>) -> TestResult {
        let signatures = &self.signatures;
        let read = |_: &RepoPath, extraction: &Extraction, source: &[u8]| {
            signatures.headers.note(extraction, source);
        };
        let keep = |changes: &[SymbolChange]| {
            let changed = changes
                .iter()
                .filter(|change| change.kind == ChangeKind::Signature)
                .filter_map(|change| Some((change.symbol, change.sig_hash_before?)));
            signatures.note(changed, FOUND_MS);
        };
        let options = IndexOptions {
            discover: DiscoverOptions {
                max_file_bytes: 1_048_576,
                extra_ignores: &[],
                global_gitignore: None,
            },
            parse_timeout: Duration::from_secs(60),
            threads: NonZeroUsize::MIN,
            now_ms: 1,
            mode: ModeSetting::Auto,
            pause: None,
            on_changed: Some(OnChanged(&keep)),
            on_parsed: Some(OnParsed(&read)),
        };
        let root = self.folder.path();
        match paths {
            Some(paths) => index_paths(root, &self.grammars, &mut self.writer, &options, paths)?,
            None => index_repository(root, &self.grammars, &mut self.writer, &options)?,
        };
        Ok(())
    }

    /// The stored row of the file at `path`.
    fn file(&self, path: &str) -> TestResult<FileRow> {
        Ok(self
            .writer
            .queries()
            .file(&RepoPath::new(path)?)?
            .ok_or("not stored")?)
    }

    /// The stored symbol `add`.
    fn add(&self) -> TestResult<SymbolId> {
        let ops = self.file("calc/ops.py")?;
        Ok(self
            .writer
            .queries()
            .symbols_in_file(ops.id)?
            .into_iter()
            .find(|symbol| symbol.name.as_str() == "add")
            .ok_or("no add")?
            .id)
    }

    /// What contracts are worked out with, under `config`.
    fn rendering<'r>(&'r self, config: &'r CoordinationConfig) -> Rendering<'r> {
        Rendering {
            config,
            grammars: &self.grammars,
            parse_timeout: Duration::from_secs(60),
            max_file_bytes: 1_048_576,
            contracts: &self.contracts,
        }
    }

    /// `add` with its signatures before and after its edit, as the edit's hook reads them.
    fn signed(&self, config: &CoordinationConfig) -> TestResult<Vec<Signed>> {
        let ops = self.file("calc/ops.py")?;
        let disk = OnDisk::read(self.root(), &ops.file, 1_048_576);
        let edited = ((&ops, disk.as_ref()), self.root());
        let add = [self.add()?];
        let changes = ((&self.signatures, WINDOW), self.rendering(config));
        Ok(signatures_of(self.writer.queries(), edited, &add, changes)?)
    }

    /// Queues at `at_ms` the names-only notice for the session reading `main.py` and keeps the
    /// contract of `signed` for it, if there is one; whether there is.
    fn queue(&mut self, signed: &[Signed], at_ms: i64) -> TestResult<bool> {
        let main = self.file("main.py")?;
        let (edited, held) = (RepoPath::new("calc/ops.py")?, RepoPath::new("main.py")?);
        let queued = Queued {
            key: (Identifier::new(SESSION)?, subject(at_ms), at_ms),
            holding: Holding {
                mode: PositionMode::Read,
                is_urgent: false,
            },
            editor: "Codex",
            edited: (self.root(), &edited),
            held: (main.id, &held),
        };
        let contract = contract_of(self.writer.queries(), queued, signed)?;
        let is_kept = contract.is_some();
        contract
            .into_iter()
            .for_each(|contract| self.contracts.keep(contract));
        let mut transaction = self.writer.transaction()?;
        let notice = NoticeRecord {
            to_session: Identifier::new(SESSION)?,
            about: subject(at_ms),
            text: NAMES_ONLY.to_owned(),
            at_ms,
        };
        transaction.queue_notice(&notice, 0)?;
        transaction.commit()?;
        Ok(is_kept)
    }

    /// The ids of the notices waiting for the session reading `main.py`, oldest first.
    fn waiting(&self) -> TestResult<Vec<NoticeRow>> {
        Ok(self
            .writer
            .queries()
            .notices_waiting(&Identifier::new(SESSION)?, 0)?)
    }

    /// The texts the session reading `main.py` is given of its waiting notices now, `given` having
    /// been given already.
    fn given(&self, config: &CoordinationConfig, given: &[NoticeId]) -> TestResult<Vec<String>> {
        let session = Identifier::new(SESSION)?;
        let held_by = (&session, self.root());
        let waiting = (self.waiting()?, given);
        let queries = self.writer.queries();
        let given = rendered(queries, held_by, waiting, self.rendering(config));
        Ok(given.into_iter().map(|notice| notice.text).collect())
    }
}

/// Makes the folder `root` a git repository whose one commit holds every file in it, with git's
/// settings read from nowhere else.
fn commit(root: &Path) -> TestResult {
    let git = |args: &[&str]| -> TestResult {
        let status = Command::new("git")
            .args(["-c", "user.name=Cortex Test", "-c", "user.email=test"])
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(args)
            .current_dir(root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join(".no-config"))
            .status()?;
        if !status.success() {
            return Err(format!("git {args:?} failed").into());
        }
        Ok(())
    };
    git(&["init", "-q"])?;
    git(&["add", "--", "calc", "main.py"])?;
    git(&["commit", "-q", "-m", "first"])
}

/// What a notice queued at `at_ms` is about.
fn subject(at_ms: i64) -> NoticeSubject {
    NoticeSubject::from_stored(format!("Codex\ncalc/ops.py\nedit\napart\nadd\n{at_ms}"))
}

/// `coordination.contracts` on.
fn on() -> CoordinationConfig {
    CoordinationConfig {
        is_contracts: true,
        ..CoordinationConfig::default()
    }
}

/// The signatures of `signed`, as said.
fn said(signed: &[Signed]) -> Vec<Option<(String, String)>> {
    signed
        .iter()
        .map(|(_, changed)| {
            changed.as_ref().map(|changed| {
                let before = changed.before.as_str().to_owned();
                (before, changed.after.as_str().to_owned())
            })
        })
        .collect()
}

/// `(a, b)` before and `(a, b, c=0)` after, as said.
fn both() -> Option<(String, String)> {
    Some(("(a, b)".to_owned(), "(a, b, c=0)".to_owned()))
}

/// Codex's edit of `add`'s header comes before the index of the file: the signature before is the
/// one the index read, after the one on disk. The session reading `main.py` is told both and the
/// calling line, before and after the index of `calc/ops.py`. Once `main.py` changed on disk and
/// was not indexed again, its line is no longer the index's: the notice names `add` alone;
/// indexed, it is told with the call's new line. Once `add` changed again on disk, the signature
/// after is no longer the one kept, and the notice names `add` alone.
#[test]
fn a_contract_is_told_while_the_files_bear_it_out() -> TestResult {
    let config = on();
    let mut repository = Repository::with(OPS, false)?;
    repository.write("calc/ops.py", OPS_THREE, false)?;
    let signed = repository.signed(&config)?;
    assert_eq!(said(&signed), [both()]);
    assert!(repository.queue(&signed, 1_000)?);
    assert_eq!(repository.given(&config, &[])?, [TOLD]);

    repository.write("calc/ops.py", OPS_THREE, true)?;
    assert_eq!(said(&repository.signed(&config)?), [both()]);
    assert_eq!(repository.given(&config, &[])?, [TOLD]);

    let moved = "from calc.ops import add\n\nprint(0)\nprint(add(1, 2))\n";
    repository.write("main.py", moved, false)?;
    assert_eq!(repository.given(&config, &[])?, [NAMES_ONLY]);
    repository.write("main.py", moved, true)?;
    assert_eq!(
        repository.given(&config, &[])?,
        [TOLD.replace("line 3", "line 4")]
    );

    repository.write("calc/ops.py", "def add(a):\n    return a\n", false)?;
    assert_eq!(repository.given(&config, &[])?, [NAMES_ONLY]);
    Ok(())
}

/// A daemon that started again has read no header of `calc/ops.py`: the signature before is read
/// from HEAD's commit, whose header hashes as the store holds `add`'s, whether the edit's hook
/// comes before the index of the file or after it. With nothing committed, or a commit holding
/// another header than the store's, there is no signature before, and the notice names `add`.
#[test]
fn after_a_restart_the_signature_before_is_read_from_head() -> TestResult {
    let config = on();
    let mut repository = Repository::with(OPS, true)?;
    repository.restart();
    repository.write("calc/ops.py", OPS_THREE, false)?;
    assert_eq!(said(&repository.signed(&config)?), [both()]);
    repository.restart();
    repository.write("calc/ops.py", OPS_THREE, true)?;
    assert_eq!(said(&repository.signed(&config)?), [both()]);

    let mut uncommitted = Repository::with(OPS, false)?;
    uncommitted.restart();
    uncommitted.write("calc/ops.py", OPS_THREE, false)?;
    assert_eq!(said(&uncommitted.signed(&config)?), [None]);

    let mut other = Repository::with("def add(x, y):\n    return x + y\n", true)?;
    other.write("calc/ops.py", OPS, true)?;
    other.restart();
    other.write("calc/ops.py", OPS_THREE, false)?;
    assert_eq!(said(&other.signed(&config)?), [None]);
    Ok(())
}

/// Of two waiting notices with contracts, one hook checks the first alone and gives the second
/// naming `add`; when the first was given already, the check goes to the second.
#[test]
fn one_contract_is_checked_at_each_hook() -> TestResult {
    let config = on();
    let mut repository = Repository::with(OPS, false)?;
    repository.write("calc/ops.py", OPS_THREE, false)?;
    let signed = repository.signed(&config)?;
    assert!(repository.queue(&signed, 1_000)?);
    assert!(repository.queue(&signed, 1_001)?);
    assert_eq!(repository.given(&config, &[])?, [TOLD, NAMES_ONLY]);
    let first = repository.waiting()?.first().ok_or("no notice")?.id;
    assert_eq!(repository.given(&config, &[first])?, [NAMES_ONLY, TOLD]);
    Ok(())
}

/// A contract over `coordination.max_tokens` names `add` alone: [`TOLD`] is 131 bytes, 132 with
/// its line break, 33 tokens at four bytes a token rounded up, so a cap of 32 leaves it out and one
/// of 33 lets it in.
#[test]
fn a_contract_over_the_token_cap_names_the_definition() -> TestResult {
    assert_eq!(TOLD.len(), 131);
    let capped = |max_tokens| CoordinationConfig { max_tokens, ..on() };
    let mut repository = Repository::with(OPS, false)?;
    repository.write("calc/ops.py", OPS_THREE, false)?;
    let signed = repository.signed(&on())?;
    assert!(repository.queue(&signed, 1_000)?);
    assert_eq!(repository.given(&capped(32), &[])?, [NAMES_ONLY]);
    assert_eq!(repository.given(&capped(33), &[])?, [TOLD]);
    Ok(())
}

/// With `coordination.contracts` off, an edit works out no signatures and a waiting notice keeps
/// its words, though a contract is kept for it. With it on, a signature holding a backquote cannot
/// be said, and the notice keeps the words that name the definition alone.
#[test]
fn without_the_key_or_a_sayable_signature_the_notice_names_the_definition() -> TestResult {
    let mut repository = Repository::with(OPS, false)?;
    repository.write("calc/ops.py", OPS_THREE, false)?;
    let off = CoordinationConfig::default();
    assert_eq!(said(&repository.signed(&off)?), Vec::new());
    let signed = repository.signed(&on())?;
    assert!(repository.queue(&signed, 1_000)?);
    assert_eq!(repository.given(&off, &[])?, [NAMES_ONLY]);

    let mut quoted = Repository::with(OPS, false)?;
    quoted.write("calc/ops.py", "def add(a, b='`'):\n    return a\n", false)?;
    let signed = quoted.signed(&on())?;
    assert_eq!(said(&signed), [None]);
    assert!(!quoted.queue(&signed, 1_000)?);
    assert_eq!(quoted.given(&on(), &[])?, [NAMES_ONLY]);
    Ok(())
}
