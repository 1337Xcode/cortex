//! The handoff a session's first prompt is given: the most recent open one, left within a day by a
//! session outside its own family, that the prompt concerns. A prompt concerns a handoff when one
//! of the files its retrieval weighed (the three best, known whether or not the gate opened) is a
//! file the stopped session edited or read, or when it asks to go on (continue, resume, pick up,
//! carry on, finish). The handoff says what was left, and comes ahead of any work order.

use cortex_coord::{Handed, handoff_text};
use cortex_core::{Event, Identifier, RepoPath, Ulid};
use cortex_store::{HandoffRow, NodeId, Queries, StoreError};

use super::coordination::family_of;

/// How long a handoff is offered after its session stopped: a day, in milliseconds.
const OFFERED_FOR_MS: i64 = 24 * 60 * 60 * 1_000;

/// Words that ask to go on with earlier work, matched as whole words in any case.
const GO_ON_WORDS: [&str; 5] = ["continue", "resume", "pick up", "carry on", "finish"];

/// The handoff the prompt `prompt` of `event`, starting the task `task_id`, is given, its retrieval
/// having weighed `weighed`: the session that left it and its text. None unless this is the
/// session's first prompt and an open handoff applies. The prompt's own task does not count: the
/// event is queued before it is answered, so the writer may already have stored it.
///
/// # Errors
///
/// The store's errors.
pub(super) fn handoff_for(
    queries: Queries<'_>,
    (event, prompt, task_id): (&Event, &str, Ulid),
    weighed: &[NodeId],
) -> Result<Option<(Identifier, String)>, StoreError> {
    let tasks = queries.tasks_of_session(&event.session_id)?;
    if tasks.iter().any(|task| task.record.task_id != task_id) {
        return Ok(None);
    }
    let open = queries.open_handoffs_since(event.ts_ms.saturating_sub(OFFERED_FOR_MS))?;
    if open.is_empty() {
        return Ok(None);
    }
    let mut sessions: Vec<Identifier> = open
        .iter()
        .map(|row| row.record.from_session.clone())
        .chain([event.session_id.clone()])
        .collect();
    sessions.sort_unstable();
    sessions.dedup();
    let seen = queries.sessions_seen(&sessions)?;
    let family = family_of(event, &seen);
    let goes_on = asks_to_go_on(prompt);
    for row in &open {
        let record = &row.record;
        let is_concerned = concerns(weighed, &record.edited, &record.read);
        if family.contains(&record.from_session) || !(goes_on || is_concerned) {
            continue;
        }
        let Some(agent) = seen
            .iter()
            .find(|seen| seen.session_id == record.from_session)
            .map(|seen| seen.agent.title())
        else {
            continue;
        };
        if let Some(text) = text_of(queries, row, (agent, event.ts_ms))? {
            return Ok(Some((record.from_session.clone(), text)));
        }
    }
    Ok(None)
}

/// The text of `row`, left by a session of `agent`, as of `now_ms`: its files by the paths the
/// store holds now (a file gone is left out). None when nothing is left to say.
fn text_of(
    queries: Queries<'_>,
    row: &HandoffRow,
    (agent, now_ms): (&str, i64),
) -> Result<Option<String>, StoreError> {
    let paths = |nodes: &[NodeId]| -> Result<Vec<RepoPath>, StoreError> {
        let mut paths = Vec::new();
        for node in nodes {
            if let Some((_, path)) = queries.node_file(*node)? {
                paths.push(path);
            }
        }
        Ok(paths)
    };
    let (edited, read) = (paths(&row.record.edited)?, paths(&row.record.read)?);
    let edited: Vec<&str> = edited.iter().map(RepoPath::as_str).collect();
    let read: Vec<&str> = read.iter().map(RepoPath::as_str).collect();
    let failure = row.record.last_failure.as_ref().map(|failure| {
        (
            failure.runner.as_str(),
            failure.kind.name(),
            failure.exit_code,
        )
    });
    Ok(handoff_text(&Handed {
        agent,
        ago_ms: now_ms.saturating_sub(row.record.created_at_ms),
        edited: &edited,
        read: &read,
        failure,
        open_warnings: row.record.open_warnings.len(),
    }))
}

/// Whether a prompt whose retrieval weighed `weighed` concerns a handoff naming `edited` and `read`:
/// one of the files weighed is one of them.
fn concerns<T: PartialEq>(weighed: &[T], edited: &[T], read: &[T]) -> bool {
    weighed
        .iter()
        .any(|node| edited.contains(node) || read.contains(node))
}

/// Whether `prompt` asks to go on with earlier work: one of [`GO_ON_WORDS`] as whole words.
fn asks_to_go_on(prompt: &str) -> bool {
    let words: Vec<String> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    GO_ON_WORDS.iter().any(|phrase| {
        let phrase: Vec<&str> = phrase.split(' ').collect();
        words
            .windows(phrase.len())
            .any(|window| window.iter().zip(&phrase).all(|(word, part)| word == part))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A handoff that edited 1 and read 2: a prompt weighing 2 or 1 concerns it, one weighing only
    /// 3 and 4, or nothing, does not.
    #[test]
    fn a_prompt_concerns_a_handoff_by_a_file_it_weighed() {
        let (edited, read) = ([1], [2]);
        assert!(concerns(&[4, 2], &edited, &read));
        assert!(concerns(&[1], &edited, &read));
        assert!(!concerns(&[3, 4], &edited, &read));
        assert!(!concerns(&[], &edited, &read));
    }

    /// The words that ask to go on match whole, in any case, across punctuation; a word that only
    /// holds one does not.
    #[test]
    fn a_prompt_asks_to_go_on_in_whole_words() {
        for prompt in [
            "Continue.",
            "please RESUME the change",
            "pick up where it stopped",
            "carry on with divide",
            "finish the refactor",
        ] {
            assert!(asks_to_go_on(prompt), "{prompt}");
        }
        for prompt in [
            "continued fraction",
            "the finished parts",
            "pick the upper one",
            "",
        ] {
            assert!(!asks_to_go_on(prompt), "{prompt}");
        }
    }
}
