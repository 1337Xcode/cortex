//! Finding a repository's files, and what their paths, attributes and first bytes say about them:
//! their language and their flags.
//!
//! The walk lists each folder without following links, loads the folder's rules files before it
//! judges the folder's entries, and keeps a stack of folders to visit rather than recursing, so a
//! deep tree cannot exhaust the call stack. It never enters `.git` or `.cortex`, passes over
//! symbolic links and special files, and includes hidden files. A file that may hold secrets is
//! recorded by path and never opened; a file over the size limit is recorded and never read; any
//! other file has its first bytes read to tell text from binary and to spot a code generator's
//! marker. What `.gitattributes` says about generated and vendored files overrides the rest.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use cortex_core::{FileFlags, Language, RepoPath};
use thiserror::Error;

use crate::gitattributes::Verdict;
use crate::rules::{FolderRules, Reading, RepositoryRules};
use crate::safe_read;
use crate::{detect, head, path_rules};

/// Entries the walk never enters or lists: git's own data and Cortex's.
pub(crate) const PRUNED_NAMES: &[&str] = &[".git", ".cortex"];

/// How many folders deep the walk goes below the root. Real repositories stay far shallower;
/// the limit bounds the time and memory a crafted, deeply nested tree can cost.
pub const MAX_FOLDER_DEPTH: usize = 256;

/// What the walk needs besides the root.
#[derive(Debug, Clone, Copy)]
pub struct DiscoverOptions<'a> {
    /// Files larger than this are recorded with [`FileFlags::TOO_LARGE`] and never read; rules
    /// files larger than this are not applied (`index.max_file_bytes`).
    pub max_file_bytes: u64,
    /// More patterns to leave out, in `.gitignore` syntax relative to the root
    /// (`index.extra_ignores`).
    pub extra_ignores: &'a [String],
    /// The user's global gitignore file, as git's configuration names it, if any.
    pub global_gitignore: Option<&'a Path>,
}

/// A file the walk found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    /// Where it is.
    pub path: RepoPath,
    /// Its language, when it is one Cortex parses.
    pub language: Option<Language>,
    /// Its size.
    pub size_bytes: u64,
    /// Its modification time in nanoseconds since the Unix epoch. A time outside what 64 bits of
    /// nanoseconds hold (before 1678 or after 2262) is clamped to the nearest end.
    pub mtime_ns: i64,
    /// What its path, attributes and first bytes say about it.
    pub flags: FileFlags,
}

/// Something the walk could not do. It goes on with everything else.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Problem {
    /// The file or folder concerned, when it has a path Cortex can store; `None` for the global
    /// gitignore and for names that cannot be stored.
    pub path: Option<RepoPath>,
    /// What went wrong.
    pub kind: ProblemKind,
}

/// What went wrong with one file or folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProblemKind {
    /// It could not be listed, inspected or read, and is left out with everything inside it.
    Unreadable,
    /// One entry of this folder could not be read and is left out; the rest of the folder is
    /// walked.
    UnlistedEntry,
    /// This folder lies deeper than [`MAX_FOLDER_DEPTH`] below the root; it is not entered.
    TooDeep,
    /// Its name cannot be stored: it is not valid UTF-8 or it holds a backslash.
    UnstorableName,
    /// An ignore or attributes file that is a link, a special file or too large: none of its rules
    /// apply.
    SkippedRulesFile,
    /// A line of an ignore or attributes file that cannot be used; the file's other lines apply.
    BadRulesLine {
        /// The 1-based line number.
        line: usize,
    },
    /// More lines of the same file that cannot be used, beyond those reported one by one.
    MoreBadRulesLines {
        /// How many.
        count: usize,
    },
}

/// Everything the walk found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// The files, sorted by path.
    pub files: Vec<DiscoveredFile>,
    /// What could not be done, sorted.
    pub problems: Vec<Problem>,
    /// How many symbolic links and special files the walk passed over without following or
    /// reading them.
    pub skipped_entries: u64,
}

/// Why the walk could not run at all.
#[derive(Debug, Error)]
pub enum DiscoverError {
    /// The root is not a folder that can be read.
    #[error("{} is not a readable folder: {source}", .root.display())]
    Root {
        /// The root.
        root: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// A pattern in `index.extra_ignores` is not valid `.gitignore` syntax.
    #[error("index.extra_ignores: {pattern:?} is not a valid pattern: {message}")]
    ExtraIgnore {
        /// The pattern.
        pattern: String,
        /// What is wrong with it.
        message: String,
    },
}

/// A folder waiting to be listed.
struct PendingFolder {
    /// Its native path.
    native: PathBuf,
    /// Its repository path.
    path: RepoPath,
    /// The rules of the folder around it; `None` for the root.
    parent: Option<Rc<FolderRules>>,
    /// How many folders deep it is below the root, which is at depth 0.
    depth: usize,
}

/// Walks the repository at `root` and returns its files with their flags.
///
/// # Errors
///
/// [`DiscoverError::Root`] when `root` is not a readable folder and
/// [`DiscoverError::ExtraIgnore`] for an invalid extra pattern. Problems with single files and
/// folders are reported in [`Discovery::problems`] instead.
pub fn discover(root: &Path, options: &DiscoverOptions<'_>) -> Result<Discovery, DiscoverError> {
    let metadata = fs::metadata(root).map_err(|source| DiscoverError::Root {
        root: root.to_owned(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(DiscoverError::Root {
            root: root.to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotADirectory),
        });
    }
    let mut problems = Vec::new();
    let mut reading = Reading {
        max_bytes: options.max_file_bytes,
        problems: &mut problems,
    };
    let repository = RepositoryRules::load(
        root,
        options.extra_ignores,
        options.global_gitignore,
        &mut reading,
    )?;
    let mut walk = Walk {
        repository: &repository,
        max_file_bytes: options.max_file_bytes,
        files: Vec::new(),
        skipped_entries: 0,
    };
    let mut pending = vec![PendingFolder {
        native: root.to_owned(),
        path: RepoPath::root(),
        parent: None,
        depth: 0,
    }];
    while let Some(folder) = pending.pop() {
        walk.visit(folder, &mut pending, &mut reading);
    }
    let Walk {
        mut files,
        skipped_entries,
        ..
    } = walk;
    // INVARIANT: deterministic. The walk's order depends on the stack; the result does not.
    files.sort_by(|left, right| left.path.cmp(&right.path));
    problems.sort();
    // A rules file that cannot be read is also a file that cannot be read: one report is enough.
    // Problems without a path stand for different entries and are all kept.
    problems.dedup_by(|later, earlier| later == earlier && later.path.is_some());
    Ok(Discovery {
        files,
        problems,
        skipped_entries,
    })
}

/// The state of one walk.
struct Walk<'a> {
    /// The repository-wide rules.
    repository: &'a RepositoryRules,
    /// The size above which a file is not read.
    max_file_bytes: u64,
    /// The files found so far.
    files: Vec<DiscoveredFile>,
    /// The links and special files passed over so far.
    skipped_entries: u64,
}

impl Walk<'_> {
    /// Lists one folder: records its files and queues its subfolders.
    fn visit(
        &mut self,
        folder: PendingFolder,
        pending: &mut Vec<PendingFolder>,
        reading: &mut Reading<'_>,
    ) {
        let Some(entries) = list(&folder.native, &folder.path, reading) else {
            reading.problems.push(Problem {
                path: Some(folder.path),
                kind: ProblemKind::Unreadable,
            });
            return;
        };
        let is_listed = |name: &str| entries.iter().any(|(entry, _)| entry == name);
        let rules = Rc::new(FolderRules::load(
            &folder.native,
            folder.path.clone(),
            folder.parent,
            &is_listed,
            reading,
        ));
        for (name, entry) in entries {
            let Some(path) = name.to_str().and_then(|name| folder.path.join(name).ok()) else {
                reading.problems.push(Problem {
                    path: None,
                    kind: ProblemKind::UnstorableName,
                });
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                reading.problems.push(Problem {
                    path: Some(path),
                    kind: ProblemKind::Unreadable,
                });
                continue;
            };
            if PRUNED_NAMES.contains(&path.file_name()) {
                continue;
            }
            let native = entry.path();
            let is_dir = file_type.is_dir();
            if self.repository.is_ignored(&rules, &native, is_dir) {
                continue;
            }
            if is_dir && folder.depth >= MAX_FOLDER_DEPTH {
                reading.problems.push(Problem {
                    path: Some(path),
                    kind: ProblemKind::TooDeep,
                });
            } else if is_dir {
                pending.push(PendingFolder {
                    native,
                    path,
                    parent: Some(Rc::clone(&rules)),
                    depth: folder.depth + 1,
                });
            } else if file_type.is_file() {
                let verdict = self.repository.verdict(&rules, &path);
                if let Some(file) = self.classify(&entry, path, verdict, reading) {
                    self.files.push(file);
                }
            } else {
                self.skipped_entries += 1;
            }
        }
    }

    /// The file `entry` with its flags, or `None` with a problem when it cannot be inspected or
    /// its first bytes cannot be read. Its size and time come from the folder listing, which on
    /// Windows needs no handle to the file.
    fn classify(
        &self,
        entry: &fs::DirEntry,
        path: RepoPath,
        verdict: Verdict,
        reading: &mut Reading<'_>,
    ) -> Option<DiscoveredFile> {
        let found = entry.metadata().ok().and_then(|metadata| {
            classify(
                &entry.path(),
                &metadata,
                &path,
                verdict,
                self.max_file_bytes,
            )
        });
        if found.is_none() {
            reading.problems.push(Problem {
                path: Some(path),
                kind: ProblemKind::Unreadable,
            });
        }
        found
    }
}

/// The file at `native`, stored as `path`, as the walk describes it: its size and time from
/// `metadata`, its flags from its path, `verdict` (what its attributes say) and, unless it may hold
/// secrets or is over `max_file_bytes`, its first bytes. `None` when its time or its first bytes
/// cannot be read.
pub(crate) fn classify(
    native: &Path,
    metadata: &fs::Metadata,
    path: &RepoPath,
    verdict: Verdict,
    max_file_bytes: u64,
) -> Option<DiscoveredFile> {
    let size_bytes = metadata.len();
    let mtime_ns = mtime_ns(metadata.modified().ok()?);
    let mut found = Found::from_path(path);
    let mut start = None;
    // A file that may hold secrets is never opened, and one over the limit is never read.
    if !found.is_secret() && size_bytes > max_file_bytes {
        found.fixed = found.fixed.with(FileFlags::TOO_LARGE);
    } else if !found.is_secret() {
        let bytes = safe_read::read_start(native, head::HEAD_BYTES).ok()?;
        let judged = head::judge(&bytes);
        if judged.is_binary {
            found.fixed = found.fixed.with(FileFlags::BINARY);
        }
        found.is_generated |= judged.is_generated;
        start = Some(bytes);
    }
    let language = detect::detect(path, start.as_deref(), verdict.language);
    Some(DiscoveredFile {
        flags: found.flags(verdict),
        language,
        path: path.clone(),
        size_bytes,
        mtime_ns,
    })
}

/// The entries of the folder at `native`, stored as `path`, sorted by name, without following
/// links; `None` when the folder cannot be listed. An entry that cannot be read is left out with
/// a problem, and the rest of the folder is kept.
fn list(
    native: &Path,
    path: &RepoPath,
    reading: &mut Reading<'_>,
) -> Option<Vec<(std::ffi::OsString, fs::DirEntry)>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(native).ok()? {
        match entry {
            Ok(entry) => entries.push((entry.file_name(), entry)),
            Err(_) => reading.problems.push(Problem {
                path: Some(path.clone()),
                kind: ProblemKind::UnlistedEntry,
            }),
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Some(entries)
}

/// What has been found out about one file so far.
#[derive(Debug)]
struct Found {
    /// Findings nothing overrides: may hold secrets, too large, binary.
    fixed: FileFlags,
    /// Its path or first lines say a tool wrote it.
    is_generated: bool,
    /// Its path says a third party wrote it.
    is_vendored: bool,
}

impl Found {
    /// What the path of a file says.
    fn from_path(path: &RepoPath) -> Self {
        let flags = path_rules::flags_for(path);
        let fixed = if path_rules::is_secret(path) {
            FileFlags::SECRET_PATTERN
        } else {
            FileFlags::empty()
        };
        Self {
            fixed,
            is_generated: flags.contains(FileFlags::GENERATED),
            is_vendored: flags.contains(FileFlags::VENDORED),
        }
    }

    /// Whether the file may hold secrets.
    fn is_secret(&self) -> bool {
        self.fixed.contains(FileFlags::SECRET_PATTERN)
    }

    /// The flags, with the attributes' verdict overriding the generated and vendored findings.
    fn flags(&self, verdict: Verdict) -> FileFlags {
        let mut flags = self.fixed;
        if verdict.is_generated.unwrap_or(self.is_generated) {
            flags = flags.with(FileFlags::GENERATED);
        }
        if verdict.is_vendored.unwrap_or(self.is_vendored) {
            flags = flags.with(FileFlags::VENDORED);
        }
        flags
    }
}

/// Whether the file `file` describes is still on disk as described: the same size and
/// modification time, as the walk compares files.
pub(crate) fn is_as_described(root: &Path, file: &DiscoveredFile) -> bool {
    fs::symlink_metadata(file.path.to_native(root)).is_ok_and(|metadata| {
        metadata.len() == file.size_bytes
            && metadata
                .modified()
                .is_ok_and(|time| mtime_ns(time) == file.mtime_ns)
    })
}

/// `time` in nanoseconds since the Unix epoch, clamped to what 64 bits hold.
fn mtime_ns(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_nanos()).map_or(i64::MIN, |nanos| -nanos),
    }
}
