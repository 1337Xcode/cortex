//! The history benchmarks' repository: the index benchmark's 10,000 generated files with a scripted
//! git history of a year, written with one `git fast-import` rather than a process per commit.
//!
//! The first commit holds every file. Then 300 commits, one a day, change 500 of the Python files
//! (every tenth file of every Python folder, 5% of the repository) in three rounds, five files a
//! commit, each round taking the files in a stride of its own: each change drops an unused
//! `legacy=None` argument from one of the file's first three helpers, so each of those helpers'
//! header line was last written by a commit of its own, and the last commit leaves every file as
//! the generator writes it. Two commits in three give a reason
//! and fix an issue, as a team's commits do; the third is a bare one-line tidy.
//!
//! `helper_0_0_1` of `pkg00/m000.py`, which the benchmarks' prompt names and their edit changes,
//! lost its argument in the second round, at transition 500 (round 1 times 500 files, plus its
//! file's place, 0, any stride times 0 being 0), so in commit 100, whose kind is the second (100
//! modulo 3 is 1): a commit with a reason that fixes an issue, the kind a why-line and an edit's
//! origin note tell of.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::common::git::git;
use crate::common::{TestResult, isolate};
use crate::synthetic;

/// Python folders (every even folder of the generator's hundred) whose files the history changes.
const FOLDERS: usize = 50;

/// Of each such folder, every how many files the history changes: every tenth, ten a folder.
const FILES_EVERY: usize = 10;

/// Files in each folder of the generator.
const FILES_PER_FOLDER: usize = 100;

/// The rounds of changes each changed file goes through, one helper each, so that each file has
/// three commits of history: as many as upkeep's warming asks of a file before it blames it.
pub(super) const ROUNDS: usize = 3;

/// The files a commit changes: few enough that the miner and the change cards count it as a normal
/// commit, not a mass change.
const FILES_A_COMMIT: usize = 5;

/// For each round, the step between the changed files that round's commits take in turn: the
/// `n`th file changed in round `r` is file `n * STRIDES[r]` modulo 500. Each stride shares no
/// factor with 500, so each round changes every file once; the strides differ, so a file's partners
/// in a commit differ from round to round, as they do in a team's history, and no two files change
/// together often enough to be told as each other's companions.
const STRIDES: [usize; ROUNDS] = [1, 7, 13];

/// Seconds in a day: the history's commits are a day apart.
const DAY_S: u64 = 86_400;

/// The people the commits name as authors, in turn.
const AUTHORS: [&str; 4] = ["Ada Field", "Ben Ito", "Chen Moss", "Dara Okoye"];

/// A file the history changes: its path, and the id its helpers' names carry (`0_0` for
/// `pkg00/m000.py`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Changed {
    /// The repository path.
    pub(super) path: String,
    /// The folder and file numbers the generator names its helpers with.
    pub(super) id: String,
}

/// The `index`th file the history changes, of [`changed_count`], folder by folder.
pub(super) fn changed(index: usize) -> Changed {
    let per_folder = FILES_PER_FOLDER / FILES_EVERY;
    let folder = 2 * (index / per_folder);
    let file = (index % per_folder) * FILES_EVERY;
    Changed {
        path: format!("pkg{folder:02}/m{file:03}.py"),
        id: format!("{folder}_{file}"),
    }
}

/// How many files the history changes.
pub(super) fn changed_count() -> usize {
    FOLDERS * FILES_PER_FOLDER / FILES_EVERY
}

/// `source`, the generated text of the changed file whose helpers carry `id`, as it was before
/// round `round` of the history: each of its helpers from `round` up to [`ROUNDS`] still takes the
/// `legacy` argument.
fn before_round(source: &str, id: &str, round: usize) -> String {
    (round..ROUNDS).fold(source.to_owned(), |text, helper| {
        let header = format!("def helper_{id}_{helper}(value):");
        let legacy = format!("def helper_{id}_{helper}(value, legacy=None):");
        text.replacen(&header, &legacy, 1)
    })
}

/// The message of commit `number` of the history, its first changed file `path`: two in three
/// give a reason and fix an issue.
fn message(number: usize, path: &str) -> String {
    let issue = 100 + number;
    match number % 3 {
        0 => format!(
            "Drop the legacy argument from the helpers of {path} and four more files\n\n\
             No caller passed it, and its default hid a wrong total.\n\nFixes #{issue}"
        ),
        1 => format!(
            "Simplify the helper signatures of {path}\n\n\
             The legacy argument made every call site read as if it mattered.\n\nFixes #{issue}"
        ),
        _ => "tidy helper signatures".to_owned(),
    }
}

/// Every file under `root` but git's and Cortex's folders, as repository paths with `/`, sorted.
fn files_of(root: &Path) -> TestResult<Vec<String>> {
    let mut found = Vec::new();
    let mut folders: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|name| name.to_str());
            if matches!(name, Some(".git" | ".cortex")) {
                continue;
            }
            if path.is_dir() {
                folders.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                found.push(relative);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Appends to `stream` a commit made on day `day` of the history (`start_s` being day 0) by the
/// `number`th author, with `message`, setting each of `files` to its contents.
fn add_commit(
    stream: &mut Vec<u8>,
    (start_s, day, number): (u64, u64, usize),
    message: &str,
    files: &[(String, Vec<u8>)],
) -> TestResult {
    let author = AUTHORS.get(number % AUTHORS.len()).unwrap_or(&AUTHORS[0]);
    let email = author.to_lowercase().replace(' ', ".");
    let when = start_s + day * DAY_S;
    writeln!(stream, "commit refs/heads/main")?;
    for role in ["author", "committer"] {
        writeln!(stream, "{role} {author} <{email}@example.com> {when} +0000")?;
    }
    writeln!(stream, "data {}\n{message}", message.len())?;
    for (path, contents) in files {
        writeln!(stream, "M 100644 inline {path}\ndata {}", contents.len())?;
        stream.extend_from_slice(contents);
        stream.push(b'\n');
    }
    Ok(())
}

/// The first commit's files: every file under `root`, the changed ones as they were before the
/// history's first round.
fn first_files(root: &Path) -> TestResult<Vec<(String, Vec<u8>)>> {
    let ids: BTreeMap<String, String> = (0..changed_count())
        .map(|index| {
            let Changed { path, id } = changed(index);
            (path, id)
        })
        .collect();
    let mut files = Vec::new();
    for path in files_of(root)? {
        let contents = std::fs::read_to_string(root.join(&path))?;
        let before = match ids.get(&path) {
            Some(id) => before_round(&contents, id, 0),
            None => contents,
        };
        files.push((path, before.into_bytes()));
    }
    Ok(files)
}

/// The fast-import stream of the whole history of the repository at `root`, its files on disk as
/// the generator wrote them, the first commit made `days` days before `now_s`.
fn history_stream(root: &Path, now_s: u64, days: u64) -> TestResult<Vec<u8>> {
    let start_s = now_s - days * DAY_S;
    let mut stream = Vec::new();
    let first = first_files(root)?;
    add_commit(
        &mut stream,
        (start_s, 0, 0),
        "Add the generated packages",
        &first,
    )?;
    let count = changed_count();
    let changes: Vec<(usize, usize)> = STRIDES
        .iter()
        .enumerate()
        .flat_map(|(round, stride)| (0..count).map(move |place| (round, place * stride % count)))
        .collect();
    for (number, commit) in changes.chunks(FILES_A_COMMIT).enumerate() {
        let mut files = Vec::new();
        for (round, index) in commit {
            let Changed { path, id } = changed(*index);
            let contents = std::fs::read_to_string(root.join(&path))?;
            files.push((path, before_round(&contents, &id, round + 1).into_bytes()));
        }
        let first_path = files.first().map_or("", |(path, _)| path.as_str());
        let day = u64::try_from(number)? + 1;
        let text = message(number, first_path);
        add_commit(&mut stream, (start_s, day, number), &text, &files)?;
    }
    Ok(stream)
}

/// Writes the generated files under `root` and makes the git repository of the history this
/// module describes there, with `home` as the user's home folder, its last commit a few days ago;
/// git's index is then set from HEAD, which holds every file as it is on disk.
///
/// # Errors
///
/// I/O errors, or a git command that fails.
pub(super) fn write_history(root: &Path, home: &Path) -> TestResult {
    synthetic::write_into(root)?;
    git(root, home, &["init", "-q"])?;
    git(root, home, &["config", "core.autocrlf", "false"])?;
    git(root, home, &["symbolic-ref", "HEAD", "refs/heads/main"])?;
    std::fs::write(root.join(".git/info/exclude"), ".cortex/\n")?;
    let now_s = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let commits = u64::try_from(ROUNDS * changed_count() / FILES_A_COMMIT)?;
    let stream = history_stream(root, now_s, commits + 3)?;
    let mut command = Command::new("git");
    command
        .args(["fast-import", "--quiet"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    isolate(&mut command, home);
    let mut child = command.spawn()?;
    child
        .stdin
        .take()
        .ok_or("fast-import's input is not piped")?
        .write_all(&stream)?;
    if !child.wait()?.success() {
        return Err("git fast-import failed".into());
    }
    git(root, home, &["reset", "-q"])
}
