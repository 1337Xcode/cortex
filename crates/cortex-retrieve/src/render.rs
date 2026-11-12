//! The work order as the agent reads it: plain text in a fixed order, every line number and line of
//! code read from the file on disk as it is when the text is built.
//!
//! The header says what agreed. "Start here" lists the items whose code is shown, each with its
//! lines, the symbol they are for and why it is listed; "Also likely" the rest, by path and lines;
//! "Current code" holds the shown spans' lines, numbered, each span followed by its why-lines,
//! the reasons git history gives for those lines (`crate::why_lines`), when any were looked up;
//! "Project facts" the facts about the listed files that are true now (`crate::facts`); then the
//! experience hint, one line about earlier work like the prompt's, when one was found
//! (`crate::experience`). A part with nothing in it is left out.
//! Before an item's lines are written its file is read and hashed: a file that is not the one the
//! index read lends no lines, and its item is listed by path alone; an item whose file is gone
//! (deleted, no longer a regular file, unreadable) is not listed. Control characters (but tabs)
//! and the characters that reorder or hide text are taken out of the code shown; an item whose
//! path, symbol or reason holds one is not listed at all, since it could only be shown as a name
//! that is not the file's. An agent's reminder tag (`cortex_core::REMINDER_TAGS`) is never
//! written, since the agent would get it altered or take it for its own words: a span whose lines
//! hold one, as they would be shown, is neither shown nor named, and an item whose path, symbol or
//! reason holds one is not listed. Every other part of the text is the work order's own words.
//!
//! The text is held to its caps where it is built: at most the files, spans a file and lines a span
//! the caller gives, the token cap the caller gives, never above the hard cap of 1,200 tokens,
//! and 8,000 characters. Past a cap, why-lines are shed first, one at a time, the last of the
//! last span shown first; then the experience hint; then code, the last body shown first: it is
//! cut to its first half, again and again down to [`MIN_SHOWN_LINES`], with a line saying which
//! lines are not shown, and only then taken out; then the facts from the last; then the items from
//! the last. A work order that cannot fit with one item is not rendered at all.
//!
//! A span that starts inside a definition, a method in its class, is shown under that definition's
//! first line, so the agent sees where the code lives without reading the file for it.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use cortex_core::{
    ContentHash, Identifier, LineRange, MAX_WORK_ORDER_TOKENS, RepoPath, estimate_tokens,
    holds_reminder_tag,
};

use crate::disk::{CheckedFile, Unchecked, is_readable};
use crate::facts::{OrderFact, true_now, write_facts};
use crate::gate::{Rule, Shut, Verdict};
use crate::reasons::Reason;
use crate::spans::MAX_SPANS_PER_FILE;
use crate::why_lines::{WhyLines, choose};
use cortex_core::{is_plain, is_taken_out};

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
    /// The repository's confidence model, fitted on its earlier work orders, judged it likely
    /// used.
    Learned,
    /// The engine was asked directly, without the gate.
    Located {
        /// The kinds of evidence behind the best-supported top file, as the header names them.
        kinds: Vec<&'static str>,
        /// What the gate would have said.
        verdict: Verdict,
    },
}

/// The fewest lines of a span a work order cuts it to before it takes its code out: a header
/// and the first lines of a body.
pub const MIN_SHOWN_LINES: u32 = 8;

/// A stretch of an item's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSpan {
    /// Its lines, as the index read them.
    pub lines: LineRange,
    /// The symbol it is for, when it is for one.
    pub name: Option<Identifier>,
    /// The first line of the definition it starts inside, a method's class, when there is one.
    pub scope: Option<u32>,
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
    /// The facts about them, the closest to the code shown first.
    pub facts: Vec<OrderFact>,
    /// The experience hint, one line ending its line, written last while it fits; none for a work
    /// order with no earlier work like its prompt to tell of.
    pub hint: Option<String>,
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
    /// The spans whose code was written, each as its item's position, its own, and the lines of it
    /// written (all of them, or its first ones when it was cut), in the order written: each from
    /// the file its item's `content` says, checked as the text was built.
    pub shown: Vec<(usize, usize, LineRange)>,
    /// The facts written, in the order written.
    pub facts: Vec<cortex_store::FactId>,
    /// Whether the experience hint was written.
    pub is_hinted: bool,
}

/// `order` written for the repository at `root`, held to `limits`, with no why-line; `None` when
/// it has no item that can be shown as it is, or not even its first fits. The engine's answer to a
/// question whose spreading ran out of time is its header alone, whatever its items.
#[must_use]
pub fn render(order: &WorkOrder, root: &Path, limits: &Limits) -> Option<Rendered> {
    render_with(order, root, limits, &WhyLines::default())
}

/// `order` written as [`render`] writes it, each span shown followed by the why-lines `why` gives
/// it.
#[must_use]
pub fn render_with(
    order: &WorkOrder,
    root: &Path,
    limits: &Limits,
    why: &WhyLines,
) -> Option<Rendered> {
    // INVARIANT: deterministic (which files a spreading the clock cut short had reached is the
    // clock's doing: an answer out of time names no file, and says only that)
    if let Basis::Located {
        verdict: Verdict::Silent(Shut::OutOfTime),
        ..
    } = order.basis
    {
        let text = format!("{}\n", heading(&order.basis));
        let tokens = u32::try_from(estimate_tokens(&text)).unwrap_or(u32::MAX);
        return Some(Rendered {
            text,
            tokens,
            written: Vec::new(),
            shown: Vec::new(),
            facts: Vec::new(),
            is_hinted: false,
        });
    }
    let files: Vec<OnDisk> = order
        .items
        .iter()
        .take(limits.max_files)
        .map(|item| OnDisk::of(item, root, limits.max_file_bytes))
        .collect();
    let mut plan = Plan::of(order, &files, limits);
    // INVARIANT: silent-when-unsure (a work order with no item to show is not given: a header
    // alone would say the evidence agrees on nothing it names)
    if plan.kept.is_empty() {
        return None;
    }
    plan.facts = true_now(&order.facts, root, limits.max_file_bytes);
    plan.is_hinted = order.hint.is_some();
    let reasons = plan.reasons(order, why);
    let max_tokens = limits.max_tokens.min(MAX_WORK_ORDER_TOKENS);
    loop {
        let text = plan.write(order, &files, &reasons);
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
                shown: plan
                    .shown
                    .iter()
                    .map(|shown| (shown.item, shown.span, shown.lines))
                    .collect(),
                facts: plan
                    .facts
                    .iter()
                    .filter_map(|at| order.facts.get(*at).map(|fact| fact.id))
                    .collect(),
                is_hinted: plan.is_hinted,
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
    /// The spans whose code is shown, in the order they are shown.
    shown: Vec<Shown>,
    /// The facts written, by position, in the order written.
    facts: Vec<usize>,
    /// The spans shown whose why-lines are written, as their item's position and their own, with
    /// how many of their lines are written (their first ones), in the order shown.
    why: Vec<((usize, usize), usize)>,
    /// Whether the experience hint is written.
    is_hinted: bool,
}

/// The why-lines of each span shown that has any, by its item's position and its own.
type Reasons = BTreeMap<(usize, usize), Vec<String>>;

/// One span whose code is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shown {
    /// Its item, by position.
    item: usize,
    /// The span, by position among its item's.
    span: usize,
    /// The lines of it shown: all of them, or its first ones once it is cut.
    lines: LineRange,
}

/// What an item's file was when the work order was written.
enum OnDisk {
    /// The file the index read: its lines can be shown.
    Checked(CheckedFile),
    /// A file that is there but lends no lines: changed since the index read it, past the size
    /// cap, or with nothing indexed to check it against. Its item is listed by path alone.
    Unchecked,
    /// No regular file that can be read: deleted, replaced by a link or a folder, or unreadable.
    /// Its item is not listed: a work order names only files that are there.
    Gone,
}

impl OnDisk {
    /// What the repository at `root` holds of `item`'s file, read up to `max_bytes`.
    fn of(item: &Item, root: &Path, max_bytes: u64) -> Self {
        // INVARIANT: true-at-render (an item's lines come from its file as it is now, and only
        // while that is the file the index read; an item whose file is gone or cannot be read is
        // not listed)
        let Some(content) = item.content else {
            return if is_readable(root, &item.path) {
                Self::Unchecked
            } else {
                Self::Gone
            };
        };
        match CheckedFile::read(root, &item.path, &content, max_bytes) {
            Ok(file) => Self::Checked(file),
            Err(Unchecked::Unreadable) => Self::Gone,
            Err(Unchecked::Changed | Unchecked::TooLarge) => Self::Unchecked,
        }
    }

    /// The checked file, when its lines can be shown.
    fn checked(&self) -> Option<&CheckedFile> {
        match self {
            Self::Checked(file) => Some(file),
            Self::Unchecked | Self::Gone => None,
        }
    }

    /// Whether no file is there to name.
    fn is_gone(&self) -> bool {
        matches!(self, Self::Gone)
    }
}

impl Plan {
    /// The first items of `order` that can be shown as they are, each with the first spans its
    /// checked file holds whole and the caps of `limits` allow, and the first `max_code_spans` of
    /// those spans, round by round over the items, with their code shown.
    fn of(order: &WorkOrder, files: &[OnDisk], limits: &Limits) -> Self {
        let kept: Vec<(usize, Vec<usize>)> = order
            .items
            .iter()
            .enumerate()
            .take(limits.max_files)
            .filter(|(at, item)| {
                is_shown_as_it_is(item) && !files.get(*at).is_some_and(OnDisk::is_gone)
            })
            .map(|(at, item)| {
                // INVARIANT: true-at-render (a span, its code and its why-lines, comes only from a
                // file whose bytes are the ones the index read: the lines blamed are the lines
                // shown)
                let spans = match files.get(at).and_then(OnDisk::checked) {
                    Some(file) => (0..item.spans.len())
                        .filter(|span| {
                            item.spans.get(*span).is_some_and(|span| {
                                span.lines.end_line() <= file.line_count()
                                    && span.lines.end_line() - span.lines.start_line()
                                        < limits.span_max_lines
                                    && !is_reminder(file, span.lines)
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
                let Some(span) = spans.get(round) else {
                    continue;
                };
                if let Some(lines) = order
                    .items
                    .get(*at)
                    .and_then(|item| item.spans.get(*span))
                    .map(|item_span| item_span.lines)
                {
                    shown.push(Shown {
                        item: *at,
                        span: *span,
                        lines,
                    });
                }
            }
        }
        Self {
            kept,
            shown,
            facts: Vec::new(),
            why: Vec::new(),
            is_hinted: false,
        }
    }

    /// The why-lines of each span this plan shows, as `why` gives them; the spans that get any are
    /// noted as written. A span is shown only from a file whose bytes are the ones the index read
    /// ([`Plan::of`]), which is the content `why`'s blames were looked up by.
    fn reasons(&mut self, order: &WorkOrder, why: &WhyLines) -> Reasons {
        let mut reasons = Reasons::new();
        for shown in &self.shown {
            let key = (shown.item, shown.span);
            let Some(item) = order.items.get(shown.item) else {
                continue;
            };
            let lines = choose(why, key, item);
            if !lines.is_empty() {
                self.why.push((key, lines.len()));
                reasons.insert(key, lines);
            }
        }
        reasons
    }

    /// Takes one thing out to make the text shorter: the last why-line of the last span that has
    /// any written, else the experience hint, else half of the last code shown, while it is more
    /// than [`MIN_SHOWN_LINES`] lines, else that code, else the last fact, else the last item not
    /// shown, else the last item after the first; `false` when there is nothing left to take.
    fn shed(&mut self) -> bool {
        // Why-lines go first: they explain code, and are worth nothing without it.
        if let Some((_, written)) = self.why.last_mut() {
            *written -= 1;
            if *written == 0 {
                self.why.pop();
            }
            return true;
        }
        // The hint goes before code: it is about earlier work, the code about this task.
        if self.is_hinted {
            self.is_hinted = false;
            return true;
        }
        if let Some(last) = self.shown.last_mut() {
            let (start, count) = (
                last.lines.start_line(),
                last.lines.end_line() - last.lines.start_line() + 1,
            );
            let kept = (count / 2).max(MIN_SHOWN_LINES);
            if kept < count
                && let Ok(cut) = LineRange::new(start, start + kept - 1)
            {
                last.lines = cut;
                return true;
            }
            self.shown.pop();
            return true;
        }
        if self.facts.pop().is_some() {
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

    /// The text this plan writes of `order`, its code from `files`, each span's why-lines from
    /// `reasons` while they are written.
    fn write(&self, order: &WorkOrder, files: &[OnDisk], reasons: &Reasons) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "{}", heading(&order.basis));
        let is_shown = |at: usize| self.shown.iter().any(|shown| shown.item == at);
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
            for shown in &self.shown {
                let (Some(item), Some(file)) = (
                    order.items.get(shown.item),
                    files.get(shown.item).and_then(OnDisk::checked),
                ) else {
                    continue;
                };
                let Some(span) = item.spans.get(shown.span) else {
                    continue;
                };
                write_code(&mut text, (item, span, shown.lines), file);
                let key = (shown.item, shown.span);
                let written = self
                    .why
                    .iter()
                    .find(|(span, _)| *span == key)
                    .map_or(0, |(_, written)| *written);
                for line in reasons.get(&key).into_iter().flatten().take(written) {
                    let _ = writeln!(text, "{line}");
                }
            }
        }
        write_facts(&mut text, &order.facts, &self.facts);
        if self.is_hinted
            && let Some(hint) = &order.hint
        {
            text.push_str(hint);
        }
        text
    }
}

/// The header line for `basis`.
fn heading(basis: &Basis) -> String {
    match basis {
        Basis::Continuity => "[cortex] Work order · continuing the work under way".to_owned(),
        Basis::Learned => "[cortex] Work order · learned from earlier work here".to_owned(),
        Basis::Agreement(kinds) => format!("[cortex] Work order · {}", agreed(kinds)),
        Basis::Located { kinds, verdict } => {
            let gate = match verdict {
                Verdict::Inject(Rule::Agreement | Rule::Calibrated) => "the gate would give it",
                Verdict::Inject(Rule::Continuity) => {
                    "the gate would give it, the work under way going on"
                }
                Verdict::Silent(Shut::TooFewKinds) => {
                    "the gate would stay silent: too few kinds of evidence agree"
                }
                Verdict::Silent(Shut::WeakMatch) => {
                    "the gate would stay silent: the words match no file strongly"
                }
                Verdict::Silent(Shut::Unlikely) => {
                    "the gate would stay silent: the confidence model puts the chance these files \
                     are used below its bar"
                }
                Verdict::Silent(Shut::OutOfTime) => {
                    // What a spreading cut short had found is the clock's doing: none of it is said.
                    return "[cortex] Where · the gate would stay silent: the spreading ran out of \
                            time, so no file is named"
                        .to_owned();
                }
            };
            let found = match kinds.as_slice() {
                [] => "no kind of evidence points here".to_owned(),
                [only] => format!("{only} alone"),
                more => agreed(more),
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
/// themselves, and none of an agent's reminder tags.
fn is_shown_as_it_is(item: &Item) -> bool {
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

/// Writes `span` of `item`'s file, of which the lines `shown`, as `file` holds them now, each
/// numbered: under the first line of the definition it starts inside, when it has one that can be
/// shown, with a line marking the lines between; and, when it was cut, a line saying which of its
/// lines are not shown.
fn write_code(
    text: &mut String,
    (item, span, shown): (&Item, &ItemSpan, LineRange),
    file: &CheckedFile,
) {
    let Some(lines) = file.lines(shown) else {
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
    let scope = span
        .scope
        .filter(|line| *line < span.lines.start_line())
        .and_then(|line| LineRange::new(line, line).ok())
        .filter(|line| !is_reminder(file, *line))
        .and_then(|line| file.lines(line));
    if let Some((number, line)) = scope.as_deref().and_then(<[(u32, &str)]>::first) {
        let _ = writeln!(text, "  {number:>width$} | {}", clean(line));
        if number + 1 < span.lines.start_line() {
            let _ = writeln!(text, "  {:>width$} | ...", "");
        }
    }
    for (number, line) in lines {
        let _ = writeln!(text, "  {number:>width$} | {}", clean(line));
    }
    if shown.end_line() < span.lines.end_line() {
        let _ = writeln!(
            text,
            "  ... lines {}-{} not shown",
            shown.end_line() + 1,
            span.lines.end_line()
        );
    }
}

/// Whether the lines `range` of `file`, as the work order would show them, hold one of an agent's
/// reminder tags: looked for once the characters taken out are gone, which may join a tag.
fn is_reminder(file: &CheckedFile, range: LineRange) -> bool {
    // INVARIANT: true-at-render. A line the agent would get altered is not shown.
    file.lines(range).is_some_and(|lines| {
        lines
            .iter()
            .any(|(_, line)| holds_reminder_tag(&clean(line)))
    })
}

/// `text` without the characters [`is_taken_out`] names: what the agent reads is what the file
/// holds, less what it could not see or what would steer its terminal.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|character| !is_taken_out(*character))
        .collect()
}
