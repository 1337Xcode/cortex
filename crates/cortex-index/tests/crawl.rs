//! Huge mode, after a full index of small repositories with the mode pinned: an inventory first,
//! then files parsed as they are asked for and as the crawl reaches them.

mod common;

use std::num::NonZeroUsize;
use std::path::Path;
use std::time::Duration;

use common::{NOW_MS, TestResult, expected, grammars, index_with_mode, ties, write_all};
use cortex_core::{FileFlags, ModeSetting, RepoId, RepoPath, ScaleMode};
use cortex_index::{Crawl, DiscoverOptions, IndexOptions, IndexReport, parse_files};
use cortex_store::{Reader, Writer, WriterOptions};

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// The scale mode setting of these tests.
const HUGE: ModeSetting = ModeSetting::Pinned(ScaleMode::Huge);

/// Whether the store holds the file at `path` as parsed.
fn is_parsed(root: &Path, path: &str) -> TestResult<bool> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let row = reader.queries().file(&RepoPath::new(path)?)?;
    Ok(row.is_some_and(|row| row.file.flags.contains(FileFlags::PARSED)))
}

/// Parses the stored files at `paths` of the repository at `root` now.
fn parse(root: &Path, paths: &[RepoPath]) -> TestResult<IndexReport> {
    let mut writer = Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(root),
        mmap_bytes: 0,
        now_ms: NOW_MS,
    })?;
    let options = IndexOptions {
        discover: DiscoverOptions {
            max_file_bytes: DEFAULT_LIMIT,
            extra_ignores: &[],
            global_gitignore: None,
        },
        parse_timeout: GENEROUS,
        threads: NonZeroUsize::MIN,
        now_ms: NOW_MS,
        mode: HUGE,
    };
    Ok(parse_files(
        root,
        &grammars()?,
        &mut writer,
        &options,
        paths,
    )?)
}

/// The waiting files of the repository at `root`, in the crawl's order once `asked` is asked for.
fn crawl_order(root: &Path, asked: &[RepoPath]) -> TestResult<Vec<String>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let mut crawl = Crawl::of_store(reader.queries())?;
    crawl.request(asked);
    Ok(crawl.next(100).iter().map(RepoPath::to_string).collect())
}

/// A huge index takes inventory only: nothing is parsed. The crawl waits on the files Cortex would
/// parse, not the generated or vendored ones; asked for `app.py`, it gives that first, then its
/// neighbour `lib.py`. Parsed alone, `app.py` cannot tie its import, since `lib.py` defines nothing
/// yet; once `lib.py` is parsed, `app.py`, which imports it, is read again and its names tied.
/// Indexed again, the parsed files stay parsed, a changed parsed file is parsed again, and a new
/// file only enters the inventory. Left to its size, the repository stays huge while its last
/// index recorded huge; that index counts it tiny, so the next one parses the files waiting.
#[test]
fn huge_mode_parses_on_demand() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("src/lib.py", "def helper():\n    pass\n"),
            ("src/app.py", "from lib import helper\n\nhelper()\n"),
            ("notes.md", "# Notes\n"),
            ("gen/out.py", "# @generated\nx = 1\n"),
            ("vendor/v.py", "y = 1\n"),
        ],
    )?;
    let limits = (2, GENEROUS, DEFAULT_LIMIT);
    let first = index_with_mode(root, limits, HUGE)?;
    assert_eq!(
        (first.files, first.parsed, first.mode),
        (5, 0, ScaleMode::Huge)
    );
    assert!(!is_parsed(root, "src/app.py")?);
    let app = RepoPath::new("src/app.py")?;
    assert_eq!(
        crawl_order(root, std::slice::from_ref(&app))?,
        ["src/app.py", "src/lib.py", "notes.md"]
    );
    let report = parse(root, &[app])?;
    assert_eq!(report.parsed, 1);
    assert_eq!(
        ties(root, "src/app.py")?,
        expected(&[("import", "helper", 1, None), ("call", "helper", 3, None)])
    );
    parse(root, &[RepoPath::new("src/lib.py")?])?;
    let helper = Some(("src/lib.py", "helper", "import_path"));
    assert_eq!(
        ties(root, "src/app.py")?,
        expected(&[
            ("import", "helper", 1, helper),
            ("call", "helper", 3, helper)
        ])
    );
    write_all(
        root,
        &[
            ("src/app.py", "from lib import helper\n\n\nhelper()\n"),
            ("src/new.py", "def fresh():\n    pass\n"),
        ],
    )?;
    let again = index_with_mode(root, limits, HUGE)?;
    assert_eq!((again.files, again.parsed), (2, 1));
    assert!(is_parsed(root, "src/app.py")? && is_parsed(root, "src/lib.py")?);
    assert!(!is_parsed(root, "src/new.py")?);
    assert_eq!(crawl_order(root, &[])?, ["src/new.py", "notes.md"]);
    let sticky = index_with_mode(root, limits, ModeSetting::Auto)?;
    assert_eq!((sticky.parsed, sticky.mode), (0, ScaleMode::Tiny));
    assert!(!is_parsed(root, "src/new.py")?);
    let tiny = index_with_mode(root, limits, ModeSetting::Auto)?;
    assert_eq!((tiny.files, tiny.parsed), (2, 2));
    assert!(is_parsed(root, "src/new.py")? && is_parsed(root, "notes.md")?);
    assert_eq!(crawl_order(root, &[])?, Vec::<String>::new());
    Ok(())
}
