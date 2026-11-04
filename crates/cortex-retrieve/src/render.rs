//! The work order as the agent reads it: plain text in a fixed order, every line number and line of
//! code read from the file on disk as it is when the text is built.
//!
//! The header says what agreed. "Start here" lists the items whose code is shown, each with its
//! lines, the symbol they are for and why it is listed; "Also likely" the rest, by path and lines;
//! "Current code" holds the shown spans' lines, numbered. A part with nothing in it is left out.
//! Before an item's lines are written its file is read and hashed: a file that is not the one the
//! index read lends no lines, and its item is listed by path alone. Control characters (but tabs)
//! and the characters that reorder or hide text are taken out of the code shown; an item whose
//! path, symbol or reason holds one is not listed at all, since it could only be shown as a name
//! that is not the file's.
//!
//! The text is held to its caps where it is built: at most the files, spans a file and lines a span
//! the caller gives, the token cap the caller gives, never above the hard cap of 1,200 tokens,
//! and 8,000 characters. Past a cap, code is shed first (the last body, then the next), then the
//! items from the last; a work order that cannot fit with one item is not rendered at all.

use std::fmt::Write as _;
use std::path::Path;

use cortex_core::{
    ContentHash, Identifier, LineRange, MAX_WORK_ORDER_TOKENS, RepoPath, estimate_tokens, is_hidden,
};

use crate::disk::CheckedFile;
use crate::gate::{Rule, Shut, Verdict};
use crate::reasons::Reason;
use crate::spans::MAX_SPANS_PER_FILE;

/// Most characters a work order may hold, whatever its token cap.
pub const MAX_WORK_ORDER_CHARS: usize = 8_000;

/// What the header says: for a work order, what agreed; for the engine asked directly, what found
/// the files and what the gate would have said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Basis {
    /// Independent kinds of evidence agreed: their names as the header says them.
    Agreement(Vec<&'static str>),
    /// The session is continuing its own work.
    Continuity,
    /// The engine was asked directly, without the gate.
    Located {
        /// The kinds of evidence behind the best-supported top file, as the header names them.
        kinds: Vec<&'static str>,
        /// What the gate would have said.
        verdict: Verdict,
    },
}

/// A stretch of an item's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSpan {
    /// Its lines, as the index read them.
    pub lines: LineRange,
    /// The symbol it is for, when it is for one.
    pub name: Option<Identifier>,
}

/// One file the work order lists.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// The file.
    pub path: RepoPath,
    /// What the index holds for it: its lines are written only while the file holds that.
    pub content: Option<ContentHash>,
    /// Its spans, the strongest first.
    pub spans: Vec<ItemSpan>,
    /// Why it is listed.
    pub reason: Reason,
}

/// What a work order holds before it is written.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkOrder {
    /// What agreed.
    pub basis: Basis,
    /// The files listed, the best first.
    pub items: Vec<Item>,
}

/// How a work order is held in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most tokens it may take, as estimated; held to the hard cap.
    pub max_tokens: u32,
    /// The most files it lists.
    pub max_files: usize,
    /// How many spans may have their code shown.
    pub max_code_spans: usize,
    /// The most lines a span may cover; a longer one is not written.
    pub span_max_lines: u32,
    /// The most bytes a file may hold to be read.
    pub max_file_bytes: u64,
}

/// A written work order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The text.
    pub text: String,
    /// Its estimated tokens.
    pub tokens: u32,
    /// The items written, by their position in the order, each with the spans whose lines were
    /// written, by position.
    pub written: Vec<(usize, Vec<usize>)>,
}

/// `order` written for the repository at `root`, held to `limits`; `None` when it has no item that
/// can be shown as it is, or not even its first fits.
#[must_use]
pub fn render(order: &WorkOrder, root: &Path, limits: &Limits) -> Option<Rendered> {
    let files: Vec<Option<CheckedFile>> = order
        .items
        .iter()
        .take(limits.max_files)
        .map(|item| {
            // INVARIANT: true-at-render (an item's lines come from its file as it is now, and only
            // while that is the file the index read)
            let content = item.content?;
            CheckedFile::read(root, &item.path, &content, limits.max_file_bytes).ok()
        })
        .collect();
    let mut plan = Plan::of(order, &files, limits);
    // INVARIANT: silent-when-unsure (a work order with no item to show is not given: a header
    // alone would say the evidence agrees on nothing it names)
    if plan.kept.is_empty() {
        return None;
    }
    let max_tokens = limits.max_tokens.min(MAX_WORK_ORDER_TOKENS);
    loop {
        let text = plan.write(order, &files);
        let tokens = u32::try_from(estimate_tokens(&text)).unwrap_or(u32::MAX);
        // INVARIANT: budgeted (the work order is held to its token cap and the hard caps where it
        // is written, shedding code, then items, until it fits). Within 1,200 tokens it holds at
        // most 4,800 bytes and so as many characters; the character cap is kept all the same, for
        // a token cap that may one day be raised.
        if tokens <= max_tokens && text.chars().count() <= MAX_WORK_ORDER_CHARS {
            return Some(Rendered {
                text,
                tokens,
                written: plan.written(),
            });
        }
        if !plan.shed() {
            return None;
        }
    }
}

/// Which items and spans are written, and which spans have their code shown.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    /// For each item kept, by position: the spans whose lines may be written, by position.
    kept: Vec<(usize, Vec<usize>)>,
    /// The spans whose code is shown, as (item, span), in the order they are shown.
    shown: Vec<(usize, usize)>,
}

impl Plan {
    /// The first items of `order` that can be shown as they are, each with the first spans its
    /// checked file holds whole and the caps of `limits` allow, and the first `max_code_spans` of
    /// those spans, round by round over the items, with their code shown.
    fn of(order: &WorkOrder, files: &[Option<CheckedFile>], limits: &Limits) -> Self {
        let kept: Vec<(usize, Vec<usize>)> = order
            .items
            .iter()
            .enumerate()
            .take(limits.max_files)
            .filter(|(_, item)| is_shown_as_it_is(item))
            .map(|(at, item)| {
                let spans = match files.get(at).and_then(Option::as_ref) {
                    Some(file) => (0..item.spans.len())
                        .filter(|span| {
                            item.spans.get(*span).is_some_and(|span| {
                                span.lines.end_line() <= file.line_count()
                                    && span.lines.end_line() - span.lines.start_line()
                                        < limits.span_max_lines
                            })
                        })
                        .take(MAX_SPANS_PER_FILE)
                        .collect(),
                    None => Vec::new(),
                };
                (at, spans)
            })
            .collect();
        let max_code_spans = limits.max_code_spans;
        let mut shown = Vec::new();
        let deepest = kept.iter().map(|(_, spans)| spans.len()).max().unwrap_or(0);
        'rounds: for round in 0..deepest {
            for (at, spans) in &kept {
                if shown.len() >= max_code_spans {
                    break 'rounds;
                }
                if let Some(span) = spans.get(round) {
                    shown.push((*at, *span));
                }
            }
        }
        Self { kept, shown }
    }

    /// Takes one thing out to make the text shorter: the last code shown, else the last item not
    /// shown, else the last item after the first; `false` when there is nothing left to take.
    fn shed(&mut self) -> bool {
        if self.shown.pop().is_some() {
            return true;
        }
        if self.kept.len() > 1 {
            self.kept.pop();
            return true;
        }
        false
    }

    /// The items written, each with the spans whose lines are written.
    fn written(&self) -> Vec<(usize, Vec<usize>)> {
        self.kept.clone()
    }

    /// The text this plan writes of `order`, its code from `files`.
    fn write(&self, order: &WorkOrder, files: &[Option<CheckedFile>]) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "{}", heading(&order.basis));
        let is_shown = |at: usize| self.shown.iter().any(|(item, _)| *item == at);
        let start: Vec<&(usize, Vec<usize>)> =
            self.kept.iter().filter(|(at, _)| is_shown(*at)).collect();
        let also: Vec<&(usize, Vec<usize>)> =
            self.kept.iter().filter(|(at, _)| !is_shown(*at)).collect();
        if !start.is_empty() {
            text.push_str("Start here:\n");
            for (number, (at, spans)) in start.iter().enumerate() {
                let Some(item) = order.items.get(*at) else {
                    continue;
                };
                let _ = writeln!(text, "  {}. {}", number + 1, item_text(item, spans, true));
            }
        }
        if !also.is_empty() {
            let listed: Vec<String> = also
                .iter()
                .filter_map(|(at, spans)| Some(item_text(order.items.get(*at)?, spans, false)))
                .collect();
            let _ = writeln!(text, "Also likely: {}", listed.join(", "));
        }
        if !self.shown.is_empty() {
            text.push_str("Current code:\n");
            for (at, span) in &self.shown {
                let (Some(item), Some(Some(file))) = (order.items.get(*at), files.get(*at)) else {
                    continue;
                };
                let Some(span) = item.spans.get(*span) else {
                    continue;
                };
                write_code(&mut text, (item, span), file);
            }
        }
        text
    }
}

/// The header line for `basis`.
fn heading(basis: &Basis) -> String {
    match basis {
        Basis::Continuity => "[cortex] Work order · continuing this session's work".to_owned(),
        Basis::Agreement(kinds) => format!("[cortex] Work order · {}", agreed(kinds)),
        Basis::Located { kinds, verdict } => {
            let found = match kinds.as_slice() {
                [] => "no kind of evidence points here".to_owned(),
                [only] => format!("{only} alone"),
                more => agreed(more),
            };
            let gate = match verdict {
                Verdict::Inject(Rule::Agreement) => "the gate would give it",
                Verdict::Inject(Rule::Continuity) => {
                    "the gate would give it, the session continuing its own work"
                }
                Verdict::Silent(Shut::TooFewKinds) => {
                    "the gate would stay silent: too few kinds of evidence agree"
                }
                Verdict::Silent(Shut::WeakMatch) => {
                    "the gate would stay silent: the words match no file strongly"
                }
            };
            format!("[cortex] Where · {found} · {gate}")
        }
    }
}

/// `kinds` said as agreeing: "a and b agree", "a, b and c agree".
fn agreed(kinds: &[&str]) -> String {
    match kinds {
        [] => "the evidence agrees".to_owned(),
        [only] => format!("{only} points here"),
        [first @ .., last] => format!("{} and {last} agree", first.join(", ")),
    }
}

/// Whether `item` can be listed as it is: its path, its spans' symbols and its reason hold no
/// character the work order would have to take out, so what it names is the file and the symbol
/// themselves.
fn is_shown_as_it_is(item: &Item) -> bool {
    let is_plain = |text: &str| text.chars().all(|character| !is_taken_out(character));
    is_plain(item.path.as_str())
        && is_plain(&item.reason.text())
        && item
            .spans
            .iter()
            .filter_map(|span| span.name.as_ref())
            .all(|name| is_plain(name.as_str()))
}

/// One item as a list writes it: its path and its spans' lines, then, at the head of the list, the
/// first span's symbol and its reason set apart by two spaces, or, in the line of the rest, its
/// reason after one.
fn item_text(item: &Item, spans: &[usize], is_head: bool) -> String {
    let path = item.path.as_str().to_owned();
    let mut text = match spans.first().and_then(|span| item.spans.get(*span)) {
        Some(span) => format!(
            "{path}:{}-{}",
            span.lines.start_line(),
            span.lines.end_line()
        ),
        None => path,
    };
    for span in spans
        .iter()
        .skip(1)
        .filter_map(|span| item.spans.get(*span))
    {
        let _ = write!(
            text,
            ",{}-{}",
            span.lines.start_line(),
            span.lines.end_line()
        );
    }
    let gap = if is_head { "  " } else { " " };
    if is_head
        && let Some(name) = spans
            .first()
            .and_then(|span| item.spans.get(*span))
            .and_then(|span| span.name.as_ref())
    {
        let _ = write!(text, "{gap}{}", name.as_str());
    }
    let _ = write!(text, "{gap}({})", item.reason.text());
    text
}

/// Writes `span` of `item`'s file, its lines as `file` holds them now, each numbered.
fn write_code(text: &mut String, (item, span): (&Item, &ItemSpan), file: &CheckedFile) {
    let Some(lines) = file.lines(span.lines) else {
        return;
    };
    let _ = writeln!(
        text,
        "  {}:{}-{}",
        item.path.as_str(),
        span.lines.start_line(),
        span.lines.end_line()
    );
    let width = span.lines.end_line().to_string().len();
    for (number, line) in lines {
        let _ = writeln!(text, "  {number:>width$} | {}", clean(line));
    }
}

/// `text` without the characters [`is_taken_out`] names: what the agent reads is what the file
/// holds, less what it could not see or what would steer its terminal.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|character| !is_taken_out(*character))
        .collect()
}

/// Whether `character` is taken out of what the work order writes: a control character other than a
/// tab, or one that reorders or hides text (`cortex_core::is_hidden`).
fn is_taken_out(character: char) -> bool {
    character != '\t' && (character.is_control() || is_hidden(character))
}
