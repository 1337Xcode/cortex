//! The rules that decide what the walk leaves out and what is generated or vendored: those a
//! repository keeps in its folders (`.gitignore`, `.cortexignore`, `.gitattributes`) and in
//! `.git/info`, the user's global gitignore, and the configured extra patterns.
//!
//! Rules files inside the repository are read through [`crate::safe_read`]: one that is a link, a
//! special file or too large is reported and none of its rules apply. A rules file inside a folder
//! whose name says it may hold secrets is never opened. A line that cannot be used is reported
//! with its number (up to [`MAX_REPORTED_LINES`] per file); the file's other lines still apply. A
//! UTF-8 byte order mark before the first line is dropped, as git drops it, and braces are
//! literal, as git has no alternation. Ignore precedence follows git: the extra patterns first,
//! then the folders from the deepest up (`.cortexignore` before `.gitignore` in each), then
//! `.git/info/exclude`, then the global gitignore. The first rule file with a matching line
//! decides, and within a file the last matching line does.

use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cortex_core::RepoPath;
use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::discover::{DiscoverError, Problem, ProblemKind};
use crate::gitattributes::{AttributesFile, Verdict};
use crate::path_rules;
use crate::patterns::literal_braces;
use crate::safe_read::{self, ReadError};

/// Cortex's own ignore files, read like `.gitignore` and ahead of it.
const CORTEX_IGNORE_FILE: &str = ".cortexignore";

/// Git's ignore files.
const GIT_IGNORE_FILE: &str = ".gitignore";

/// The files that hold git attributes.
const ATTRIBUTES_FILE: &str = ".gitattributes";

/// How many bad lines of one rules file are reported one by one; the rest are counted in one
/// more problem, so a hostile file cannot fill memory with reports.
pub const MAX_REPORTED_LINES: usize = 20;

/// The UTF-8 byte order mark some editors put before the first line.
const BYTE_ORDER_MARK: &[u8] = b"\xEF\xBB\xBF";

/// Rules that hold for the whole repository.
#[derive(Debug)]
pub struct RepositoryRules {
    /// `index.extra_ignores`, which nothing overrides.
    extra: Gitignore,
    /// `.git/info/exclude`.
    info_exclude: Option<Gitignore>,
    /// The user's global gitignore.
    global: Option<Gitignore>,
    /// `.git/info/attributes`, which wins over every `.gitattributes`.
    info_attributes: Option<AttributesFile>,
}

/// The rules of one folder, linked to those of the folder around it.
#[derive(Debug)]
pub struct FolderRules {
    /// The folder.
    path: RepoPath,
    /// Its `.cortexignore`, then its `.gitignore`, as far as they exist.
    ignores: Vec<Gitignore>,
    /// Its `.gitattributes`, if any.
    attributes: Option<AttributesFile>,
    /// The rules of the folder around it.
    parent: Option<Rc<FolderRules>>,
}

/// The size limit for rules files, and where to report problems.
pub struct Reading<'a> {
    /// The largest rules file read.
    pub max_bytes: u64,
    /// Where problems go.
    pub problems: &'a mut Vec<Problem>,
}

impl RepositoryRules {
    /// Loads the extra patterns, the global gitignore at `global` and the files in `.git/info`.
    ///
    /// # Errors
    ///
    /// [`DiscoverError::ExtraIgnore`] for an invalid extra pattern.
    pub fn load(
        root: &Path,
        extra_ignores: &[String],
        global: Option<&Path>,
        reading: &mut Reading<'_>,
    ) -> Result<Self, DiscoverError> {
        let extra = extra_patterns(root, extra_ignores)?;
        let global = global.and_then(|path| load_global(root, path, reading));
        let info = info_folder(root);
        let info_exclude = info.as_ref().and_then(|info| {
            let path = RepoPath::new(".git/info/exclude").ok()?;
            load_ignore(root, &info.join("exclude"), path, reading)
        });
        let info_attributes = info.as_ref().and_then(|info| {
            let path = RepoPath::new(".git/info/attributes").ok()?;
            load_attributes(&info.join("attributes"), &path, reading)
        });
        Ok(Self {
            extra,
            info_exclude,
            global,
            info_attributes,
        })
    }

    /// Whether the entry at `native`, inside the folder `folder` describes, is left out.
    #[must_use]
    pub fn is_ignored(&self, folder: &FolderRules, native: &Path, is_dir: bool) -> bool {
        if self.extra.matched(native, is_dir).is_ignore() {
            return true;
        }
        let folders = std::iter::successors(Some(folder), |rules| rules.parent.as_deref());
        let matchers = folders
            .flat_map(|rules| rules.ignores.iter())
            .chain(self.info_exclude.iter())
            .chain(self.global.iter());
        for matcher in matchers {
            match matcher.matched(native, is_dir) {
                Match::Ignore(_) => return true,
                Match::Whitelist(_) => return false,
                Match::None => {}
            }
        }
        false
    }

    /// What the attributes say about the file at `path`, inside the folder `folder` describes.
    #[must_use]
    pub fn verdict(&self, folder: &FolderRules, path: &RepoPath) -> Verdict {
        let mut verdict = Verdict::default();
        let mut folders: Vec<&FolderRules> =
            std::iter::successors(Some(folder), |rules| rules.parent.as_deref()).collect();
        // Shallower files first, so that a deeper file's lines come later and win.
        folders.reverse();
        for rules in folders {
            if let Some(attributes) = &rules.attributes {
                attributes.apply(relative_to(&rules.path, path), &mut verdict);
            }
        }
        if let Some(attributes) = &self.info_attributes {
            attributes.apply(path.as_str(), &mut verdict);
        }
        verdict
    }
}

impl FolderRules {
    /// Loads the rules files of the folder at `path`, whose native path is `native`, inside the
    /// folder `parent` describes. `is_listed` tells which names the folder's listing holds, so
    /// that only rules files that exist are looked at; a rules file whose path says it may hold
    /// secrets is never opened.
    pub fn load(
        native: &Path,
        path: RepoPath,
        parent: Option<Rc<FolderRules>>,
        is_listed: &dyn Fn(&str) -> bool,
        reading: &mut Reading<'_>,
    ) -> Self {
        let present = |name: &str| {
            path.join(name)
                .ok()
                .filter(|file| is_listed(name) && !path_rules::is_secret(file))
        };
        let mut ignores = Vec::new();
        for name in [CORTEX_IGNORE_FILE, GIT_IGNORE_FILE] {
            if let Some(file) = present(name)
                && let Some(matcher) = load_ignore(native, &native.join(name), file, reading)
            {
                ignores.push(matcher);
            }
        }
        let attributes = present(ATTRIBUTES_FILE)
            .and_then(|file| load_attributes(&native.join(ATTRIBUTES_FILE), &file, reading));
        Self {
            path,
            ignores,
            attributes,
            parent,
        }
    }
}

/// `path` relative to `folder`, which contains it.
fn relative_to<'a>(folder: &RepoPath, path: &'a RepoPath) -> &'a str {
    if folder.is_root() {
        return path.as_str();
    }
    path.as_str()
        .strip_prefix(folder.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path.as_str())
}

/// `.git/info` when `.git` and `info` are real folders, not links.
fn info_folder(root: &Path) -> Option<PathBuf> {
    let git = root.join(".git");
    let info = git.join("info");
    let is_real_folder =
        |path: &Path| fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir());
    (is_real_folder(&git) && is_real_folder(&info)).then_some(info)
}

/// The matcher for the extra patterns; an invalid pattern stops discovery, named.
fn extra_patterns(root: &Path, patterns: &[String]) -> Result<Gitignore, DiscoverError> {
    let refuse = |pattern: &str, message: String| DiscoverError::ExtraIgnore {
        pattern: pattern.to_owned(),
        message,
    };
    let mut builder = GitignoreBuilder::new(root);
    for pattern in patterns {
        let literal = literal_braces(pattern);
        // Checked alone first, so that the error names the pattern at fault.
        GitignoreBuilder::new(root)
            .add_line(None, &literal)
            .and_then(|single| single.build())
            .map_err(|error| refuse(pattern, error.to_string()))?;
        builder
            .add_line(None, &literal)
            .map_err(|error| refuse(pattern, error.to_string()))?;
    }
    builder
        .build()
        .map_err(|error| refuse(&patterns.join(" "), error.to_string()))
}

/// The user's global gitignore at `path`, read as the user's own file: through links, within the
/// size limit. Its patterns are relative to the repository root, as git reads them. A missing file
/// is skipped, as git skips it.
fn load_global(root: &Path, path: &Path, reading: &mut Reading<'_>) -> Option<Gitignore> {
    let report = |reading: &mut Reading<'_>, kind| {
        reading.problems.push(Problem { path: None, kind });
    };
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(_) => {
            report(reading, ProblemKind::Unreadable);
            return None;
        }
    };
    let mut bytes = Vec::new();
    if file
        .take(reading.max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .is_err()
    {
        report(reading, ProblemKind::Unreadable);
        return None;
    }
    if u64::try_from(bytes.len()).map_or(true, |length| length > reading.max_bytes) {
        report(reading, ProblemKind::SkippedRulesFile);
        return None;
    }
    build_ignore(root, path, None, &bytes, reading)
}

/// The ignore file at `native` inside the repository, stored as `file`, with patterns relative
/// to `folder`; `None` when there is none or it cannot be used.
fn load_ignore(
    folder: &Path,
    native: &Path,
    file: RepoPath,
    reading: &mut Reading<'_>,
) -> Option<Gitignore> {
    let bytes = read_rules_file(native, &file, reading)?;
    build_ignore(folder, native, Some(file), &bytes, reading)
}

/// An ignore matcher from a file's `bytes`, reporting the lines that cannot be used.
fn build_ignore(
    folder: &Path,
    native: &Path,
    file: Option<RepoPath>,
    bytes: &[u8],
    reading: &mut Reading<'_>,
) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(folder);
    let mut bad_lines = Vec::new();
    let text = bytes.strip_prefix(BYTE_ORDER_MARK).unwrap_or(bytes);
    for (index, line) in text.split(|byte| *byte == b'\n').enumerate() {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let added = std::str::from_utf8(line).is_ok_and(|text| {
            builder
                .add_line(Some(native.to_owned()), &literal_braces(text))
                .is_ok()
        });
        if !added {
            bad_lines.push(index + 1);
        }
    }
    report_bad_lines(file.as_ref(), &bad_lines, reading);
    let matcher = builder.build().ok();
    if matcher.is_none() {
        reading.problems.push(Problem {
            path: file,
            kind: ProblemKind::SkippedRulesFile,
        });
    }
    matcher
}

/// The attributes file at `native` inside the repository, stored as `file`.
fn load_attributes(
    native: &Path,
    file: &RepoPath,
    reading: &mut Reading<'_>,
) -> Option<AttributesFile> {
    let bytes = read_rules_file(native, file, reading)?;
    let text = bytes.strip_prefix(BYTE_ORDER_MARK).unwrap_or(&bytes);
    let (attributes, unusable) = AttributesFile::parse(text);
    report_bad_lines(Some(file), &unusable, reading);
    Some(attributes)
}

/// Reports the first [`MAX_REPORTED_LINES`] of a file's bad lines one by one and counts the rest.
fn report_bad_lines(file: Option<&RepoPath>, lines: &[usize], reading: &mut Reading<'_>) {
    let reported = lines.iter().take(MAX_REPORTED_LINES).map(|line| Problem {
        path: file.cloned(),
        kind: ProblemKind::BadRulesLine { line: *line },
    });
    reading.problems.extend(reported);
    if lines.len() > MAX_REPORTED_LINES {
        reading.problems.push(Problem {
            path: file.cloned(),
            kind: ProblemKind::MoreBadRulesLines {
                count: lines.len() - MAX_REPORTED_LINES,
            },
        });
    }
}

/// The bytes of a rules file inside the repository; `None`, silently, when there is none, and
/// with a problem when it is a link, a special file, too large or unreadable.
fn read_rules_file(native: &Path, file: &RepoPath, reading: &mut Reading<'_>) -> Option<Vec<u8>> {
    let kind = match safe_read::read_whole(native, reading.max_bytes) {
        Ok(bytes) => return Some(bytes),
        Err(ReadError::Io(error)) if error.kind() == ErrorKind::NotFound => return None,
        Err(ReadError::NotRegular | ReadError::TooLarge) => ProblemKind::SkippedRulesFile,
        Err(ReadError::Io(_)) => ProblemKind::Unreadable,
    };
    reading.problems.push(Problem {
        path: Some(file.clone()),
        kind,
    });
    None
}
