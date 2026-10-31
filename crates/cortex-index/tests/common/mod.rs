//! Helpers shared by the indexer's integration tests. Each test file uses part of them.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, File};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cortex_core::{ModeSetting, RepoId, RepoPath};
use cortex_index::{
    DiscoverOptions, Grammars, IndexOptions, IndexReport, index_paths, index_repository,
};
use cortex_store::{Reader, SymbolId, Writer, WriterOptions};

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Every grammar, loaded once for all the tests of one test file: compiling the queries takes
/// longer than anything a test reads.
pub fn grammars() -> Result<Arc<Grammars>, Box<dyn Error>> {
    static GRAMMARS: OnceLock<Result<Arc<Grammars>, String>> = OnceLock::new();
    GRAMMARS
        .get_or_init(|| {
            Grammars::load()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(Into::into)
}

/// The time every file is indexed at.
pub const NOW_MS: i64 = 1_700_000_000_000;

/// Indexes the repository at `root` into its `.cortex` folder on `threads` threads, allowing each
/// file `parse_timeout` and reading no file over `max_file_bytes`, its scale mode chosen from its
/// size.
pub fn index(
    root: &Path,
    threads: usize,
    parse_timeout: Duration,
    max_file_bytes: u64,
) -> TestResult<IndexReport> {
    index_with_mode(
        root,
        (threads, parse_timeout, max_file_bytes),
        ModeSetting::Auto,
    )
}

/// Indexes the repository at `root` as [`index`] does with the threads, time limit and size limit
/// of `limits`, its scale mode set by `mode`.
pub fn index_with_mode(
    root: &Path,
    (threads, parse_timeout, max_file_bytes): (usize, Duration, u64),
    mode: ModeSetting,
) -> TestResult<IndexReport> {
    let mut writer = Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(root),
        mmap_bytes: 0,
        now_ms: NOW_MS,
    })?;
    let options = IndexOptions {
        discover: DiscoverOptions {
            max_file_bytes,
            extra_ignores: &[],
            global_gitignore: None,
        },
        parse_timeout,
        threads: NonZeroUsize::new(threads).ok_or("no threads")?,
        now_ms: NOW_MS,
        mode,
    };
    Ok(index_repository(root, &grammars()?, &mut writer, &options)?)
}

/// Indexes the files at `paths` of the repository at `root` into its `.cortex` folder, as the
/// daemon does when they change: on one thread, with a minute for each file and the default size
/// limit, 1 MiB.
pub fn index_changed(root: &Path, paths: &[&str]) -> TestResult<IndexReport> {
    let mut writer = Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(root),
        mmap_bytes: 0,
        now_ms: NOW_MS,
    })?;
    let options = IndexOptions {
        discover: DiscoverOptions {
            max_file_bytes: 1_048_576,
            extra_ignores: &[],
            global_gitignore: None,
        },
        parse_timeout: Duration::from_secs(60),
        threads: NonZeroUsize::MIN,
        now_ms: NOW_MS,
        mode: ModeSetting::Auto,
    };
    let changed: Vec<RepoPath> = paths
        .iter()
        .map(|path| RepoPath::new(path))
        .collect::<Result<_, _>>()?;
    Ok(index_paths(
        root,
        &grammars()?,
        &mut writer,
        &options,
        &changed,
    )?)
}

/// Writes `text` to `path` under `root`, creating folders as needed.
pub fn write(root: &Path, path: &str, text: &[u8]) -> TestResult {
    let native = root.join(path);
    if let Some(folder) = native.parent() {
        fs::create_dir_all(folder)?;
    }
    fs::write(native, text)?;
    Ok(())
}

/// Keeps the file at `path` from being opened by this process while the returned value lives: on
/// Unix by removing every permission, on Windows by holding it open with no sharing.
pub fn lock_out(path: &Path) -> std::io::Result<Option<File>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o000))?;
        Ok(None)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        File::options()
            .read(true)
            .share_mode(0)
            .open(path)
            .map(Some)
    }
}

/// What a reference is tied to, as a test reads it: the target's file and qualified name, and how
/// it was tied.
pub type Target = (String, String, &'static str);

/// One stored reference, as a test reads it: its kind, name and line, and its target.
pub type Tie = (&'static str, String, u32, Option<Target>);

/// Writes each file of `files`, by path and text, under `root`.
pub fn write_all(root: &Path, files: &[(&str, &str)]) -> TestResult {
    for (path, text) in files {
        write(root, path, text.as_bytes())?;
    }
    Ok(())
}

/// Every reference stored for the file at `path` of the repository at `root`, in the order it was
/// written, with what it is tied to.
pub fn ties(root: &Path, path: &str) -> TestResult<Vec<Tie>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let queries = reader.queries();
    let files = queries.file_ids()?;
    let mut symbols: BTreeMap<SymbolId, (String, String)> = BTreeMap::new();
    for (file_path, id) in &files {
        for symbol in queries.symbols_in_file(*id)? {
            let name = symbol.qual_name.as_str().to_owned();
            symbols.insert(symbol.id, (file_path.as_str().to_owned(), name));
        }
    }
    let wanted = RepoPath::new(path)?;
    let (_, id) = files
        .iter()
        .find(|(file_path, _)| *file_path == wanted)
        .ok_or("no such file")?;
    let mut found = Vec::new();
    for reference in queries.refs_in_file(*id)? {
        let target = match reference.target {
            Some(target) => {
                let (file, name) = symbols
                    .get(&target.symbol)
                    .ok_or("a target with no symbol")?;
                Some((file.clone(), name.clone(), target.resolution.name()))
            }
            None => None,
        };
        let name = reference.dst_name.as_str().to_owned();
        found.push((reference.kind.name(), name, reference.line, target));
    }
    Ok(found)
}

/// An expected target: the file, the qualified name and how the reference was tied.
pub type Expected<'a> = Option<(&'a str, &'a str, &'static str)>;

/// The expected references: kind, name, line, and the target, if any.
pub fn expected(refs: &[(&'static str, &str, u32, Expected<'_>)]) -> Vec<Tie> {
    refs.iter()
        .map(|(kind, name, line, target)| {
            let target = target.map(|(file, qual_name, resolution)| {
                (file.to_owned(), qual_name.to_owned(), resolution)
            });
            (*kind, (*name).to_owned(), *line, target)
        })
        .collect()
}
