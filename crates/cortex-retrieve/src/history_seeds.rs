//! Seeds from the repository's past changes, while `history.seeds` is on: the files of a change
//! the prompt names, and the files of past changes whose words are like the prompt's. Both read
//! only the change cards the daemon keeps (a card for each commit it found owning lines of code it
//! showed), never git; a change with no card seeds nothing.
//!
//! A change is named by its pull or merge request (`#482`, `!482`), an issue it fixes (`#471`), or
//! its commit id written in full or cut to at least seven hexadecimal digits that start the id of
//! one card only (see [`named_in`] for the words that are not taken as a change's number).
//!
//! A past change is like the prompt when its words (its subject's and reason line's, split like
//! identifiers) match the prompt's words by BM25 over every card's words. Its score is taken over
//! the most a card could score against these words, the sum of the words' weights (each word's
//! inverse document frequency, as the full-text index weighs it), and never over less than twice
//! the weight of a word one card in a hundred holds, so that one common word cannot make a perfect
//! match: a card holding every word of the prompt that any card holds, once each, at the cards'
//! usual length, scores about 1. The best [`MAX_SIMILAR_CHANGES`] normal changes at
//! [`SIMILAR_CHANGE_BAR`] or above seed.
//!
//! Each change seeds a share spread evenly over the files it changed: [`REFERENCE_WEIGHT`] for a
//! named change, `history.seed_weight` times its score for a change like the prompt, so that a
//! change of many files weighs no more in the search than one of a single file and cannot drown
//! what the other sources found. A file seeded by several changes takes the most; a weight of 0 or
//! less (`history.seed_weight = 0`) seeds nothing.
//!
//! The sources are a table ([`SOURCES`]): adding one is a function and a line.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use cortex_store::{CardKind, ChangeCardRecord, NodeId, Queries, StoreError};

use crate::graph::Graph;
use crate::prompt::PromptTerms;

/// What a named change weighs as a seed, spread over the files it changed: the prompt points at
/// that change in so many words, so it comes just below a file the prompt names (1), and above any
/// change found by likeness. It is no setting: nothing has measured another value, and it only
/// orders the named change's files against what the other sources find.
pub const REFERENCE_WEIGHT: f64 = 0.8;

/// The most past changes like the prompt that seed.
pub const MAX_SIMILAR_CHANGES: usize = 3;

/// The least score a past change like the prompt needs to seed, chosen on a replay of two Python
/// projects' histories: each sampled commit's subject the prompt, and a card made of every commit
/// before it.
pub const SIMILAR_CHANGE_BAR: f64 = 0.6;

/// The fewest hexadecimal digits a commit id is written with to name a change, as git abbreviates
/// one by default.
pub const MIN_ID_DIGITS: usize = 7;

/// The least number a `#` or `!` names a change by: a smaller one is too often a count or a rank
/// (`step #1`, `the #1 priority`, `!0`).
pub const MIN_CHANGE_NUMBER: u64 = 10;

/// The words that, written just before `#` and a number, make it a count or a rank, not a
/// change's (`no. #13`, `step #12`), sorted so that a word is found by binary search.
const ORDINAL_WORDS: &[&str] = &[
    "item", "level", "no", "nr", "num", "number", "option", "page", "part", "phase", "point",
    "rank", "stage", "step", "top",
];

/// The words that make a prompt one about colours, in which `#` and three, four, six or eight
/// digits is a colour (`#333`), sorted so that a word is found by binary search.
const COLOUR_WORDS: &[&str] = &[
    "background",
    "border",
    "color",
    "colors",
    "colour",
    "colours",
    "css",
    "fill",
    "palette",
    "rgb",
    "rgba",
    "stroke",
];

/// The most changes one prompt names that are looked up, the first written first: a pasted log
/// names dozens, and each is a query.
const MAX_NAMED: usize = 4;

/// The most cards one number finds that seed, the newest first: an issue many commits name says
/// less about each.
const MAX_CARDS_A_NUMBER: usize = 3;

/// The most cards searched for the best normal ones: cards of other kinds that match are passed
/// over.
const SEARCHED_CARDS: usize = 24;

/// Most of the prompt's words matched against the cards, the most written first: each is counted
/// with a query.
const MAX_WORDS: usize = 24;

/// The share of cards whose word sets the least the most a card could score is taken as: twice
/// the weight of a word one card in a hundred holds.
const FLOOR_SHARE: f64 = 0.01;

/// What the past changes give.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HistoryMatch {
    /// Each file seeded, with its weight, by node of the graph.
    pub seeds: BTreeMap<u32, f64>,
}

/// What history seeds are found with.
#[derive(Clone, Copy)]
pub struct HistoryAsk<'a> {
    /// The prompt's text.
    pub prompt: &'a str,
    /// The prompt's words.
    pub terms: &'a PromptTerms,
    /// `history.seed_weight`: what a past change like the prompt weighs, times its score, spread
    /// over its files.
    pub seed_weight: f64,
}

impl fmt::Debug for HistoryAsk<'_> {
    /// The weight; the prompt is a person's words and stays out of logs.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HistoryAsk")
            .field("seed_weight", &self.seed_weight)
            .finish_non_exhaustive()
    }
}

/// One source of history seeds: each file node it finds, with its weight.
type Source = fn(Queries<'_>, &HistoryAsk<'_>) -> Result<Vec<(NodeId, f64)>, StoreError>;

/// Every source of history seeds.
const SOURCES: &[Source] = &[named_changes, similar_changes];

/// The seeds `ask` finds among the cards `queries` reads, as nodes of `graph`: each file node of
/// the graph any source found at a weight above 0, with the most weight a source gave it.
///
/// # Errors
///
/// The store's errors.
pub fn history_seeds(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    ask: &HistoryAsk<'_>,
) -> Result<HistoryMatch, StoreError> {
    let mut found = HistoryMatch::default();
    for source in SOURCES {
        for (node, weight) in source(queries, ask)? {
            // A seed of no weight would still count as history's evidence for its file.
            let Some(index) = graph.index_of(node).filter(|_| weight > 0.0) else {
                continue;
            };
            let held = found.seeds.entry(index).or_insert(0.0);
            *held = held.max(weight);
        }
    }
    Ok(found)
}

/// A change a prompt names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// A pull or merge request, or an issue, by its number.
    Number(u64),
    /// A commit, by the start of its id, lower case.
    Commit(String),
}

/// The changes `prompt` names, in the order first written, each once, at most [`MAX_NAMED`].
///
/// A number is a word that is `#` or `!` and digits (`#482`, `(!12)`), standing alone: not part
/// of a longer word (`C#12`, `owner/repo#12`, which names another repository's issue, `#12x`).
/// It names nothing when it is below [`MIN_CHANGE_NUMBER`] (`step #1`, `the #1 priority`, `!0`,
/// `!1`), when the word before it makes it a count or a rank (`no. #13`, `step #12`),
/// when no card could hold it (past the largest number the store keeps), or when the prompt is
/// about colours and its digits are as many as a colour's (three, four, six or eight: `#333`). A
/// prompt is about colours when it writes a colour word (`color`, `css`, `background`, ...), a `#`
/// word of a colour's length holding a hexadecimal letter (`#fff`), or two or more `#` words of
/// a colour's length (`#333 to #444`): two different changes named by three-digit numbers in
/// one prompt are then not taken either.
///
/// A commit is a word of [`MIN_ID_DIGITS`] to 40 hexadecimal digits standing alone, in any case.
#[must_use]
pub fn named_in(prompt: &str) -> Vec<Named> {
    let words: Vec<&str> = prompt
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '#' | '!' | '/' | '_' | '-'))
        })
        .filter(|word| !word.is_empty())
        .collect();
    let is_about_colours = is_about_colours(&words);
    let mut named: Vec<Named> = Vec::new();
    let mut before: Option<&str> = None;
    for word in &words {
        let found = number_in(word, before, is_about_colours)
            .map(Named::Number)
            .or_else(|| {
                let lower = word.to_ascii_lowercase();
                is_commit_start(&lower).then_some(Named::Commit(lower))
            });
        before = Some(word);
        if let Some(found) = found
            && !named.contains(&found)
        {
            named.push(found);
        }
        if named.len() == MAX_NAMED {
            break;
        }
    }
    named
}

/// The number `word` names a change by, `before` being the word written just before it and
/// `is_about_colours` whether the prompt is about colours (see [`named_in`]).
fn number_in(word: &str, before: Option<&str>, is_about_colours: bool) -> Option<u64> {
    let digits = word.strip_prefix('#').or_else(|| word.strip_prefix('!'))?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if is_about_colours && word.starts_with('#') && is_colour_length(digits) {
        return None;
    }
    let is_ordinal = before.is_some_and(|before| {
        ORDINAL_WORDS
            .binary_search(&before.to_ascii_lowercase().as_str())
            .is_ok()
    });
    if is_ordinal {
        return None;
    }
    let number: u64 = digits.parse().ok()?;
    // The store keeps numbers as SQLite integers: a larger one is no change's.
    (number >= MIN_CHANGE_NUMBER && i64::try_from(number).is_ok()).then_some(number)
}

/// Whether `words` make a prompt about colours (see [`named_in`]).
fn is_about_colours(words: &[&str]) -> bool {
    let mut colour_lengths: BTreeSet<String> = BTreeSet::new();
    for word in words {
        let lower = word.to_ascii_lowercase();
        if COLOUR_WORDS.binary_search(&lower.as_str()).is_ok() {
            return true;
        }
        let Some(hex) = lower.strip_prefix('#') else {
            continue;
        };
        if is_colour_length(hex) && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            if hex.bytes().any(|byte| byte.is_ascii_alphabetic()) {
                return true;
            }
            colour_lengths.insert(lower.clone());
        }
    }
    // Two different ones: the same number written twice is one reference.
    colour_lengths.len() >= 2
}

/// Whether `digits` are as many as a colour's: three, four, six or eight.
fn is_colour_length(digits: &str) -> bool {
    matches!(digits.len(), 3 | 4 | 6 | 8)
}

/// Whether `word`, lower case, could start a commit id: from [`MIN_ID_DIGITS`] to 40 hexadecimal
/// digits.
fn is_commit_start(word: &str) -> bool {
    (MIN_ID_DIGITS..=40).contains(&word.len()) && word.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Each file of `card` with its share of `weight`, spread evenly over them.
fn spread(card: &ChangeCardRecord, weight: f64) -> Vec<(NodeId, f64)> {
    let Ok(count) = u32::try_from(card.files.len()) else {
        return Vec::new();
    };
    if count == 0 {
        return Vec::new();
    }
    let share = weight / f64::from(count);
    card.files.iter().map(|file| (*file, share)).collect()
}

/// The files of the changes the prompt names, each change's [`REFERENCE_WEIGHT`] spread over its
/// files: for a number, the newest [`MAX_CARDS_A_NUMBER`] cards of its pull request or naming its
/// issue; for a commit, the one card whose id starts so, none when two do.
fn named_changes(
    queries: Queries<'_>,
    ask: &HistoryAsk<'_>,
) -> Result<Vec<(NodeId, f64)>, StoreError> {
    let mut cards: Vec<ChangeCardRecord> = Vec::new();
    for named in named_in(ask.prompt) {
        match named {
            Named::Number(number) => {
                let found = queries.change_cards_numbered(number)?;
                cards.extend(found.into_iter().take(MAX_CARDS_A_NUMBER));
            }
            Named::Commit(start) => {
                // INVARIANT: silent-when-unsure (a start two cards share names neither)
                if let [card] = &queries.change_cards_starting(&start, 2)?[..] {
                    cards.push(card.clone());
                }
            }
        }
    }
    Ok(cards
        .iter()
        .flat_map(|card| spread(card, REFERENCE_WEIGHT))
        .collect())
}

/// The files of the best [`MAX_SIMILAR_CHANGES`] normal changes like the prompt, each change's
/// `history.seed_weight` times its score spread over its files, the changes below
/// [`SIMILAR_CHANGE_BAR`] left out.
fn similar_changes(
    queries: Queries<'_>,
    ask: &HistoryAsk<'_>,
) -> Result<Vec<(NodeId, f64)>, StoreError> {
    Ok(scored_changes(queries, ask.terms)?
        .into_iter()
        // INVARIANT: silent-when-unsure (a change scoring below the bar seeds nothing)
        .filter(|(_, score)| *score >= SIMILAR_CHANGE_BAR)
        .flat_map(|(card, score)| spread(&card, ask.seed_weight * score))
        .collect())
}

/// The best [`MAX_SIMILAR_CHANGES`] normal changes whose words match `terms`' words, best first,
/// each with its score from 0 to 1, whatever the bar: BM25 over the most a card could score (see
/// the module's documentation). None when no card holds any of the words.
///
/// # Errors
///
/// The store's errors.
pub fn scored_changes(
    queries: Queries<'_>,
    terms: &PromptTerms,
) -> Result<Vec<(ChangeCardRecord, f64)>, StoreError> {
    let rows = queries.commit_term_rows()?;
    if rows == 0 {
        return Ok(Vec::new());
    }
    let mut held: Vec<&str> = Vec::new();
    let mut total = 0.0;
    for word in terms.frequent_words(MAX_WORDS) {
        let count = queries.commit_term_cards(word)?;
        if count > 0 {
            held.push(word);
            total += idf(rows, count);
        }
    }
    if held.is_empty() {
        return Ok(Vec::new());
    }
    let whole = total.max(2.0 * idf(rows, share_of(rows, FLOOR_SHARE)));
    Ok(queries
        .search_change_cards(&held, SEARCHED_CARDS)?
        .into_iter()
        .filter(|(card, _)| card.kind == CardKind::Normal)
        .take(MAX_SIMILAR_CHANGES)
        .map(|(card, score)| (card, (score / whole).clamp(0.0, 1.0)))
        .collect())
}

/// The weight the full-text index gives a word `count` of `rows` cards hold: the inverse document
/// frequency of its BM25, `ln((rows - count + 0.5) / (count + 0.5))`, and a millionth for a word
/// half the cards or more hold, as the index takes it.
fn idf(rows: u64, count: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "counts of cards are far below where a float loses whole numbers"
    )]
    let (rows, count) = (rows as f64, count as f64);
    let weight = ((rows - count + 0.5) / (count + 0.5)).ln();
    if weight > 0.0 { weight } else { 1e-6 }
}

/// How many of `rows` cards make `share` of them, at least one.
fn share_of(rows: u64, share: f64) -> u64 {
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a count of cards is far below where a float loses whole numbers, and the share \
                  is between 0 and 1"
    )]
    let count = (rows as f64 * share).ceil() as u64;
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The changes a prompt names, case by case: `#482` and `!12` by number, a seven-digit start
    /// and a whole id in any case by commit, each once and in order; not `C#12`, `owner/repo#12`,
    /// `#12x`, a bare `#`, six digits, a word with a letter past `f`, or 41 digits; at most four.
    #[test]
    fn a_prompt_names_changes_by_number_and_by_commit() {
        let whole = "0123456789abcdef0123456789abcdef01234567";
        let prompt = format!(
            "Redo #482 (see !12), like a1b2c3d and {}; not C#12, owner/repo#12, #12x, #, abcdef, \
             a1b2c3g, {whole}8. Again #482.",
            whole.to_uppercase()
        );
        assert_eq!(
            named_in(&prompt),
            [
                Named::Number(482),
                Named::Number(12),
                Named::Commit("a1b2c3d".to_owned()),
                Named::Commit(whole.to_owned()),
            ]
        );
        assert_eq!(
            named_in("#11 #12 #13 #14 #15"),
            [11, 12, 13, 14].map(Named::Number)
        );
        assert_eq!(named_in("fix the tax rounding"), []);
    }

    /// Counts, ranks and colours name no change, each prompt worked by hand: below 10 (`step #1`,
    /// `the #1 priority`, `!0`, `!1`, `no. #3`); after a word that makes it a rank (`no. #13`,
    /// `Step #12`); two colours of three digits (`#333 to #444`); one colour word (`color`) or one
    /// colour with a letter (`#fff`) with `#333`. Four digits beside a colour word is a colour, five
    /// is not; with no colour around, `#333` names a change. A number past the largest the store
    /// keeps names nothing, the one at it does.
    #[test]
    fn counts_ranks_and_colours_name_no_change() {
        for prompt in [
            "do step #1 first",
            "make it the #1 priority",
            "change the header from #333 to #444",
            "set !0 and !1",
            "see no. #3",
            "see no. #13",
            "Step #12 fails",
            "the color #333 is too dark",
            "use #fff and #333",
            "set the border to #1234",
            "#9223372036854775808",
        ] {
            assert_eq!(named_in(prompt), [], "{prompt}");
        }
        assert_eq!(
            named_in("set the border to #12345"),
            [Named::Number(12_345)]
        );
        assert_eq!(named_in("undo #333"), [Named::Number(333)]);
        assert_eq!(
            named_in("#9223372036854775807"),
            [Named::Number(9_223_372_036_854_775_807)]
        );
    }

    /// The word weights, worked by hand: a word 1 of 8 cards holds weighs ln(7.5 / 1.5) = ln 5; a
    /// word 4 of 8 hold weighs ln(4.5 / 4.5) = 0, taken as a millionth; one card in a hundred of 8
    /// cards is one card, of 250 three.
    #[test]
    fn words_are_weighed_as_the_index_weighs_them() {
        assert!((idf(8, 1) - 5.0_f64.ln()).abs() < 1e-12);
        assert!((idf(8, 4) - 1e-6).abs() < 1e-18);
        assert!((idf(8, 7) - 1e-6).abs() < 1e-18);
        assert_eq!(share_of(8, FLOOR_SHARE), 1);
        assert_eq!(share_of(250, FLOOR_SHARE), 3);
    }
}
