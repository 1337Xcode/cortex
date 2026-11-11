//! Why code is where it is, as `cortex why` and the `cortex_why` tool tell it: the files its file is
//! linked to by code, by history and by use in past tasks, the strongest of each with its strength
//! as the engine weighs it (a link too weak to count as support left out);
//! the project facts about it that are true now, those on the symbol asked of first; the sessions
//! working on its file now; and the warnings about its file whose tasks have not ended. A target
//! names a file or a symbol (see [`target`]); for a symbol, the links, sessions and warnings are
//! its file's. Nothing is guessed: a target that names nothing or several things is refused, a fact
//! whose code changed is withheld as in a work order, and a name that is not plain text is left
//! out. The answer is the same for the same store, files and time.

mod target;

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use cortex_coord::{Position, PositionMode, Timing, ago, is_live};
use cortex_core::{EdgeType, Identifier, WarningKind, estimate_tokens, is_plain};
use cortex_learn::{MAX_LINK_WEIGHT, decayed};
use cortex_store::{FileLink, NodeId, Queries, StoreError, WarningRecord};

use crate::facts::{ListedFile, fact_line, order_facts, true_now};
use crate::open_warnings::plain_path;
use crate::support::{COACCESS_FLOOR, STRUCTURAL_FLOOR};
use target::Named;

/// Most tokens an answer takes: what an MCP tool's answer may hold. Past it, the last lines go,
/// warnings first, then sessions, facts and links.
pub const MAX_WHY_TOKENS: usize = 1_200;

/// Most files an answer names for each kind of link: the strongest.
pub const MAX_LINKS_PER_KIND: usize = 5;

/// Most sessions an answer names: the ones heard from last.
pub const MAX_WHY_SESSIONS: usize = 5;

/// Most warnings an answer names: the latest.
pub const MAX_WHY_WARNINGS: usize = 5;

/// The kinds of link an answer names, each under its heading, in this order.
const LINK_HEADINGS: [(EdgeType, &str); 3] = [
    (EdgeType::Structural, "Linked by code:"),
    (EdgeType::CoChange, "Changed together in history:"),
    (EdgeType::CoAccess, "Used together in past tasks:"),
];

/// When and where an answer is given.
#[derive(Debug, Clone, Copy)]
pub struct WhyAt<'w> {
    /// Now, in UTC milliseconds: which positions hold, and how long ago things were.
    pub now_ms: i64,
    /// How long positions hold and sessions count as live.
    pub timing: &'w Timing,
    /// The repository's root folder, where files are read to check what is said of them.
    pub root: &'w Path,
    /// The largest file read.
    pub max_file_bytes: u64,
    /// `learning.link_decay`: what a co-access link keeps of its weight for each task.
    pub link_decay: f64,
}

/// An answer, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The answer.
    Answer(String),
    /// Why there is none, in words: the target names nothing or several things, or a line that
    /// cannot be known now.
    Refused(String),
}

/// One part of an answer: its heading and its lines.
struct Part {
    /// The heading.
    heading: &'static str,
    /// The lines, in order.
    lines: Vec<String>,
}

/// Why `target` is where it is, from the store `queries` reads, `at` a time and a repository.
///
/// # Errors
///
/// The store's errors.
pub fn why(queries: Queries<'_>, target: &str, at: &WhyAt<'_>) -> Result<Why, StoreError> {
    let answer = match target::named(queries, target, (at.root, at.max_file_bytes))? {
        Ok(named) if is_shown_as_it_is(&named) => {
            let mut parts = links(queries, named.file, at.link_decay)?;
            parts.push(facts(queries, &named, at)?);
            parts.push(sessions(queries, named.file, at)?);
            parts.push(warnings(queries, named.file, at.now_ms)?);
            Why::Answer(written(&heading_of(&named), parts))
        }
        Ok(_) => Why::Refused(NOT_PLAIN.to_owned()),
        Err(refused) => Why::Refused(refused),
    };
    // INVARIANT: budgeted (no answer or refusal over its cap reaches the agent, whatever the
    // target or the names it echoes)
    let text = match &answer {
        Why::Answer(text) | Why::Refused(text) => text,
    };
    if estimate_tokens(text) > MAX_WHY_TOKENS {
        return Ok(Why::Refused(TOO_LONG.to_owned()));
    }
    Ok(answer)
}

/// The refusal of a target whose file or definition has a name not shown as it is.
const NOT_PLAIN: &str = "the file or definition it names has a name Cortex does not show as it is";

/// The refusal of an answer that would be over its cap even shed of every line, or that echoes a
/// target too long.
const TOO_LONG: &str = "the answer would be longer than Cortex gives: name the target by a \
                        shorter path or name";

/// Whether the names `named` puts in the answer's first line are plain text: a symbol's name
/// comes from the repository, and so does a file's path found by a symbol's name.
fn is_shown_as_it_is(named: &Named) -> bool {
    is_plain(named.path.as_str())
        && named
            .symbol
            .as_ref()
            .is_none_or(|(_, symbol)| is_plain(symbol))
}

/// The first line: what the answer is about; for a symbol, that what follows is its file's, its
/// own facts first; for a line no definition holds, that what follows is the file's.
fn heading_of(named: &Named) -> String {
    let path = named.path.as_str();
    match (&named.symbol, named.outside_line) {
        (Some((_, symbol)), _) => {
            format!(
                "[cortex] Why · {path}:{symbol} · what is known of its file, its own facts first\n"
            )
        }
        (None, Some(line)) => {
            format!(
                "[cortex] Why · {path} · line {line} is in no definition, so this is the file\n"
            )
        }
        (None, None) => format!("[cortex] Why · {path}\n"),
    }
}

/// The parts naming the files `file` is linked to, by kind, the strongest now first, then by
/// path; a co-access link fades by `link_decay` for each task since its last.
fn links(queries: Queries<'_>, file: NodeId, link_decay: f64) -> Result<Vec<Part>, StoreError> {
    let latest_seq = queries.latest_task_seq()?;
    let linked = queries.links_of_file(file)?;
    Ok(LINK_HEADINGS
        .iter()
        .map(|(kind, heading)| {
            // A file's links of one kind, kept apart by the task each was last changed at, are
            // weighed one by one before the strongest is taken: one faded more may be weaker now.
            let mut by_file: BTreeMap<&str, f64> = BTreeMap::new();
            for link in linked
                .iter()
                .filter(|link| link.edge_type == *kind && is_plain(link.path.as_str()))
            {
                let Some(now) = strength(link, (latest_seq, link_decay)) else {
                    continue;
                };
                let kept = by_file.entry(link.path.as_str()).or_insert(now);
                *kept = kept.max(now);
            }
            let mut strongest: Vec<(f64, &str)> =
                by_file.into_iter().map(|(path, now)| (now, path)).collect();
            strongest.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
            Part {
                heading,
                lines: strongest
                    .into_iter()
                    .take(MAX_LINKS_PER_KIND)
                    .map(|(now, path)| format!("  - {path} ({now:.2})"))
                    .collect(),
            }
        })
        .collect())
}

/// How strong `link` is at task `latest_seq`, from 0 to 1, as the engine weighs it; none when it
/// is too weak to count as support. A reference must be tied as surely as [`STRUCTURAL_FLOOR`];
/// a co-access link fades by `link_decay` for each task since its last, must keep
/// [`COACCESS_FLOOR`], and is said out of the most a link holds; a co-change link is its
/// confidence.
fn strength(link: &FileLink, (latest_seq, link_decay): (u64, f64)) -> Option<f64> {
    match link.edge_type {
        EdgeType::Structural => (link.weight >= STRUCTURAL_FLOOR).then_some(link.weight),
        EdgeType::CoAccess => {
            let faded = decayed(link.weight, link.last_seq, latest_seq, link_decay);
            (faded >= COACCESS_FLOOR).then(|| (faded / MAX_LINK_WEIGHT).clamp(0.0, 1.0))
        }
        _ => Some(link.weight),
    }
}

/// The part naming the active facts about `named` true now, those on its symbol first.
fn facts(queries: Queries<'_>, named: &Named, at: &WhyAt<'_>) -> Result<Part, StoreError> {
    let listed = ListedFile {
        path: named.path.clone(),
        node: named.file,
        shown: named.symbol.iter().map(|(node, _)| *node).collect(),
    };
    let facts = order_facts(queries, &[listed])?;
    let kept = true_now(&facts, at.root, at.max_file_bytes);
    Ok(Part {
        heading: "Project facts:",
        lines: kept
            .iter()
            .filter_map(|position| facts.get(*position))
            .map(|fact| format!("  - {}", fact_line(fact)))
            .collect(),
    })
}

/// The part naming the sessions holding a position on `file` now and heard from lately, the one
/// that acted last first.
fn sessions(queries: Queries<'_>, file: NodeId, at: &WhyAt<'_>) -> Result<Part, StoreError> {
    let since_ms = at.now_ms.saturating_sub(at.timing.longest_ms());
    let held: Vec<Position<Identifier, NodeId>> = queries
        .positions_since(since_ms)?
        .into_iter()
        .filter(|row| row.record.node == file)
        .map(|row| Position {
            session: row.record.session_id,
            node: row.record.node,
            mode: row.record.mode,
            last_ms: row.record.last_ms,
            task_ended_ms: row.task_ended_ms,
        })
        .filter(|position| position.holds_at(at.now_ms, at.timing))
        .collect();
    let ids: Vec<Identifier> = held
        .iter()
        .map(|position| position.session.clone())
        .collect();
    let seen = queries.sessions_seen(&ids)?;
    let mut here: Vec<(&Position<Identifier, NodeId>, &str)> = held
        .iter()
        .filter_map(|position| {
            let session = seen
                .iter()
                .find(|seen| seen.session_id == position.session)?;
            let heard = session.heard_from_ms()?;
            is_live(heard, at.now_ms, at.timing).then_some((position, session.agent.title()))
        })
        .collect();
    here.sort_by_key(|(position, _)| (Reverse(position.last_ms), position.session.clone()));
    Ok(Part {
        heading: "Sessions working on it now:",
        lines: here
            .into_iter()
            .take(MAX_WHY_SESSIONS)
            .map(|(position, agent)| {
                let did = match position.mode {
                    PositionMode::Edit => "edited it",
                    PositionMode::Read => "read it",
                    PositionMode::Claim => "claimed it",
                };
                // A session id holds no control or hidden character, and 8 characters are too few
                // to hold an agent's reminder tag: the part shown is plain text.
                let short: String = position.session.as_str().chars().take(8).collect();
                let when = ago(at.now_ms - position.last_ms);
                format!("  - {agent} session {short} {did} {when}")
            })
            .collect(),
    })
}

/// The part naming the warnings about `file` whose tasks have not ended, the latest first: one
/// for each kind, edited code and file warned of, given again or not.
fn warnings(queries: Queries<'_>, file: NodeId, now_ms: i64) -> Result<Part, StoreError> {
    let mut said: Vec<(WarningKind, NodeId, NodeId)> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for row in queries.open_warnings_about(file)? {
        let warning = row.record;
        let key = (warning.kind, warning.source, warning.target);
        if said.contains(&key) {
            continue;
        }
        said.push(key);
        lines.extend(warning_line(queries, &warning, now_ms)?);
        if lines.len() == MAX_WHY_WARNINGS {
            break;
        }
    }
    Ok(Part {
        heading: "Warnings whose tasks have not ended:",
        lines,
    })
}

/// `warning` as a line, `now_ms` saying how long ago it was given; none when a file or a symbol it
/// names cannot be named as it is, and none for an origin note.
fn warning_line(
    queries: Queries<'_>,
    warning: &WarningRecord,
    now_ms: i64,
) -> Result<Option<String>, StoreError> {
    let Some(target) = plain_path(queries, warning.target)? else {
        return Ok(None);
    };
    let when = ago(now_ms - warning.at_ms);
    let said = match warning.kind {
        WarningKind::CoChange => plain_path(queries, warning.source)?
            .map(|source| format!("usually changes with {source}, which was edited {when}")),
        WarningKind::Test => plain_path(queries, warning.source)?
            .map(|source| format!("holds the tests of {source}, which was edited {when}")),
        WarningKind::CallSite => queries
            .anchor_names(&[warning.source])?
            .into_values()
            .next()
            .filter(|name| is_plain(name))
            .map(|name| format!("calls {name}, whose signature changed {when}")),
        // An origin note names the file just edited and asks for nothing: no warning to list.
        WarningKind::Origin => None,
    };
    Ok(said.map(|said| format!("  - {target} {said}")))
}

/// The answer: `heading`, then each part with lines, at most [`MAX_WHY_TOKENS`], the last lines
/// shed first; a line saying nothing is known when no part has one.
fn written(heading: &str, mut parts: Vec<Part>) -> String {
    loop {
        let text = joined(heading, &parts);
        if estimate_tokens(&text) <= MAX_WHY_TOKENS {
            return text;
        }
        let Some(last) = parts.iter_mut().rev().find(|part| !part.lines.is_empty()) else {
            return text;
        };
        last.lines.pop();
    }
}

/// `heading` and each part of `parts` that has lines, under its heading.
fn joined(heading: &str, parts: &[Part]) -> String {
    let mut text = heading.to_owned();
    let mut is_empty = true;
    for part in parts.iter().filter(|part| !part.lines.is_empty()) {
        is_empty = false;
        let _ = writeln!(text, "{}", part.heading);
        for line in &part.lines {
            let _ = writeln!(text, "{line}");
        }
    }
    if is_empty {
        text.push_str("Nothing is known yet of its links, facts, sessions or warnings.\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The headings the tests' parts are written under.
    const HEADINGS: [&str; 4] = ["One:", "Two:", "Three:", "Four:"];

    /// Parts under [`HEADINGS`], each of `counts` lines of `length` bytes, `  - ` and `x`s.
    fn parts(counts: &[usize], length: usize) -> Vec<Part> {
        let line = format!("  - {}", "x".repeat(length - 4));
        HEADINGS
            .iter()
            .zip(counts)
            .map(|(heading, count)| Part {
                heading,
                lines: vec![line.clone(); *count],
            })
            .collect()
    }

    /// Worked by hand: four parts of five lines of 300 bytes (301 with the line break) under
    /// `H` (2 bytes) take 2 + 23 (the headings) + 20 * 301 = 6,045 bytes, 1,512 tokens. The last
    /// part's lines go first: four gone leave 4,841 bytes, 1,211 tokens, still over 1,200; the
    /// fifth and its heading gone leave 4,534 bytes, 1,134 tokens. The first three parts stay
    /// whole.
    #[test]
    fn the_last_lines_are_shed_until_the_answer_fits() {
        let line = format!("  - {}\n", "x".repeat(296));
        let three = format!(
            "H\nOne:\n{lines}Two:\n{lines}Three:\n{lines}",
            lines = line.repeat(5)
        );
        assert_eq!(written("H\n", parts(&[5, 5, 5, 5], 300)), three);
        assert_eq!(estimate_tokens(&three), 1_134);
    }

    proptest::proptest! {
        /// Whatever the parts hold, the answer is within its cap, and what is shed is the end:
        /// each part keeps its first lines, and a part that lost a line is followed only by parts
        /// that lost all of theirs.
        #[test]
        fn no_answer_is_over_its_cap(
            counts in proptest::collection::vec(0_usize..8, 4),
            length in 5_usize..900,
        ) {
            let text = written("H\n", parts(&counts, length));
            proptest::prop_assert!(estimate_tokens(&text) <= MAX_WHY_TOKENS);
            let line = format!("  - {}", "x".repeat(length - 4));
            let kept: Vec<usize> = HEADINGS
                .iter()
                .map(|heading| {
                    text.split_once(&format!("{heading}\n")).map_or(0, |(_, rest)| {
                        rest.lines().take_while(|kept| *kept == line).count()
                    })
                })
                .collect();
            for (at, (held, count)) in kept.iter().zip(&counts).enumerate() {
                proptest::prop_assert!(held <= count);
                if held < count {
                    let later = kept.get(at + 1..).unwrap_or_default();
                    proptest::prop_assert!(later.iter().all(|after| *after == 0));
                }
            }
        }
    }
}
