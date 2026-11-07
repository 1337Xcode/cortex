//! The session primer: what an agent is told when a session starts. The repository's mode and
//! size, its landmark files, and one line on what Cortex adds from then on.
//!
//! The primer stays byte for byte the same from one session to the next, so it keeps an agent's
//! cached prompt warm: it holds nothing of the session or the time, its sizes are rounded to two
//! significant digits so that small growth changes nothing, and its landmarks, at most eight, are
//! listed by path. It changes only when the mode, a rounded size or the landmarks do.

use std::fmt::Write as _;

use cortex_core::{FileFlags, InterestTier, NodeKind, ScaleMode, estimate_tokens};
use cortex_store::{MetaKey, NodeId, Queries, StoreError};

use crate::graph::Graph;
use cortex_core::is_plain;

/// Most landmarks the primer names, the most interesting first.
pub const MAX_PRIMER_LANDMARKS: usize = 8;

/// Most tokens the primer may take, as estimated.
pub const MAX_PRIMER_TOKENS: usize = 400;

/// The primer for the repository the store `queries` reads holds, its landmarks from `graph`;
/// none while the store has no mode yet, the first index not done.
///
/// # Errors
///
/// The store's errors.
pub fn primer(queries: Queries<'_>, graph: &Graph<NodeId>) -> Result<Option<String>, StoreError> {
    let Some(mode) = queries
        .meta(MetaKey::ScaleMode)?
        .and_then(|name| ScaleMode::from_name(&name))
    else {
        return Ok(None);
    };
    let (files, lines) = queries.source_size(FileFlags::GENERATED.with(FileFlags::VENDORED))?;
    let mut text = String::new();
    let _ = writeln!(
        text,
        "[cortex] Project map · {} mode · about {}, {}",
        mode.name(),
        counted(files, ("source file", "source files")),
        counted(lines, ("line", "lines")),
    );
    let landmarks = landmarks(queries, graph)?;
    if !landmarks.is_empty() {
        let _ = writeln!(text, "Landmarks: {}", landmarks.join(", "));
    }
    text.push_str(match mode {
        ScaleMode::Tiny => {
            "Cortex records what agents do here; with this little code it adds nothing to prompts.\n"
        }
        _ => "Cortex adds the code a prompt points at before the agent starts, when the evidence agrees.\n",
    });
    // INVARIANT: budgeted (the primer is held to its cap: a primer past it is not given)
    Ok((estimate_tokens(&text) <= MAX_PRIMER_TOKENS).then_some(text))
}

/// The paths of the landmark files, at most [`MAX_PRIMER_LANDMARKS`] of the most interesting, by
/// path; a path that is not plain (a line break, a control or hidden character, an agent's reminder
/// tag) is not named, since the agent would read more than a name, or a name that is not the file's.
fn landmarks(queries: Queries<'_>, graph: &Graph<NodeId>) -> Result<Vec<String>, StoreError> {
    let mut chosen: Vec<(u32, f64)> = (0..graph.len())
        .filter_map(|at| u32::try_from(at).ok())
        .filter(|node| graph.kind(*node) == Some(NodeKind::File))
        .filter_map(|node| {
            let standing = graph.standing(node)?;
            (standing.tier == InterestTier::Landmark).then_some((node, standing.interest))
        })
        .collect();
    chosen.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    chosen.truncate(MAX_PRIMER_LANDMARKS);
    let ids: Vec<NodeId> = chosen
        .iter()
        .filter_map(|(node, _)| graph.id(*node))
        .collect();
    let mut paths: Vec<String> = queries
        .files_of_nodes(&ids)?
        .into_iter()
        .map(|(_, row)| row.file.path.as_str().to_owned())
        // INVARIANT: true-at-render. A path that is not plain is not named: a crafted file name
        // cannot add a line, a hidden character or a tag to every session's start.
        .filter(|path| is_plain(path))
        .collect();
    paths.sort();
    Ok(paths)
}

/// `count`, rounded and grouped, before the singular or plural of what it counts.
fn counted(count: u64, (one, many): (&str, &str)) -> String {
    let rounded = rounded(count);
    format!(
        "{} {}",
        grouped(rounded),
        if rounded == 1 { one } else { many }
    )
}

/// `count` rounded to two significant digits, half up: 1,234 is 1,200 and 1,250 is 1,300; below
/// 100, the count itself.
fn rounded(count: u64) -> u64 {
    let digits = count.checked_ilog10().map_or(1, |log| log + 1);
    if digits <= 2 {
        return count;
    }
    let step = 10_u64.pow(digits - 2);
    count.saturating_add(step / 2) / step * step
}

/// `count` written with a comma between each group of three digits: 150000 is 150,000.
fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut text = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts rounded to two significant digits, half up, as worked out by hand: below 100 a count
    /// is itself; 1,234 is 1,200; 1,250 is 1,300; 150,321 is 150,000; 999,999 is 1,000,000.
    #[test]
    fn counts_are_rounded_to_two_significant_digits() {
        let cases = [
            (0, 0),
            (7, 7),
            (99, 99),
            (100, 100),
            (1_234, 1_200),
            (1_250, 1_300),
            (150_321, 150_000),
            (999_999, 1_000_000),
        ];
        for (count, expected) in cases {
            assert_eq!(rounded(count), expected, "{count}");
        }
    }

    /// Counts are written with a comma between groups of three digits.
    #[test]
    fn counts_are_grouped_by_thousands() {
        let cases = [
            (0, "0"),
            (999, "999"),
            (1_000, "1,000"),
            (150_000, "150,000"),
            (1_234_567, "1,234,567"),
        ];
        for (count, expected) in cases {
            assert_eq!(grouped(count), expected, "{count}");
        }
    }
}
