//! The miner's looks, one at a time, on repositories made with the `git` program and a store
//! written as the index and the writer write it, the writer's part played by the test: when a
//! HEAD is mined, what is handed over, failures and their pauses, and stopping.

use std::process::Command;

use cortex_core::{FileFlags, RepoId, RepoPath};
use cortex_git::cochange::{FileCommits, PairCommits};
use cortex_git::mine::{FULL_EVERY_MS, Mining};
use cortex_index::Change;
use cortex_store::{FileId, FileIndex, FileRecord, Writer, WriterOptions};

use super::*;
use crate::inbox::Work;

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A repository made with the `git` program in a temporary folder, git reading no configuration
/// but the repository's own.
struct Repo {
    /// The folder holding the work tree (`repo`) and git's home folder (`home`).
    folder: tempfile::TempDir,
}

impl Repo {
    /// A new repository with no commit yet.
    fn new() -> TestResult<Self> {
        Self::init(&[])
    }

    /// A new repository with no commit yet, made by `git init` with `options`.
    fn init(options: &[&str]) -> TestResult<Self> {
        let folder = tempfile::tempdir()?;
        std::fs::create_dir(folder.path().join("repo"))?;
        std::fs::create_dir(folder.path().join("home"))?;
        let repo = Self { folder };
        let mut init = vec!["init", "-q"];
        init.extend_from_slice(options);
        repo.git(&init)?;
        repo.git(&["config", "commit.gpgsign", "false"])?;
        Ok(repo)
    }

    /// The top of the work tree.
    fn root(&self) -> PathBuf {
        self.folder.path().join("repo")
    }

    /// Runs git with `args` in the work tree and returns what it printed, trimmed.
    fn git(&self, args: &[&str]) -> TestResult<String> {
        let home = self.folder.path().join("home");
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(self.root())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Cortex Test")
            .env("GIT_AUTHOR_EMAIL", "test")
            .env("GIT_COMMITTER_NAME", "Cortex Test")
            .env("GIT_COMMITTER_EMAIL", "test");
        for name in [
            "GIT_CONFIG_GLOBAL",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "XDG_CONFIG_HOME",
        ] {
            command.env_remove(name);
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(format!("git {args:?} failed").into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    /// Writes the files at `paths`, each naming `message`, and commits them as `message`; returns
    /// the new HEAD.
    fn commit(&self, message: &str, paths: &[&str]) -> TestResult<String> {
        for path in paths {
            std::fs::write(self.root().join(path), format!("# {path} in {message}\n"))?;
        }
        let mut add = vec!["add", "--"];
        add.extend_from_slice(paths);
        self.git(&add)?;
        self.git(&["commit", "-q", "-m", message])?;
        self.git(&["rev-parse", "HEAD"])
    }
}

/// The store's writer for the repository at `root`.
fn writer(root: &Path) -> TestResult<Writer> {
    Ok(Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(root),
        mmap_bytes: 0,
        now_ms: 1,
    })?)
}

/// Stores the files at `paths` as the index would, and returns their ids, in the order of `paths`.
fn store_files(writer: &mut Writer, paths: &[&str]) -> TestResult<Vec<FileId>> {
    let mut transaction = writer.transaction()?;
    for path in paths {
        transaction.replace_file(&FileIndex {
            file: FileRecord {
                path: RepoPath::new(path)?,
                language: None,
                size_bytes: 10,
                line_count: None,
                content_hash: None,
                mtime_ns: 1,
                flags: FileFlags::empty(),
                indexed_at_ms: 1,
            },
            symbols: Vec::new(),
            refs: Vec::new(),
            passes_on: Vec::new(),
        })?;
    }
    transaction.commit()?;
    let counted = writer.queries().cochange_files()?;
    paths
        .iter()
        .map(|path| {
            counted
                .iter()
                .find(|file| file.path.as_str() == *path)
                .map(|file| file.file)
                .ok_or_else(|| format!("{path} does not count").into())
        })
        .collect()
}

/// A miner of the repository at `root` with `config`, and its inbox.
fn looker(root: PathBuf, config: Config) -> (Looker, Arc<Inbox>) {
    let inbox = Arc::new(Inbox::default());
    (Looker::new(root, config, Arc::clone(&inbox)), inbox)
}

/// Whether the writer, coming back for work, finds nothing waiting: no mining was handed over.
fn finds_nothing(inbox: &Inbox) -> bool {
    matches!(inbox.wait(Some(Instant::now())), Work::Timeout)
}

/// How many times each error the inbox keeps was met, oldest first.
fn counts(inbox: &Inbox) -> Vec<u64> {
    inbox
        .recent_errors()
        .iter()
        .map(|error| error.count)
        .collect()
}

/// Looks at `now` and `wall_ms`, expecting a mining handed to the writer; stores it at `wall_ms`
/// as the writer does, answers, lets the miner take the answer, and returns the mining.
fn mined(
    looker: &mut Looker,
    inbox: &Inbox,
    writer: &mut Writer,
    now: Instant,
    wall_ms: i64,
) -> TestResult<Mining> {
    let Next::Handed(handed) = looker.look(now, wall_ms) else {
        return Err("no mining was handed to the writer".into());
    };
    let Work::Mining(job) = inbox.wait(Some(Instant::now())) else {
        return Err("the writer found no mining".into());
    };
    let job = *job;
    mine::write(writer, &job.mining, wall_ms, None)?;
    let mining = job.mining.clone();
    job.answer(Ok(()));
    let answer = handed.answer.recv();
    if !matches!(
        looker.written(handed.head, answer, now, wall_ms),
        Next::Wait
    ) {
        return Err("the miner stopped after a write".into());
    }
    Ok(mining)
}

/// A HEAD due is mined at a look after the one that found it, once every change that had reached
/// the inbox by then is indexed: the writer taking a change is not enough, coming back for work
/// after it is. With a half-life of 1 commit, the newest commit weighs 1 and the one before 0.5.
/// Two commits change `a.py` and `b.py`: the full mining counts each file twice, at 1.5, and the
/// pair twice at 1.5, its newest commit numbered 2, so each file's edge to the other has
/// confidence 1.5 / 1.5 = 1. A third commit changes `a.py` and adds `c.py`, which the index
/// stores while the mining waits: the mining of the commit gained, numbered 3, one half-life past
/// the full mining's newest commit (its anchor, numbered 2), counts `a.py` and `c.py` once at 2 and
/// their pair once at 2, too few for an edge.
#[test]
fn a_head_is_mined_once_the_index_has_taken_its_files() -> TestResult {
    let repo = Repo::new()?;
    repo.commit("one", &["a.py", "b.py"])?;
    let second = repo.commit("two", &["a.py", "b.py"])?;
    let mut writer = writer(&repo.root())?;
    let [a, b] = store_files(&mut writer, &["a.py", "b.py"])?[..] else {
        return Err("two files stored, two ids".into());
    };
    let config = Config {
        git_half_life_commits: 1,
        ..Config::default()
    };
    let (mut looker, inbox) = looker(repo.root(), config);
    let (now, wall_ms) = (Instant::now(), now_ms());

    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    assert_eq!(inbox.git().state, GitState::Mining);
    assert!(finds_nothing(&inbox));
    inbox.push_change(Change::Everything, 1);
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    assert!(matches!(inbox.wait(Some(Instant::now())), Work::Changed(_)));
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    assert!(finds_nothing(&inbox));
    let Mining::Full(harvest) = mined(&mut looker, &inbox, &mut writer, now, wall_ms)? else {
        return Err("the first mining was not full".into());
    };
    assert_eq!(
        (harvest.head.as_str(), harvest.last_seq),
        (second.as_str(), 2)
    );
    let commits = |file, count, weighted| FileCommits {
        file,
        count,
        weighted,
    };
    let pair = |file_a, file_b, count, weighted, last_commit_seq| PairCommits {
        file_a,
        file_b,
        count,
        weighted,
        last_commit_seq,
    };
    assert_eq!(
        harvest.cochange.files,
        [commits(a, 2, 1.5), commits(b, 2, 1.5)]
    );
    assert_eq!(harvest.cochange.pairs, [pair(a, b, 2, 1.5, 2)]);
    assert_eq!(
        inbox.git(),
        GitReport {
            state: GitState::Ready,
            detail: None
        }
    );
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    assert!(finds_nothing(&inbox));

    let third = repo.commit("three", &["a.py", "c.py"])?;
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    assert!(finds_nothing(&inbox));
    inbox.push_change(Change::Paths([RepoPath::new("c.py")?].into()), 2);
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    assert!(matches!(inbox.wait(Some(Instant::now())), Work::Changed(_)));
    let [c] = store_files(&mut writer, &["c.py"])?[..] else {
        return Err("one file stored, one id".into());
    };
    assert!(finds_nothing(&inbox));
    let Mining::Added(harvest) = mined(&mut looker, &inbox, &mut writer, now, wall_ms)? else {
        return Err("the commit gained was not added".into());
    };
    assert_eq!(
        (harvest.head.as_str(), harvest.last_seq),
        (third.as_str(), 3)
    );
    assert_eq!(
        harvest.cochange.files,
        [commits(a, 1, 2.0), commits(c, 1, 2.0)]
    );
    assert_eq!(harvest.cochange.pairs, [pair(a, c, 1, 2.0, 3)]);
    Ok(())
}

/// A mining the writer cannot store is a failure that holds back a mining of the same HEAD for
/// its pause: a look within it finds HEAD due but hands nothing over, and the miner still says it
/// failed. A HEAD that moved is tried at once, the index first.
#[test]
fn a_mining_that_cannot_be_written_holds_its_head_back() -> TestResult {
    let repo = Repo::new()?;
    repo.commit("one", &["a.py", "b.py"])?;
    let mut writer = writer(&repo.root())?;
    store_files(&mut writer, &["a.py", "b.py"])?;
    let (mut looker, inbox) = looker(repo.root(), Config::default());
    let (now, wall_ms) = (Instant::now(), now_ms());
    let later = now + Duration::from_secs(1);

    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    let Next::Handed(handed) = looker.look(now, wall_ms) else {
        return Err("no mining was handed to the writer".into());
    };
    assert!(matches!(inbox.wait(Some(Instant::now())), Work::Mining(_)));
    let answer = Ok(Err("disk full".to_owned()));
    assert!(matches!(
        looker.written(handed.head, answer, now, wall_ms),
        Next::Wait
    ));
    let failed = GitReport {
        state: GitState::Failed,
        detail: Some("writing what was mined: disk full".to_owned()),
    };
    assert_eq!(inbox.git(), failed);
    let messages: Vec<String> = inbox
        .recent_errors()
        .into_iter()
        .map(|error| error.message)
        .collect();
    assert_eq!(
        messages,
        ["mining the git history failed: writing what was mined: disk full"]
    );
    assert!(matches!(looker.look(later, wall_ms), Next::Wait));
    assert_eq!(inbox.git(), failed);
    assert!(finds_nothing(&inbox));

    repo.commit("two", &["a.py"])?;
    assert!(matches!(looker.look(later, wall_ms), Next::Wait));
    assert_eq!(inbox.git().state, GitState::Mining);
    assert!(matches!(looker.look(later, wall_ms), Next::Handed(_)));
    Ok(())
}

/// A week after the last full mining, HEAD unmoved, the history is due again: a millisecond
/// before the week nothing is, at the week a look finds it due and the look after hands the
/// writer a full mining.
#[test]
fn a_week_after_the_full_mining_it_is_mined_whole_again() -> TestResult {
    let repo = Repo::new()?;
    repo.commit("one", &["a.py", "b.py"])?;
    let mut writer = writer(&repo.root())?;
    store_files(&mut writer, &["a.py", "b.py"])?;
    let (mut looker, inbox) = looker(repo.root(), Config::default());
    let (now, mined_ms) = (Instant::now(), now_ms());
    assert!(matches!(looker.look(now, mined_ms), Next::Wait));
    mined(&mut looker, &inbox, &mut writer, now, mined_ms)?;

    assert!(matches!(
        looker.look(now, mined_ms + FULL_EVERY_MS - 1),
        Next::Wait
    ));
    assert_eq!(inbox.git().state, GitState::Ready);
    assert!(finds_nothing(&inbox));
    let week_ms = mined_ms + FULL_EVERY_MS;
    assert!(matches!(looker.look(now, week_ms), Next::Wait));
    assert_eq!(inbox.git().state, GitState::Mining);
    assert!(matches!(
        mined(&mut looker, &inbox, &mut writer, now, week_ms)?,
        Mining::Full(_)
    ));
    Ok(())
}

/// A folder that is not the top of a repository has no history: the miner says so and hands
/// nothing over, look after look.
#[test]
fn a_folder_without_git_has_no_history() -> TestResult {
    let folder = tempfile::tempdir()?;
    let (mut looker, inbox) = looker(folder.path().to_path_buf(), Config::default());
    for _ in 0..2 {
        assert!(matches!(looker.look(Instant::now(), now_ms()), Next::Wait));
        assert_eq!(
            inbox.git(),
            GitReport {
                state: GitState::NoRepository,
                detail: None
            }
        );
        assert!(finds_nothing(&inbox));
    }
    Ok(())
}

/// The miner stops when the writer takes no more: when the inbox is closed as a mining would be
/// handed over, and when the writer dropped a mining without answering.
#[test]
fn the_miner_stops_when_the_writer_takes_no_more() -> TestResult {
    let repo = Repo::new()?;
    repo.commit("one", &["a.py"])?;
    let mut writer = writer(&repo.root())?;
    store_files(&mut writer, &["a.py"])?;
    let (mut looker, inbox) = looker(repo.root(), Config::default());
    let (now, wall_ms) = (Instant::now(), now_ms());
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    inbox.close();
    assert!(matches!(looker.look(now, wall_ms), Next::Stop));
    assert!(matches!(
        looker.written("h".to_owned(), Err(RecvError), now, wall_ms),
        Next::Stop
    ));
    Ok(())
}

/// A look that panics is a failure like any other: the miner says so, keeps it among the recent
/// errors, and holds every look back for 2 s; then it looks as before and finds HEAD due.
#[test]
#[allow(clippy::panic, reason = "the look under test panics on purpose")]
fn a_look_that_panics_is_a_failure_tried_again_later() -> TestResult {
    let repo = Repo::new()?;
    repo.commit("one", &["a.py"])?;
    let mut writer = writer(&repo.root())?;
    store_files(&mut writer, &["a.py"])?;
    let (mut looker, inbox) = looker(repo.root(), Config::default());
    let (now, wall_ms) = (Instant::now(), now_ms());
    let at = |seconds| now + Duration::from_secs(seconds);
    let next = looker.guarded(now, wall_ms, |_, _, _| panic!("a look under test"));
    assert!(matches!(next, Next::Wait));
    let failed = GitReport {
        state: GitState::Failed,
        detail: Some("a look stopped on an internal error, logged as a panic".to_owned()),
    };
    assert_eq!(inbox.git(), failed);
    assert_eq!(counts(&inbox), [1]);
    assert!(matches!(
        looker.guarded(at(1), wall_ms, Looker::look),
        Next::Wait
    ));
    assert_eq!(inbox.git(), failed);
    assert!(matches!(
        looker.guarded(at(2), wall_ms, Looker::look),
        Next::Wait
    ));
    assert_eq!(inbox.git().state, GitState::Mining);
    assert_eq!(counts(&inbox), [1]);
    Ok(())
}

/// A store that cannot be read fails the look: the miner says so and keeps it among the recent
/// errors, then holds every look back for 2 s, and for 4 s after the second failure in a row.
/// Once the store is there, the next look finds HEAD due.
#[test]
fn a_store_that_cannot_be_read_is_tried_again_later() -> TestResult {
    let repo = Repo::new()?;
    repo.commit("one", &["a.py"])?;
    let (mut looker, inbox) = looker(repo.root(), Config::default());
    let (now, wall_ms) = (Instant::now(), now_ms());
    let at = |seconds| now + Duration::from_secs(seconds);
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    let report = inbox.git();
    assert_eq!(report.state, GitState::Failed);
    assert!(
        report
            .detail
            .as_deref()
            .is_some_and(|detail| detail.starts_with("reading the store: ")),
        "{report:?}"
    );
    assert_eq!(counts(&inbox), [1]);
    looker.look(at(1), wall_ms);
    assert_eq!(counts(&inbox), [1]);
    looker.look(at(2), wall_ms);
    assert_eq!(counts(&inbox), [2]);
    looker.look(at(5), wall_ms);
    assert_eq!(counts(&inbox), [2]);
    looker.look(at(6), wall_ms);
    assert_eq!(counts(&inbox), [3]);
    drop(writer(&repo.root())?);
    assert!(matches!(looker.look(at(14), wall_ms), Next::Wait));
    assert_eq!(inbox.git().state, GitState::Mining);
    assert_eq!(counts(&inbox), [3]);
    Ok(())
}

/// A repository `gix` cannot read, here one whose objects are named by SHA-256 (this build's
/// `gix` knows SHA-1 only), is read with the `git` program from then on: its 64-character HEAD is
/// mined and the status says how. While a HEAD waits for the index the miner looks every 2 s, and
/// once nothing is due every 30 s, each look running git. A repository that goes away has no
/// history, and the program is forgotten with it.
#[test]
fn what_gix_cannot_read_the_git_program_reads() -> TestResult {
    let repo = Repo::init(&["--object-format=sha256"])?;
    let head = repo.commit("one", &["a.py"])?;
    let mut writer = writer(&repo.root())?;
    let [a] = store_files(&mut writer, &["a.py"])?[..] else {
        return Err("one file stored, one id".into());
    };
    let (mut looker, inbox) = looker(repo.root(), Config::default());
    let (now, wall_ms) = (Instant::now(), now_ms());
    assert_eq!(looker.look_every(), HEAD_EVERY);
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    assert!(looker.program.is_some());
    assert_eq!(looker.look_every(), HEAD_EVERY);
    let Mining::Full(harvest) = mined(&mut looker, &inbox, &mut writer, now, wall_ms)? else {
        return Err("the first mining was not full".into());
    };
    assert_eq!((harvest.head.len(), &harvest.head), (64, &head));
    let one_commit = FileCommits {
        file: a,
        count: 1,
        weighted: 1.0,
    };
    assert_eq!(harvest.cochange.files, [one_commit]);
    let report = inbox.git();
    assert_eq!(report.state, GitState::Ready);
    assert!(
        report.detail.as_deref().is_some_and(
            |detail| detail.starts_with("read with the git program, as gix could not: ")
        ),
        "{report:?}"
    );
    assert_eq!(looker.look_every(), PROGRAM_HEAD_EVERY);

    std::fs::rename(repo.root().join(".git"), repo.root().join("away"))?;
    assert!(matches!(looker.look(now, wall_ms), Next::Wait));
    assert_eq!(
        inbox.git(),
        GitReport {
            state: GitState::NoRepository,
            detail: None
        }
    );
    assert!(looker.program.is_none());
    Ok(())
}
