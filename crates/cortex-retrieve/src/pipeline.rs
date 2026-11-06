//! One retrieval, from a prompt to what the agent is told: the seeds, the spreading, the scores,
//! the gate, and, when the gate lets it through, the files listed and the work order written from
//! them as they are on disk. And the same engine without the gate, as a question asked of it
//! directly: the files it finds, and what the gate would have said.
//!
//! Each step that finds nothing ends a retrieval in silence and says why: in tiny mode nothing is
//! retrieved; a prompt and a session that point at nothing seed nothing; the gate can stay shut;
//! and a work order with no file it can truly list, none that fits its caps, or none of the three
//! best files the gate weighed, is not given. The caller decides whether a work order is injected,
//! and a held-out task gets none however strong.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use cortex_core::{GateFeatures, Identifier, RetrievalConfig, ScaleMode};
use cortex_store::{NodeId, Queries};

use crate::error::RetrieveError;
use crate::features::{Found, GATED_FILES, features_of};
use crate::gate::{Rule, Shut, Verdict, decide};
use crate::graph::Graph;
use crate::items::{Listed, Listing, listed};
use crate::prompt::PromptTerms;
use crate::push::{Activation, spread};
use crate::render::{Basis, Item, Limits, Rendered, WorkOrder, render};
use crate::score::{ScoredFile, named_first, scored_files};
use crate::seeds::{SeedQuery, Seeds, seeds};
use crate::support::{Evidence, Support};
use crate::two_stage::{Staged, two_stage};

/// The most tokens a work order holds in small mode, whatever the setting.
pub const SMALL_MODE_MAX_TOKENS: u32 = 400;

/// What one retrieval is asked.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The prompt's text, or the question asked.
    pub prompt: &'a str,
    /// The session it comes in; none for a question asked outside one.
    pub session: Option<&'a Identifier>,
    /// The file nodes of the uncommitted diff, as the daemon last read them; empty when it has not.
    pub uncommitted: &'a [NodeId],
    /// The repository's mode.
    pub mode: ScaleMode,
    /// The `retrieval.*` settings.
    pub settings: &'a RetrievalConfig,
    /// `index.max_file_bytes`: the most bytes a file may hold to be read.
    pub max_file_bytes: u64,
    /// The self-check's raise of the gate's bar.
    pub raise: f64,
    /// The share of the repository's latest delivered injections that were used, when known.
    pub recent_precision: Option<f64>,
    /// When spreading must stop, if it has not.
    pub deadline: Option<Instant>,
    /// The repository's root, where files are read from.
    pub root: &'a Path,
}

/// Why a retrieval gives nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Silence {
    /// Tiny mode: nothing is retrieved.
    Tiny,
    /// The prompt and the session point at nothing.
    NothingFound,
    /// The gate stayed shut.
    Gate(Shut),
    /// The gate opened, but no file could be listed with a true reason, or none fit the caps.
    NothingToShow,
}

/// What a retrieval came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A work order, and the rule that let it through.
    WorkOrder(Rule, Rendered),
    /// Nothing, and why.
    Silent(Silence),
}

/// One retrieval: what it came to, the gate's features and the files it weighed when it got that
/// far, and the files the work order lists.
#[derive(Debug, Clone, PartialEq)]
pub struct Retrieval {
    /// What it came to.
    pub outcome: Outcome,
    /// The gate's features, when the retrieval reached the gate.
    pub features: Option<GateFeatures>,
    /// The files the gate weighed, the three best, by node; none when it did not get that far.
    pub weighed: Vec<NodeId>,
    /// The files the work order lists, the best first, each with the spans whose lines it writes;
    /// none when it is silent.
    pub listed: Vec<Listed>,
}

/// What the engine found for a question, without the gate.
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    /// The gate's features and what the gate would have said; none when nothing was found.
    pub judged: Option<(GateFeatures, Verdict)>,
    /// The files found, the best first, each with the spans its answer writes and its reason; none
    /// when the spreading ran out of time.
    pub listed: Vec<Listed>,
    /// The answer as text: the files, their reasons and code as they are on disk, under a header
    /// saying what found them and what the gate would have said; the header alone, naming no file,
    /// when the spreading ran out of time; none when nothing was found or nothing could be shown.
    pub rendered: Option<Rendered>,
}

/// The retrieval `request` asks for, over the store `queries` reads and its `graph`.
///
/// # Errors
///
/// The store's errors.
pub fn retrieve(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    request: &Request<'_>,
) -> Result<Retrieval, RetrieveError> {
    let silent = |silence, features: Option<GateFeatures>, weighed: &[NodeId]| Retrieval {
        outcome: Outcome::Silent(silence),
        features,
        weighed: weighed.to_vec(),
        listed: Vec::new(),
    };
    // INVARIANT: silent-when-unsure (tiny mode retrieves nothing, and a retrieval with no seed
    // spreads nothing: each says so instead of guessing)
    if request.mode == ScaleMode::Tiny {
        return Ok(silent(Silence::Tiny, None, &[]));
    }
    let Some(reached) = reached(queries, graph, request)? else {
        return Ok(silent(Silence::NothingFound, None, &[]));
    };
    let (support, features, verdict) = reached.judged(graph, request);
    let weighed = reached.weighed(graph);
    let rule = match verdict {
        Verdict::Inject(rule) => rule,
        Verdict::Silent(shut) => {
            return Ok(silent(Silence::Gate(shut), Some(features), &weighed));
        }
    };
    let listing = Listing {
        queries,
        graph,
        seeds: &reached.seeds,
        support: &support,
    };
    let max_files = usize::try_from(request.settings.max_files).unwrap_or(usize::MAX);
    let listed = listed(
        &listing,
        &reached.scored,
        (max_files, request.settings.span_max_lines),
    )?;
    if listed.is_empty() {
        return Ok(silent(Silence::NothingToShow, Some(features), &weighed));
    }
    let order = WorkOrder {
        basis: basis_of(rule, &reached.scored, &support),
        items: listed.iter().map(|entry| entry.item.clone()).collect(),
    };
    let Some(rendered) = render(&order, request.root, &limits_of(request)) else {
        return Ok(silent(Silence::NothingToShow, Some(features), &weighed));
    };
    let listed = written_of(&listed, &rendered);
    // INVARIANT: silent-when-unsure (the gate weighed the three best files: a work order showing
    // none of them shows what the gate never weighed)
    if !shows_gated(&reached.scored, &listed) {
        return Ok(silent(Silence::NothingToShow, Some(features), &weighed));
    }
    Ok(Retrieval {
        outcome: Outcome::WorkOrder(rule, rendered),
        features: Some(features),
        weighed,
        listed,
    })
}

/// The engine without the gate: the files `request` points at, at most `limit`, each with its
/// spans and reason, whatever the gate would say, and what it would say. It answers in every mode;
/// a file of which nothing true can be said is still passed over, and a question that points at
/// nothing finds nothing.
///
/// # Errors
///
/// The store's errors.
pub fn locate(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    request: &Request<'_>,
    limit: usize,
) -> Result<Location, RetrieveError> {
    let Some(reached) = reached(queries, graph, request)? else {
        return Ok(Location {
            judged: None,
            listed: Vec::new(),
            rendered: None,
        });
    };
    let (support, features, verdict) = reached.judged(graph, request);
    let listing = Listing {
        queries,
        graph,
        seeds: &reached.seeds,
        support: &support,
    };
    let listed = listed(
        &listing,
        &reached.scored,
        (limit, request.settings.span_max_lines),
    )?;
    let answer = WorkOrder {
        basis: Basis::Located {
            kinds: most_kinds(&reached.scored, &support),
            verdict,
        },
        items: listed.iter().map(|entry| entry.item.clone()).collect(),
    };
    let limits = Limits {
        max_files: limit,
        ..limits_of(request)
    };
    let rendered = render(&answer, request.root, &limits);
    let listed = rendered
        .as_ref()
        .map(|rendered| written_of(&listed, rendered))
        .unwrap_or_default();
    Ok(Location {
        judged: Some((features, verdict)),
        listed,
        rendered,
    })
}

/// What a retrieval reached before the gate.
struct Reached {
    /// What each source seeded.
    seeds: Seeds,
    /// Where activation went.
    activation: Activation,
    /// The files it reached, the best first.
    scored: Vec<ScoredFile>,
    /// Whether the deadline stopped the spreading, or either of huge mode's two stages.
    is_out_of_time: bool,
}

impl Reached {
    /// The files the gate weighs, the three best, by node.
    fn weighed(&self, graph: &Graph<NodeId>) -> Vec<NodeId> {
        self.scored
            .iter()
            .take(GATED_FILES)
            .filter_map(|file| graph.id(file.file))
            .collect()
    }

    /// The evidence behind the files reached over `graph`, the gate's features, and what the gate
    /// says, its bar raised as `request` asks.
    fn judged<'a>(
        &'a self,
        graph: &'a Graph<NodeId>,
        request: &Request<'_>,
    ) -> (Support<'a, NodeId>, GateFeatures, Verdict) {
        let support = Support::new(graph, &self.seeds);
        let found = Found {
            seeds: &self.seeds,
            activation: &self.activation,
            scored: &self.scored,
            support: &support,
        };
        let features = features_of(&found, request.recent_precision);
        // INVARIANT: deterministic (a spreading the deadline cut short hangs on how fast the machine
        // was, so what it ranked is never shown: the gate stays shut and says why)
        let verdict = if self.is_out_of_time {
            Verdict::Silent(Shut::OutOfTime)
        } else {
            decide(&features, request.raise)
        };
        (support, features, verdict)
    }
}

/// The seeds `request`'s prompt and session give, the activation spread from them and the files
/// it reached; none when nothing is seeded.
fn reached(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    request: &Request<'_>,
) -> Result<Option<Reached>, RetrieveError> {
    let terms = PromptTerms::of(request.prompt);
    let seeds = seeds(
        queries,
        graph,
        &SeedQuery {
            terms: &terms,
            session: request.session,
            uncommitted: request.uncommitted,
        },
    )?;
    if seeds.vector.is_empty() {
        return Ok(None);
    }
    let start: Vec<(u32, f64)> = seeds
        .vector
        .iter()
        .map(|(node, share)| (*node, *share))
        .collect();
    let (teleport, deadline) = (request.settings.teleport, request.deadline);
    // Huge repositories spread over files first, then over the symbols of the best files.
    let (activation, scored, is_out_of_time) = if request.mode == ScaleMode::Huge {
        let Staged {
            activation,
            scored,
            within,
        } = two_stage(graph, &start, teleport, deadline)?;
        let is_out_of_time = activation.is_out_of_time || within.is_out_of_time;
        (activation, scored, is_out_of_time)
    } else {
        let activation = spread(graph, &start, teleport, deadline);
        let scored = scored_files(graph, &activation);
        let is_out_of_time = activation.is_out_of_time;
        (activation, scored, is_out_of_time)
    };
    let named: BTreeSet<u32> = seeds
        .lexical
        .files
        .iter()
        .filter(|(_, matched)| matched.is_named)
        .map(|(file, _)| *file)
        .collect();
    let scored = named_first(scored, &named);
    Ok(Some(Reached {
        seeds,
        activation,
        scored,
        is_out_of_time,
    }))
}

/// The entries of `listed` the work order `rendered` wrote, each with only the spans whose lines it
/// wrote, and those whose code it wrote, as `rendered` gives them by position.
fn written_of(listed: &[Listed], rendered: &Rendered) -> Vec<Listed> {
    rendered
        .written
        .iter()
        .filter_map(|(at, spans)| {
            let entry = listed.get(*at)?;
            Some(Listed {
                file: entry.file,
                spans: spans
                    .iter()
                    .filter_map(|span| entry.spans.get(*span).copied())
                    .collect(),
                item: Item {
                    spans: spans
                        .iter()
                        .filter_map(|span| entry.item.spans.get(*span).cloned())
                        .collect(),
                    ..entry.item.clone()
                },
                shown: rendered
                    .shown
                    .iter()
                    .filter(|(item, _)| item == at)
                    .filter_map(|(_, span)| entry.spans.get(*span).copied())
                    .collect(),
            })
        })
        .collect()
}

/// Whether `listed` holds one of the three best of `scored`, the files the gate weighed.
fn shows_gated(scored: &[ScoredFile], listed: &[Listed]) -> bool {
    scored
        .iter()
        .take(GATED_FILES)
        .any(|file| listed.iter().any(|entry| entry.file == file.file))
}

/// What the header says agreed: for agreement, the kinds of evidence behind the one of the top
/// three files that has the most.
fn basis_of<Id: Ord + Copy>(rule: Rule, scored: &[ScoredFile], support: &Support<'_, Id>) -> Basis {
    match rule {
        Rule::Continuity => Basis::Continuity,
        Rule::Agreement => Basis::Agreement(most_kinds(scored, support)),
    }
}

/// The names of the kinds of evidence behind the one of the three best files that has the most,
/// the better ranked of two alike.
fn most_kinds<Id: Ord + Copy>(
    scored: &[ScoredFile],
    support: &Support<'_, Id>,
) -> Vec<&'static str> {
    let mut most: Vec<Evidence> = Vec::new();
    for file in scored.iter().take(GATED_FILES) {
        let kinds: Vec<Evidence> = support.of(file.file).into_iter().collect();
        if kinds.len() > most.len() {
            most = kinds;
        }
    }
    most.into_iter().map(evidence_name).collect()
}

/// How the header names a kind of evidence.
fn evidence_name(evidence: Evidence) -> &'static str {
    match evidence {
        Evidence::Lexical => "the prompt's words",
        Evidence::Structural => "the code's references",
        Evidence::CoChange => "the commit history",
        Evidence::CoAccess => "earlier work on these files",
        Evidence::Similar => "similar earlier tasks",
    }
}

/// The limits `request`'s work order is held to: small mode holds it to 400 tokens.
fn limits_of(request: &Request<'_>) -> Limits {
    let max_tokens = match request.mode {
        ScaleMode::Small => request.settings.max_tokens.min(SMALL_MODE_MAX_TOKENS),
        _ => request.settings.max_tokens,
    };
    Limits {
        max_tokens,
        max_files: usize::try_from(request.settings.max_files).unwrap_or(usize::MAX),
        max_code_spans: usize::try_from(request.settings.max_code_spans).unwrap_or(0),
        span_max_lines: request.settings.span_max_lines,
        max_file_bytes: request.max_file_bytes,
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::{LineRange, RepoPath};

    use super::*;
    use crate::reasons::Reason;
    use crate::render::ItemSpan;
    use crate::spans::Span;

    /// A file scored at `score`, by node.
    fn scored(file: u32, score: f64) -> ScoredFile {
        ScoredFile {
            file,
            score,
            activation: score,
            symbols: Vec::new(),
        }
    }

    /// File `file`, at `path`, listed with spans of lines 1 to 2 and 4 to 5.
    fn entry(file: u32, path: &str) -> Result<Listed, Box<dyn std::error::Error>> {
        let lines = [LineRange::new(1, 2)?, LineRange::new(4, 5)?];
        Ok(Listed {
            file,
            spans: lines
                .iter()
                .map(|lines| Span {
                    lines: *lines,
                    symbol: None,
                })
                .collect(),
            item: Item {
                path: RepoPath::new(path)?,
                content: None,
                spans: lines
                    .iter()
                    .map(|lines| ItemSpan {
                        lines: *lines,
                        name: None,
                    })
                    .collect(),
                reason: Reason::ReadHere,
            },
            shown: Vec::new(),
        })
    }

    /// The gate weighs the three best files: a list holding the third shows one of them, a list of
    /// the fourth and fifth alone shows none.
    #[test]
    fn a_list_shows_the_gated_files_only_when_it_holds_one()
    -> Result<(), Box<dyn std::error::Error>> {
        let ranked: Vec<ScoredFile> = (1..=5)
            .map(|file| scored(file, 1.0 / f64::from(file)))
            .collect();
        assert!(shows_gated(
            &ranked,
            &[entry(3, "c.py")?, entry(4, "d.py")?]
        ));
        assert!(!shows_gated(
            &ranked,
            &[entry(4, "d.py")?, entry(5, "e.py")?]
        ));
        assert!(!shows_gated(&ranked, &[]));
        Ok(())
    }

    /// Only what the work order wrote is listed: of three entries, the third and then the first
    /// written, the third with its second span alone, the first with both; the second, shed, is not.
    /// The code shown: the first entry's second span alone (lines 4 to 5); none of the third.
    #[test]
    fn only_what_was_written_is_listed() -> Result<(), Box<dyn std::error::Error>> {
        let listed = [entry(1, "a.py")?, entry(2, "b.py")?, entry(3, "c.py")?];
        let rendered = Rendered {
            text: String::new(),
            tokens: 0,
            written: vec![(2, vec![1]), (0, vec![0, 1])],
            shown: vec![(0, 1)],
        };
        let written = written_of(&listed, &rendered);
        let files: Vec<u32> = written.iter().map(|entry| entry.file).collect();
        assert_eq!(files, [3, 1]);
        let lines: Vec<Vec<(u32, u32)>> = written
            .iter()
            .map(|entry| {
                entry
                    .item
                    .spans
                    .iter()
                    .map(|span| (span.lines.start_line(), span.lines.end_line()))
                    .collect()
            })
            .collect();
        assert_eq!(lines, [vec![(4, 5)], vec![(1, 2), (4, 5)]]);
        let spans: Vec<usize> = written.iter().map(|entry| entry.spans.len()).collect();
        assert_eq!(spans, [1, 2]);
        let shown: Vec<Vec<(u32, u32)>> = written
            .iter()
            .map(|entry| {
                entry
                    .shown
                    .iter()
                    .map(|span| (span.lines.start_line(), span.lines.end_line()))
                    .collect()
            })
            .collect();
        assert_eq!(shown, [vec![], vec![(4, 5)]]);
        Ok(())
    }
}
