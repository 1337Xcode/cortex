//! The meter in the daemon: the transcripts the agents' stops name, read on the writer's thread
//! from where each read last stopped, and the tokens each response took written as `usage` rows.
//!
//! The end of a session's answer, a subagent's stop (naming the subagent's own transcript) and a
//! session's end each name a transcript. It waits until the writer is free, and is read only when
//! its path is absolute, its name ends in `.jsonl`, it lies inside a folder its agent keeps its
//! transcripts in once every link on the way is followed, and it is a regular file itself, opened
//! without following a link. A pass reads the whole lines within [`READ_PASS_BYTES`] of it, and a
//! transcript longer than that waits again, to be read on in the next pass, with the hook events
//! waiting recorded between. A turn is a response's place in its transcript, so a transcript read
//! again from its start, as after a restart, writes the same rows again.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use cortex_core::{Agent, Event, HookKind, Identifier};
use cortex_index::{ReadError, open_regular};
use cortex_meter::{MeterError, Tail, TranscriptFormat, Turn, format_of};
use cortex_store::{UsageRow, Writer};

use crate::host::{count_failed, now_ms};
use crate::inbox::Inbox;

/// Most transcripts waiting to be read. A read takes every line written since the last, so past
/// this the oldest waiting are let go: the next stop of their session reads what they would have.
pub(crate) const MAX_TRANSCRIPTS_WAITING: usize = 256;

/// Most bytes of whole lines one pass reads of a transcript; a line longer than this is read whole
/// as far as the meter's longest line. A transcript longer than a pass is read on in the next, the
/// hook events waiting recorded between: read whole, as after a restart, a long transcript of
/// short responses would hold the writer for seconds, as each response is parsed apart.
pub(crate) const READ_PASS_BYTES: u64 = 1024 * 1024;

/// Most transcripts the meter keeps its place in. Past this the one read longest ago is
/// forgotten; when a stop names it again it is read from its start, which writes the same rows.
pub(crate) const MAX_TAILS: usize = 256;

/// A transcript a hook event named, and the session whose usage it reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Transcript {
    /// The session: a subagent's own for its stop.
    session_id: Identifier,
    /// The agent that writes it.
    agent: Agent,
    /// Where the event says it is.
    path: PathBuf,
}

impl Transcript {
    /// The transcript `event` names for the meter: that of the end of an answer, of a subagent's
    /// stop or of a session's end, when the event names one.
    pub(crate) fn named_by(event: &Event) -> Option<Self> {
        let is_stop = matches!(
            event.hook,
            HookKind::Stop | HookKind::SubagentStop | HookKind::SessionEnd
        );
        let path = event.transcript_path.as_ref().filter(|_| is_stop)?;
        Some(Self {
            session_id: event.session_id.clone(),
            agent: event.agent,
            path: path.clone(),
        })
    }
}

/// Why a transcript was not read.
#[derive(Debug)]
enum Unread {
    /// Its path is not absolute or its name does not end in `.jsonl`.
    NotTranscript,
    /// It is not inside a folder its agent keeps its transcripts in.
    Outside,
    /// It is a link, a folder or a special file.
    NotRegular,
    /// It, or its agent's folder, could not be found, opened or read.
    Io(std::io::Error),
}

impl fmt::Display for Unread {
    /// Why, in words that name no path, so that the same refusal of many transcripts is one error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTranscript => formatter.write_str("its path is not one of a transcript"),
            Self::Outside => formatter.write_str("it is not in its agent's transcripts folder"),
            Self::NotRegular => formatter.write_str("it is not a regular file"),
            Self::Io(error) => write!(formatter, "it cannot be read: {error}"),
        }
    }
}

impl From<ReadError> for Unread {
    /// The refusal a safe open gave.
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::NotRegular | ReadError::TooLarge => Self::NotRegular,
            ReadError::Io(error) => Self::Io(error),
        }
    }
}

impl From<MeterError> for Unread {
    /// The error the reading met.
    fn from(error: MeterError) -> Self {
        match error {
            MeterError::Read(error) => Self::Io(error),
        }
    }
}

/// What one pass over a transcript read.
#[derive(Debug)]
struct Pass {
    /// The path its reading is kept under.
    kept_as: PathBuf,
    /// The responses it added.
    turns: Vec<Turn>,
    /// Whether it read from the transcript's start: the first pass over it, or one over a
    /// transcript written again shorter than where the last pass stopped.
    is_from_start: bool,
    /// Whether the transcript holds more after this pass, for the next.
    has_more: bool,
}

/// How far the reading of one transcript has gone, and when it was last read.
#[derive(Debug)]
struct Kept {
    /// Where the reading stands.
    tail: Tail,
    /// The number of the meter's read that last read it.
    read_number: u64,
}

/// The transcripts that wait to be read, and where the reading of each read before stands.
#[derive(Debug)]
pub(crate) struct Meter {
    /// The folders each agent whose transcripts are read keeps them in.
    folders: BTreeMap<Agent, Vec<PathBuf>>,
    /// The transcripts waiting, in the order their events were recorded.
    waiting: Vec<Transcript>,
    /// Each transcript read, by its canonical path.
    tails: BTreeMap<PathBuf, Kept>,
    /// How many reads the meter has made, to tell which transcript was read longest ago.
    reads: u64,
}

impl Meter {
    /// A meter for the user whose home folder is `home`, in an environment whose variables
    /// `variable` reads, nothing read yet. Where each agent keeps its transcripts is settled now.
    pub(crate) fn new(home: &Path, variable: &dyn Fn(&str) -> Option<OsString>) -> Self {
        let folders = Agent::ALL
            .iter()
            .filter_map(|agent| Some((*agent, format_of(*agent)?.folders(home, variable))))
            .collect();
        Self {
            folders,
            waiting: Vec::new(),
            tails: BTreeMap::new(),
            reads: 0,
        }
    }

    /// Adds the transcripts that recorded events just named, each in place of one waiting at the
    /// same path, the latest event's session kept, and lets the oldest waiting go past
    /// [`MAX_TRANSCRIPTS_WAITING`].
    pub(crate) fn wait(&mut self, transcripts: impl IntoIterator<Item = Transcript>) {
        for transcript in transcripts {
            // A read takes every line written since the last, so one wait a path is enough.
            self.waiting
                .retain(|waiting| waiting.path != transcript.path);
            self.waiting.push(transcript);
        }
        // INVARIANT: budgeted (at most MAX_TRANSCRIPTS_WAITING transcripts wait to be read)
        let over = self.waiting.len().saturating_sub(MAX_TRANSCRIPTS_WAITING);
        if over > 0 {
            tracing::warn!(dropped = over, "too many transcripts waiting to be read");
            self.waiting.drain(..over);
        }
    }

    /// Whether no transcript waits.
    pub(crate) fn is_idle(&self) -> bool {
        self.waiting.is_empty()
    }

    /// Makes one pass over each transcript waiting and writes the usage of the responses it adds
    /// in a transaction of `writer` of its own, running `between` after each pass to record the
    /// hook events waiting and hand back the transcripts they named. A transcript with more to
    /// read after its pass waits again; it and those named meanwhile are read the next time. A
    /// transcript that may not or cannot be read is noted in `inbox` and logged; usage the store
    /// refuses is counted there with the error, and its transcript read from its start when an
    /// event names it again, so that no row is lost.
    pub(crate) fn read(
        &mut self,
        writer: &mut Writer,
        inbox: &Inbox,
        mut between: impl FnMut(&mut Writer) -> Vec<Transcript>,
    ) {
        for transcript in std::mem::take(&mut self.waiting) {
            // An agent whose transcripts Cortex does not read yet names them all the same.
            let Some(format) = format_of(transcript.agent) else {
                continue;
            };
            match self.pass(&transcript.path, (transcript.agent, format)) {
                Ok(pass) => {
                    let has_more = pass.has_more;
                    let is_written = self.write(writer, inbox, (&transcript.session_id, pass));
                    // A transcript whose usage the store refused starts over when it is next
                    // named, not at once, which would refuse it again for ever.
                    if has_more && is_written {
                        self.wait([transcript]);
                    }
                }
                Err(unread) => {
                    // A path's Debug form escapes what could forge a log line.
                    tracing::warn!(
                        path = ?transcript.path,
                        reason = %unread,
                        "a transcript is not read"
                    );
                    inbox.note_error(&format!("a transcript is not read: {unread}"), now_ms());
                }
            }
            let more = between(writer);
            self.wait(more);
        }
    }

    /// One pass over the transcript at `path`, which `agent` wrote in `format`: the responses the
    /// whole lines within [`READ_PASS_BYTES`] of where the last pass stopped add.
    fn pass(
        &mut self,
        path: &Path,
        (agent, format): (Agent, &dyn TranscriptFormat),
    ) -> Result<Pass, Unread> {
        let folders = self.folders.get(&agent).map_or(&[][..], Vec::as_slice);
        let kept_as = transcript_in(path, folders)?;
        let mut file = open_regular(path)?;
        let mut tail = self
            .tails
            .remove(&kept_as)
            .map(|kept| kept.tail)
            .unwrap_or_default();
        let start = tail.offset();
        let read = tail.read(&mut file, format, READ_PASS_BYTES);
        let offset = tail.offset();
        self.reads += 1;
        let read_number = self.reads;
        self.tails
            .insert(kept_as.clone(), Kept { tail, read_number });
        self.forget_oldest();
        let turns = read?;
        let length = file.metadata().map_err(Unread::Io)?.len();
        Ok(Pass {
            kept_as,
            turns,
            // A tail goes back only when it starts over, at a file shorter than its place.
            is_from_start: start == 0 || offset < start,
            // A pass that moved and stopped short of the end has more to read; one that did not
            // move found only a line still being written.
            has_more: offset != start && offset < length,
        })
    }

    /// Writes the usage of `pass` as rows of `session_id`, in one transaction of `writer`, and
    /// says whether it was written. A pass from the transcript's start replaces every row of the
    /// session, so that none past the transcript's end stays. When the store refuses it, the
    /// failure is counted in `inbox` and the transcript's place forgotten, so that it is read from
    /// its start next time.
    fn write(
        &mut self,
        writer: &mut Writer,
        inbox: &Inbox,
        (session_id, pass): (&Identifier, Pass),
    ) -> bool {
        let Pass {
            kept_as,
            turns,
            is_from_start,
            ..
        } = pass;
        if turns.is_empty() && !is_from_start {
            return true;
        }
        let rows: Vec<UsageRow> = turns
            .into_iter()
            .map(|turn| UsageRow {
                session_id: session_id.clone(),
                turn: turn.ordinal,
                at_ms: turn.at_ms,
                // A model's name that is no name, too long or holding control characters, is
                // stored as no model rather than as text from the transcript.
                model: Identifier::new(&turn.model).ok(),
                input: turn.usage.input,
                cache_write: turn.usage.cache_write,
                cache_write_1h: turn.usage.cache_write_1h,
                cache_read: turn.usage.cache_read,
                output: turn.usage.output,
                reasoning: turn.usage.reasoning,
                service_tier: turn.service.tier,
                speed: turn.service.speed,
                inference_geo: turn.service.inference_geo,
            })
            .collect();
        let written = writer.transaction().and_then(|mut transaction| {
            if is_from_start {
                transaction.forget_usage(session_id)?;
            }
            transaction.replace_usage(&rows)?;
            transaction.commit()
        });
        if let Err(error) = written {
            self.tails.remove(&kept_as);
            count_failed(inbox, "the usage a transcript reports", &error);
            return false;
        }
        true
    }

    /// Forgets the transcript read longest ago while more than [`MAX_TAILS`] are kept.
    fn forget_oldest(&mut self) {
        // INVARIANT: budgeted (the meter keeps its place in at most MAX_TAILS transcripts)
        while self.tails.len() > MAX_TAILS {
            let oldest = self
                .tails
                .iter()
                .min_by_key(|(_, kept)| kept.read_number)
                .map(|(path, _)| path.clone());
            let Some(oldest) = oldest else {
                return;
            };
            self.tails.remove(&oldest);
        }
    }
}

/// The canonical path of the transcript at `path`, when its agent's transcripts are kept in
/// `folders`: `path` is absolute, its name ends in `.jsonl`, it lies inside one of `folders` by
/// their text alone, and still does once every link on the way to either is followed. A folder
/// that does not exist holds nothing.
fn transcript_in(path: &Path, folders: &[PathBuf]) -> Result<PathBuf, Unread> {
    let is_named_as_one = path.is_absolute() && path.extension().is_some_and(|ext| ext == "jsonl");
    if !is_named_as_one {
        return Err(Unread::NotTranscript);
    }
    // INVARIANT: local-only (a payload's path is placed inside a transcripts folder by its text
    // before anything on disk is touched: following a network path would open a connection, and
    // on Windows send the user's credentials, before any check could refuse it)
    if !folders.iter().any(|folder| is_inside_by_text(path, folder)) {
        return Err(Unread::Outside);
    }
    let canonical = std::fs::canonicalize(path).map_err(Unread::Io)?;
    let is_inside = folders
        .iter()
        .filter_map(|folder| std::fs::canonicalize(folder).ok())
        .any(|folder| canonical.starts_with(folder));
    if !is_inside {
        return Err(Unread::Outside);
    }
    Ok(canonical)
}

/// Whether `path` lies inside `folder` by their text alone: `path` names no `.` or `..`, and its
/// first components are the folder's, compared without case where the file system ignores it. On
/// Windows the first is the prefix, so a network or device path (`\\host\share`, `\\?\C:`) is
/// never inside a folder on a drive.
fn is_inside_by_text(path: &Path, folder: &Path) -> bool {
    let is_plain = path
        .components()
        .all(|component| !matches!(component, Component::CurDir | Component::ParentDir));
    let mut own = path.components();
    is_plain
        && folder
            .components()
            .all(|part| own.next().is_some_and(|mine| is_same_part(mine, part)))
}

/// Whether two components of a path name the same folder or file: on Windows, whose file systems
/// ignore case, the same but for the case of ASCII letters.
#[cfg(windows)]
fn is_same_part(one: Component<'_>, other: Component<'_>) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let folded = |part: Component<'_>| -> Vec<u16> {
        part.as_os_str()
            .encode_wide()
            .map(|unit| match u8::try_from(unit) {
                Ok(byte) => u16::from(byte.to_ascii_lowercase()),
                Err(_) => unit,
            })
            .collect()
    };
    folded(one) == folded(other)
}

/// Whether two components of a path name the same folder or file: alike byte for byte.
#[cfg(not(windows))]
fn is_same_part(one: Component<'_>, other: Component<'_>) -> bool {
    one == other
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cortex_core::RepoId;
    use cortex_meter::Claude;
    use cortex_store::{SessionRecord, WriterOptions};

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A Claude Code transcript of a session that searched with Grep, reduced to what a usage
    /// reader reads. Its four responses, read off its lines: 3,702 input, 29,797 written to the
    /// cache and 100 output; then 2, 3,819 written, 29,797 read and 95; then 2, 155, 33,616 and
    /// 360, written four times; then 2, 763, 33,771 and 3.
    const GREP_SESSION: &str =
        include_str!("../../../tests/fixtures/claude/2.1.316/transcripts/grep-session.jsonl");

    /// An event of `hook` from session `s1` at 1,000 ms naming `transcript`.
    fn event(
        hook: HookKind,
        transcript: Option<&Path>,
    ) -> Result<Event, Box<dyn std::error::Error>> {
        Ok(Event {
            event_id: cortex_core::Ulid::from_seed(1_000, b"event"),
            ts_ms: 1_000,
            agent: Agent::Claude,
            agent_version: None,
            session_id: Identifier::new("s1")?,
            parent_session_id: None,
            cwd: PathBuf::from("/repo"),
            repo_root: PathBuf::from("/repo"),
            hook,
            source: None,
            tool: None,
            tool_target: None,
            tool_result: None,
            prompt_text: None,
            transcript_path: transcript.map(Path::to_path_buf),
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
            is_probe: false,
        })
    }

    /// The transcript of session `session` at `path`, written by Claude Code.
    fn transcript(session: &str, path: &Path) -> Result<Transcript, Box<dyn std::error::Error>> {
        Ok(Transcript {
            session_id: Identifier::new(session)?,
            agent: Agent::Claude,
            path: path.to_path_buf(),
        })
    }

    /// A home folder whose Claude Code projects folder holds `C--repo/s1.jsonl`, the Grep
    /// session's transcript, and the path of that file.
    fn home_with_transcript() -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let project = projects(home.path()).join("C--repo");
        fs::create_dir_all(&project)?;
        let path = project.join("s1.jsonl");
        fs::write(&path, GREP_SESSION)?;
        Ok((home, path))
    }

    /// The folder Claude Code keeps transcripts in under `home`, its configuration not moved.
    fn projects(home: &Path) -> PathBuf {
        home.join(".claude").join("projects")
    }

    /// A writer for a store in `root`.
    fn writer(root: &Path) -> Result<Writer, Box<dyn std::error::Error>> {
        Ok(Writer::open(&WriterOptions {
            folder: &root.join(".cortex"),
            repo_id: &RepoId::of(root),
            mmap_bytes: 0,
            now_ms: 1,
        })?)
    }

    /// Records session `session` in the store `writer` writes.
    fn record_session(writer: &mut Writer, session: &str) -> TestResult {
        let mut transaction = writer.transaction()?;
        transaction.record_session(&SessionRecord {
            session_id: Identifier::new(session)?,
            agent: Agent::Claude,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 1,
        })?;
        Ok(transaction.commit()?)
    }

    /// A usage row's turn, input, cache writes, cache reads and output.
    type Counts = (u32, u64, u64, u64, u64);

    /// The turn, input, cache writes, cache reads and output of each usage row of `session`.
    fn usage_of(writer: &Writer, session: &str) -> Result<Vec<Counts>, Box<dyn std::error::Error>> {
        Ok(writer
            .queries()
            .usage_of_session(&Identifier::new(session)?)?
            .into_iter()
            .map(|row| {
                (
                    row.turn,
                    row.input,
                    row.cache_write,
                    row.cache_read,
                    row.output,
                )
            })
            .collect())
    }

    /// The end of an answer, a subagent's stop and a session's end name their transcript for the
    /// meter; a prompt does not, though its payload names it too, and a stop that names none
    /// leaves nothing to read.
    #[test]
    fn only_the_stops_name_a_transcript_to_read() -> TestResult {
        let path = Path::new("/home/dev/.claude/projects/C--repo/s1.jsonl");
        for hook in [HookKind::Stop, HookKind::SubagentStop, HookKind::SessionEnd] {
            assert_eq!(
                Transcript::named_by(&event(hook, Some(path))?),
                Some(transcript("s1", path)?),
                "{hook:?}"
            );
            assert_eq!(Transcript::named_by(&event(hook, None)?), None, "{hook:?}");
        }
        for hook in [HookKind::Prompt, HookKind::SessionStart, HookKind::PostTool] {
            assert_eq!(
                Transcript::named_by(&event(hook, Some(path))?),
                None,
                "{hook:?}"
            );
        }
        Ok(())
    }

    /// Usage the store refuses, here for a session it does not hold yet, is counted as a failed
    /// batch, and the transcript is read from its start the next time: once the session is held,
    /// all four responses are written, though the first read had passed every line.
    #[test]
    fn usage_the_store_refuses_is_read_again_from_the_start() -> TestResult {
        let (home, path) = home_with_transcript()?;
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let inbox = Inbox::default();
        let mut meter = Meter::new(home.path(), &|_| None);
        meter.wait([transcript("s1", &path)?]);
        meter.read(&mut writer, &inbox, |_| Vec::new());
        assert_eq!(inbox.counts().failed_batches, 1);
        assert_eq!(usage_of(&writer, "s1")?, []);
        record_session(&mut writer, "s1")?;
        meter.wait([transcript("s1", &path)?]);
        meter.read(&mut writer, &inbox, |_| Vec::new());
        assert_eq!(
            usage_of(&writer, "s1")?,
            [
                (1, 3_702, 29_797, 0, 100),
                (2, 2, 3_819, 29_797, 95),
                (3, 2, 155, 33_616, 360),
                (4, 2, 763, 33_771, 3),
            ]
        );
        assert_eq!(inbox.counts().failed_batches, 1);
        Ok(())
    }

    /// A transcript written again from its start, shorter than where the last read stopped,
    /// replaces its session's usage: once the Grep session's four responses are written, the file
    /// is cut back to the lines before its second response, which hold the first alone, and read
    /// again. Only the first response's row is left, where the three responses no longer in the
    /// file would otherwise keep their rows.
    #[test]
    fn a_transcript_written_again_replaces_its_usage() -> TestResult {
        let (home, path) = home_with_transcript()?;
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        record_session(&mut writer, "s1")?;
        let inbox = Inbox::default();
        let mut meter = Meter::new(home.path(), &|_| None);
        meter.wait([transcript("s1", &path)?]);
        meter.read(&mut writer, &inbox, |_| Vec::new());
        assert_eq!(usage_of(&writer, "s1")?.len(), 4);
        let before_second: String = GREP_SESSION
            .lines()
            .take_while(|line| !line.contains("msg_fixture0002"))
            .map(|line| format!("{line}\n"))
            .collect();
        assert!(before_second.len() < GREP_SESSION.len());
        fs::write(&path, before_second)?;
        meter.wait([transcript("s1", &path)?]);
        meter.read(&mut writer, &inbox, |_| Vec::new());
        assert_eq!(usage_of(&writer, "s1")?, [(1, 3_702, 29_797, 0, 100)]);
        assert_eq!(inbox.counts().failed_batches, 0);
        Ok(())
    }

    /// A transcript waits once however many stops named it, for the latest event's session; the
    /// events recorded after its pass name it again for the next, which finds nothing new.
    #[test]
    fn a_transcript_named_twice_is_read_once() -> TestResult {
        let (home, path) = home_with_transcript()?;
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        record_session(&mut writer, "s1")?;
        let inbox = Inbox::default();
        let mut meter = Meter::new(home.path(), &|_| None);
        meter.wait([transcript("s2", &path)?, transcript("s1", &path)?]);
        assert_eq!(meter.waiting, [transcript("s1", &path)?]);
        let mut passes = 0;
        meter.read(&mut writer, &inbox, |_| {
            passes += 1;
            transcript("s1", &path).into_iter().collect()
        });
        assert_eq!(passes, 1);
        assert!(!meter.is_idle(), "the transcript the events named waits");
        assert_eq!(usage_of(&writer, "s1")?.len(), 4);
        meter.read(&mut writer, &inbox, |_| Vec::new());
        assert!(meter.is_idle());
        assert_eq!(usage_of(&writer, "s1")?.len(), 4);
        assert_eq!(inbox.recent_errors(), []);
        Ok(())
    }

    /// Writes at `path` a transcript of 2,000 responses, each the first response's line of the
    /// Grep session with a message id of its own, all as long as one another, and returns that
    /// length with the line ending.
    fn long_transcript(path: &Path) -> Result<usize, Box<dyn std::error::Error>> {
        let first = GREP_SESSION.lines().nth(21).ok_or("line 22")?;
        let mut text = String::new();
        for number in 0..2_000 {
            let line = first.replace("msg_fixture0001", &format!("msg_long{number:06}"));
            text.push_str(&line);
            text.push('\n');
        }
        fs::write(path, &text)?;
        Ok(first.len() + "msg_long000000".len() - "msg_fixture0001".len() + 1)
    }

    /// A transcript longer than a pass waits again after it and is read on in the next, the events
    /// waiting recorded between: its lines are all `line` bytes long, so the first pass takes the
    /// `1_048_576 / line` whole lines within its 1 MiB, the second the rest of the 2,000, and the
    /// last line read ends the file, so nothing waits after it.
    #[test]
    fn a_transcript_longer_than_a_pass_is_read_on_in_the_next() -> TestResult {
        let (home, path) = home_with_transcript()?;
        let line = long_transcript(&path)?;
        let first_pass = 1_048_576 / line;
        assert!(first_pass < 2_000, "the transcript is longer than a pass");
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        record_session(&mut writer, "s1")?;
        let inbox = Inbox::default();
        let mut meter = Meter::new(home.path(), &|_| None);
        meter.wait([transcript("s1", &path)?]);
        let mut passes = 0;
        meter.read(&mut writer, &inbox, |_| {
            passes += 1;
            Vec::new()
        });
        assert_eq!((passes, usage_of(&writer, "s1")?.len()), (1, first_pass));
        assert_eq!(meter.waiting, [transcript("s1", &path)?]);
        meter.read(&mut writer, &inbox, |_| {
            passes += 1;
            Vec::new()
        });
        let turns: Vec<u32> = usage_of(&writer, "s1")?.iter().map(|row| row.0).collect();
        assert_eq!(turns, (1..=2_000).collect::<Vec<u32>>());
        assert_eq!(passes, 2);
        assert!(meter.is_idle());
        Ok(())
    }

    /// A pass whose usage the store refuses does not wait again with the rest of its transcript:
    /// the transcript starts over from its start when an event names it next.
    #[test]
    fn a_refused_pass_waits_for_the_next_stop() -> TestResult {
        let (home, path) = home_with_transcript()?;
        long_transcript(&path)?;
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let inbox = Inbox::default();
        let mut meter = Meter::new(home.path(), &|_| None);
        meter.wait([transcript("s1", &path)?]);
        meter.read(&mut writer, &inbox, |_| Vec::new());
        assert_eq!(inbox.counts().failed_batches, 1);
        assert!(meter.is_idle());
        record_session(&mut writer, "s1")?;
        meter.wait([transcript("s1", &path)?]);
        for _ in 0..2 {
            meter.read(&mut writer, &inbox, |_| Vec::new());
        }
        assert_eq!(usage_of(&writer, "s1")?.len(), 2_000);
        Ok(())
    }

    /// The reading rules: a relative path and a name that does not end in `.jsonl` are no
    /// transcript; a file outside the projects folder is refused, and so is every file when no
    /// folder exists; a missing file cannot be read. A transcript in a project's folder, or in a
    /// subagents folder below it, is read.
    #[test]
    fn a_transcript_is_read_only_in_its_agent_s_folder() -> TestResult {
        let (home, path) = home_with_transcript()?;
        let folder = projects(home.path());
        let elsewhere = home.path().join("elsewhere");
        fs::create_dir_all(&elsewhere)?;
        fs::write(elsewhere.join("s1.jsonl"), GREP_SESSION)?;
        fs::write(folder.join("C--repo").join("s1.json"), GREP_SESSION)?;
        let subagents = folder.join("C--repo").join("s1").join("subagents");
        fs::create_dir_all(&subagents)?;
        fs::write(subagents.join("agent-a1.jsonl"), GREP_SESSION)?;
        let outcome = |path: &Path, folder: &Path| match transcript_in(path, &[folder.into()]) {
            Ok(_) => "read".to_owned(),
            Err(Unread::Io(_)) => "cannot be read".to_owned(),
            Err(unread) => unread.to_string(),
        };
        let not_one = "its path is not one of a transcript";
        let cases = [
            (PathBuf::from("C--repo/s1.jsonl"), not_one),
            (folder.join("C--repo").join("s1.json"), not_one),
            (
                elsewhere.join("s1.jsonl"),
                "it is not in its agent's transcripts folder",
            ),
            (folder.join("C--repo").join("s2.jsonl"), "cannot be read"),
            (subagents.join("agent-a1.jsonl"), "read"),
            (path.clone(), "read"),
        ];
        for (path, expected) in &cases {
            assert_eq!(outcome(path, &folder), *expected, "{}", path.display());
        }
        let no_folder = tempfile::tempdir()?;
        assert_eq!(
            outcome(&path, &projects(no_folder.path())),
            "it is not in its agent's transcripts folder"
        );
        Ok(())
    }

    /// With Claude Code's configuration moved by `CLAUDE_CONFIG_DIR`, a transcript in the moved
    /// folder's `projects` is read, and one in the home folder's own still is.
    #[test]
    fn a_moved_configuration_folder_holds_transcripts_too() -> TestResult {
        let (home, path) = home_with_transcript()?;
        let config = home.path().join("claude-work");
        let moved = config.join("projects").join("C--repo").join("s2.jsonl");
        fs::create_dir_all(moved.parent().ok_or("a folder")?)?;
        fs::write(&moved, GREP_SESSION)?;
        let variable = |name: &str| (name == "CLAUDE_CONFIG_DIR").then(|| config.clone().into());
        let mut meter = Meter::new(home.path(), &variable);
        for read in [&moved, &path] {
            let pass = meter
                .pass(read, (Agent::Claude, &Claude))
                .map_err(|unread| unread.to_string())?;
            assert_eq!(pass.turns.len(), 4, "{}", read.display());
        }
        let unmoved = Meter::new(home.path(), &|_| None).pass(&moved, (Agent::Claude, &Claude));
        assert!(matches!(unmoved, Err(Unread::Outside)));
        Ok(())
    }

    /// Unix only, where links are made without privileges: a link in the projects folder to a
    /// transcript inside it is refused rather than followed, and a folder there that links
    /// outside it leads outside.
    #[cfg(unix)]
    #[test]
    fn links_in_the_projects_folder_are_not_followed() -> TestResult {
        let (home, path) = home_with_transcript()?;
        let folder = projects(home.path());
        let link = folder.join("C--repo").join("link.jsonl");
        std::os::unix::fs::symlink(&path, &link)?;
        let mut meter = Meter::new(home.path(), &|_| None);
        assert!(matches!(
            meter.pass(&link, (Agent::Claude, &Claude)),
            Err(Unread::NotRegular)
        ));
        let elsewhere = home.path().join("elsewhere");
        fs::create_dir_all(&elsewhere)?;
        fs::write(elsewhere.join("s1.jsonl"), GREP_SESSION)?;
        std::os::unix::fs::symlink(&elsewhere, folder.join("linked"))?;
        let through = folder.join("linked").join("s1.jsonl");
        assert!(matches!(
            meter.pass(&through, (Agent::Claude, &Claude)),
            Err(Unread::Outside)
        ));
        Ok(())
    }

    /// A transcript that is not read is noted for the daemon's status in words that name no path,
    /// so transcripts refused alike are one error counted twice; no batch is counted as failed.
    #[test]
    fn transcripts_refused_alike_are_one_error() -> TestResult {
        let home = tempfile::tempdir()?;
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let inbox = Inbox::default();
        let mut meter = Meter::new(home.path(), &|_| None);
        let elsewhere = home.path().join("elsewhere");
        fs::create_dir_all(projects(home.path()))?;
        fs::create_dir_all(&elsewhere)?;
        let mut refused = Vec::new();
        for name in ["a.jsonl", "b.jsonl"] {
            fs::write(elsewhere.join(name), GREP_SESSION)?;
            refused.push(transcript("s1", &elsewhere.join(name))?);
        }
        meter.wait(refused);
        meter.read(&mut writer, &inbox, |_| Vec::new());
        let errors: Vec<(String, u64)> = inbox
            .recent_errors()
            .into_iter()
            .map(|error| (error.message, error.count))
            .collect();
        let message = "a transcript is not read: it is not in its agent's transcripts folder";
        assert_eq!(errors, [(message.to_owned(), 2)]);
        assert_eq!(inbox.counts().failed_batches, 0);
        Ok(())
    }

    /// Past MAX_TRANSCRIPTS_WAITING the oldest transcripts waiting are let go.
    #[test]
    fn only_so_many_transcripts_wait() -> TestResult {
        let mut meter = Meter::new(Path::new("no-home"), &|_| None);
        let named: Vec<Transcript> = (0..MAX_TRANSCRIPTS_WAITING + 3)
            .map(|number| transcript("s1", Path::new(&format!("/t/{number}.jsonl"))))
            .collect::<Result<_, _>>()?;
        meter.wait(named);
        assert_eq!(meter.waiting.len(), MAX_TRANSCRIPTS_WAITING);
        assert_eq!(
            meter.waiting.first().map(|kept| kept.path.clone()),
            Some(PathBuf::from("/t/3.jsonl"))
        );
        Ok(())
    }

    /// Past MAX_TAILS the transcript read longest ago is forgotten: with every place taken, the
    /// first transcript read again, then one more read, it is the second that is forgotten.
    #[test]
    fn the_transcript_read_longest_ago_is_forgotten() -> TestResult {
        let home = tempfile::tempdir()?;
        let project = projects(home.path()).join("C--repo");
        fs::create_dir_all(&project)?;
        let mut meter = Meter::new(home.path(), &|_| None);
        let mut read = |path: &Path| -> Result<PathBuf, Box<dyn std::error::Error>> {
            let pass = meter
                .pass(path, (Agent::Claude, &Claude))
                .map_err(|unread| unread.to_string())?;
            Ok(pass.kept_as)
        };
        let mut kept_as = Vec::new();
        for number in 0..=MAX_TAILS {
            let path = project.join(format!("{number}.jsonl"));
            fs::write(&path, "")?;
            if number < MAX_TAILS {
                kept_as.push(read(&path)?);
            }
        }
        read(&project.join("0.jsonl"))?;
        read(&project.join(format!("{MAX_TAILS}.jsonl")))?;
        assert_eq!(meter.tails.len(), MAX_TAILS);
        assert_eq!(
            kept_as
                .iter()
                .take(3)
                .map(|path| meter.tails.contains_key(path))
                .collect::<Vec<_>>(),
            [true, false, true]
        );
        Ok(())
    }

    /// A path is placed by its text before the disk is asked: one that steps through `..` is
    /// refused even when it ends inside the folder, and on Windows a network path is refused as
    /// outside without being looked up, while a path that differs from the folder's only in case
    /// is inside.
    #[test]
    fn a_path_is_placed_by_its_text_first() -> TestResult {
        let (home, path) = home_with_transcript()?;
        let folder = projects(home.path());
        let stepped = folder
            .join("C--repo")
            .join("..")
            .join("C--repo")
            .join("s1.jsonl");
        assert!(fs::metadata(&stepped).is_ok());
        assert!(matches!(
            transcript_in(&stepped, std::slice::from_ref(&folder)),
            Err(Unread::Outside)
        ));
        assert!(transcript_in(&path, std::slice::from_ref(&folder)).is_ok());
        #[cfg(windows)]
        {
            let network = Path::new(r"\\nohost.invalid\share\s1.jsonl");
            assert!(matches!(
                transcript_in(network, std::slice::from_ref(&folder)),
                Err(Unread::Outside)
            ));
            let device = Path::new(r"\\?\C:\repo\s1.jsonl");
            assert!(!is_inside_by_text(device, Path::new(r"C:\repo")));
            let parent = path.parent().ok_or("a folder")?;
            let name = path.file_name().ok_or("a name")?;
            let upper = PathBuf::from(parent.to_string_lossy().to_uppercase()).join(name);
            assert!(transcript_in(&upper, std::slice::from_ref(&folder)).is_ok());
        }
        Ok(())
    }
}
