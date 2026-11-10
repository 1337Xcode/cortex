//! Advisory claims, as `cortex claims` makes, lists and releases them. A claim is a position of
//! mode claim on a file, held by the session at work when it is made (the session whose latest
//! event came last within the five minutes before, as for an agent's note): neither a command nor
//! an MCP call carries the agent's session. It holds for `coordination.claim_minutes` and, while
//! its session is live, makes other sessions' edits of the file a concern, as any position does.
//! Claims inform; anyone may release any of them.

use cortex_coord::{Position, Timing};
use cortex_core::ipc::{ClaimLine, ClaimsAnswer};
use cortex_core::{Identifier, PositionMode, RepoPath};
use cortex_store::{NodeId, PositionRecord, PositionRow, Queries, StoreError, WriteTx};

use crate::asked::Refused;

/// Claims the files at `at` at `now_ms` in `transaction` for `at_work`, the session at work: a
/// claim takes the place of the session's read or edit of the file, and a claim made again while
/// the first holds keeps when the first was made, as `timing` says.
///
/// # Errors
///
/// A refusal in words when no session is at work or a path is not a file Cortex indexes; the
/// store's errors.
pub(crate) fn claimed(
    transaction: &mut WriteTx<'_>,
    at: &[String],
    (now_ms, at_work, timing): (i64, Option<&Identifier>, &Timing),
) -> Result<ClaimsAnswer, Refused> {
    let Some(session) = at_work else {
        return Err(Refused::Named(
            "no agent session is at work in this repository: a claim is held by the session \
             that makes it, so make it from the agent's own shell"
                .to_owned(),
        ));
    };
    if at.is_empty() {
        return Err(Refused::Named("name the files to claim".to_owned()));
    }
    let nodes = files_at(transaction.queries(), at)?;
    // A claim is made from a shell, which names no working tree: it is taken where the session's
    // latest position was.
    let latest = transaction
        .queries()
        .positions_since(0)?
        .into_iter()
        .filter(|row| row.record.session_id == *session)
        .max_by_key(|row| row.record.last_ms);
    let (worktree, branch) =
        latest.map_or((None, None), |row| (row.record.worktree, row.record.branch));
    for node in &nodes {
        let kept = transaction.queries().position_of(session, *node)?;
        let since_ms = kept
            .filter(|kept| {
                kept.record.mode == PositionMode::Claim && rule_of(kept).holds_at(now_ms, timing)
            })
            .map_or(now_ms, |kept| kept.record.since_ms);
        transaction.keep_position(&PositionRecord {
            session_id: session.clone(),
            node: *node,
            mode: PositionMode::Claim,
            since_ms,
            last_ms: now_ms,
            worktree: worktree.clone(),
            branch: branch.clone(),
        })?;
    }
    let seen = transaction
        .queries()
        .sessions_seen(std::slice::from_ref(session))?;
    let Some(seen) = seen.first() else {
        return Err(Refused::Named(
            "the session at work is not stored yet: try again in a moment".to_owned(),
        ));
    };
    Ok(ClaimsAnswer::Added {
        agent: seen.agent.title().to_owned(),
        session: session.as_str().to_owned(),
        files: u32::try_from(nodes.len()).unwrap_or(u32::MAX),
    })
}

/// Releases in `transaction` the claims holding at `now_ms`, as `timing` says, on the files at
/// `at`, whoever holds them, or on every file when `at` is empty.
///
/// # Errors
///
/// A refusal in words when a path is not a file Cortex indexes; the store's errors.
pub(crate) fn released(
    transaction: &mut WriteTx<'_>,
    at: &[String],
    (now_ms, timing): (i64, &Timing),
) -> Result<ClaimsAnswer, Refused> {
    // A claim holds while `now` is before its last time plus the hold: taken after this.
    let since_ms = now_ms.saturating_sub(timing.claim_ms).saturating_add(1);
    let count = if at.is_empty() {
        transaction.release_claims(None, since_ms)?
    } else {
        let nodes = files_at(transaction.queries(), at)?;
        transaction.release_claims(Some(&nodes), since_ms)?
    };
    Ok(ClaimsAnswer::Released { count })
}

/// The claims holding at `now_ms` under `timing`, by file, then by session.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn listed(
    queries: Queries<'_>,
    now_ms: i64,
    timing: &Timing,
) -> Result<Vec<ClaimLine>, StoreError> {
    let rows = queries.positions_since(now_ms.saturating_sub(timing.claim_ms))?;
    let held: Vec<_> = rows
        .into_iter()
        .filter(|row| row.record.mode == PositionMode::Claim)
        .filter(|row| rule_of(row).holds_at(now_ms, timing))
        .collect();
    let sessions: Vec<Identifier> = held
        .iter()
        .map(|row| row.record.session_id.clone())
        .collect();
    let seen = queries.sessions_seen(&sessions)?;
    let mut lines = Vec::new();
    for row in held {
        let Some((_, path)) = queries.node_file(row.record.node)? else {
            continue;
        };
        let Some(holder) = seen
            .iter()
            .find(|seen| seen.session_id == row.record.session_id)
        else {
            continue;
        };
        lines.push(ClaimLine {
            path: path.as_str().to_owned(),
            agent: holder.agent.title().to_owned(),
            session: row.record.session_id.as_str().to_owned(),
            since_ms: row.record.since_ms,
        });
    }
    lines.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.session.cmp(&b.session)));
    Ok(lines)
}

/// `row` as the coordination rules read a position.
fn rule_of(row: &PositionRow) -> Position<&Identifier, NodeId> {
    Position {
        session: &row.record.session_id,
        node: row.record.node,
        mode: row.record.mode,
        last_ms: row.record.last_ms,
        task_ended_ms: row.task_ended_ms,
    }
}

/// The file nodes of the repository paths `at`, in order, each once.
///
/// # Errors
///
/// A refusal in words for a path that is not a file Cortex indexes; the store's errors.
fn files_at(queries: Queries<'_>, at: &[String]) -> Result<Vec<NodeId>, Refused> {
    let mut nodes: Vec<NodeId> = Vec::new();
    for text in at {
        let node = match RepoPath::new(text) {
            Ok(path) if !path.is_root() => queries.file_node_at(&path)?,
            _ => None,
        };
        let Some(node) = node else {
            return Err(Refused::Named(format!(
                "{text} is not a file Cortex indexes"
            )));
        };
        if !nodes.contains(&node) {
            nodes.push(node);
        }
    }
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use cortex_core::{Agent, CoordinationConfig};
    use cortex_store::{SessionRecord, Writer};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// A minute in milliseconds.
    const MINUTE_MS: i64 = 60_000;

    /// Stores `a.py`, `b.py` and `c.py`, and sessions `s1` (Claude Code) and `s2` (Codex).
    fn store(root: &std::path::Path) -> TestResult<(Writer, [NodeId; 3])> {
        let mut writer = writer(root)?;
        let nodes = [
            add_file(&mut writer, "a.py", 1)?,
            add_file(&mut writer, "b.py", 1)?,
            add_file(&mut writer, "c.py", 1)?,
        ];
        let mut transaction = writer.transaction()?;
        for (session, agent) in [("s1", Agent::Claude), ("s2", Agent::Codex)] {
            transaction.record_session(&SessionRecord {
                session_id: Identifier::new(session)?,
                agent,
                agent_version: None,
                model: None,
                parent_session_id: None,
                cwd: None,
                transcript_path: None,
                started_at_ms: 1,
            })?;
        }
        transaction.commit()?;
        Ok((writer, nodes))
    }

    /// Worked by hand, now being minute 200 and a claim holding 120 minutes: `s1` claimed `a.py`
    /// at 150 and `c.py` at 60, which no longer holds; `s2` claimed `a.py` at 190 and read `b.py`
    /// at 195. The list: `a.py` by `s1` since 150, then by `s2` since 190; `c.py`'s claim and the
    /// read are left out. Releasing every claim holding removes the two on `a.py`, not the old one.
    #[test]
    fn the_claims_holding_are_listed_and_released() -> TestResult {
        let root = tempfile::tempdir()?;
        let (mut writer, [a, b, c]) = store(root.path())?;
        let timing = Timing::of(&CoordinationConfig::default());
        let mut transaction = writer.transaction()?;
        for (session, node, mode, minute) in [
            ("s1", a, PositionMode::Claim, 150),
            ("s1", c, PositionMode::Claim, 60),
            ("s2", a, PositionMode::Claim, 190),
            ("s2", b, PositionMode::Read, 195),
        ] {
            transaction.keep_position(&PositionRecord {
                session_id: Identifier::new(session)?,
                node,
                mode,
                since_ms: minute * MINUTE_MS,
                last_ms: minute * MINUTE_MS,
                worktree: None,
                branch: None,
            })?;
        }
        let now_ms = 200 * MINUTE_MS;
        let line = |session: &str, agent: &str, minute: i64| ClaimLine {
            path: "a.py".to_owned(),
            agent: agent.to_owned(),
            session: session.to_owned(),
            since_ms: minute * MINUTE_MS,
        };
        assert_eq!(
            listed(transaction.queries(), now_ms, &timing)?,
            [line("s1", "Claude Code", 150), line("s2", "Codex", 190)]
        );
        let released = released(&mut transaction, &[], (now_ms, &timing));
        assert!(matches!(released, Ok(ClaimsAnswer::Released { count: 2 })));
        assert_eq!(listed(transaction.queries(), now_ms, &timing)?, []);
        Ok(())
    }

    /// With no session at work a claim is refused, and an unknown file too; claimed by `s1` at
    /// minute 10 and again at 20, the claim keeps its first time, 10, and is now as of 20.
    #[test]
    fn a_claim_is_held_by_the_session_at_work_from_its_first_time() -> TestResult {
        let root = tempfile::tempdir()?;
        let (mut writer, [a, _, _]) = store(root.path())?;
        let timing = Timing::of(&CoordinationConfig::default());
        let s1 = Identifier::new("s1")?;
        let mut transaction = writer.transaction()?;
        let at = ["a.py".to_owned()];
        let words = |refused: Result<ClaimsAnswer, Refused>| match refused {
            Err(Refused::Named(words)) => words,
            other => format!("{other:?}"),
        };
        assert_eq!(
            words(claimed(&mut transaction, &at, (1, None, &timing))),
            "no agent session is at work in this repository: a claim is held by the session that \
             makes it, so make it from the agent's own shell"
        );
        assert_eq!(
            words(claimed(
                &mut transaction,
                &["z.py".to_owned()],
                (1, Some(&s1), &timing)
            )),
            "z.py is not a file Cortex indexes"
        );
        for minute in [10, 20] {
            let made = claimed(
                &mut transaction,
                &at,
                (minute * MINUTE_MS, Some(&s1), &timing),
            );
            assert!(matches!(made, Ok(ClaimsAnswer::Added { files: 1, .. })));
        }
        let kept = transaction
            .queries()
            .position_of(&s1, a)?
            .ok_or("a claim")?;
        assert_eq!(
            (kept.record.mode, kept.record.since_ms, kept.record.last_ms),
            (PositionMode::Claim, 10 * MINUTE_MS, 20 * MINUTE_MS)
        );
        Ok(())
    }
}
