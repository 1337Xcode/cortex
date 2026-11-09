//! The retrieval settings: how much a work order may hold, how activation spreads, and how much
//! each kind of link counts while it spreads.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value
//! outside what retrieval can use is refused, so a work order's caps can never be configured past
//! the hard caps that keep it under every agent's limit.

/// The default for `retrieval.max_tokens`: the most estimated tokens a work order holds.
pub const DEFAULT_MAX_TOKENS: u32 = 900;

/// The most `retrieval.max_tokens` may be set to: the work order's hard cap, which no setting
/// lifts.
pub const MAX_WORK_ORDER_TOKENS: u32 = 1_200;

/// The default for `retrieval.max_files`: how many files a work order lists.
pub const DEFAULT_MAX_FILES: u32 = 5;

/// The most `retrieval.max_files` may be set to: each listed file is read from disk while the
/// prompt waits, to check its lines, so the list stays short.
const MAX_FILES_LIMIT: u32 = 20;

/// The default for `retrieval.max_code_spans`: how many spans a work order shows as code; the rest
/// are listed by path and lines.
pub const DEFAULT_MAX_CODE_SPANS: u32 = 2;

/// The most `retrieval.max_code_spans` may be set to.
const MAX_CODE_SPANS_LIMIT: u32 = 5;

/// The default for `retrieval.span_max_lines`: the most lines one span covers.
pub const DEFAULT_SPAN_MAX_LINES: u32 = 60;

/// The most `retrieval.span_max_lines` may be set to.
const SPAN_MAX_LINES_LIMIT: u32 = 200;

/// The default for `retrieval.teleport`: the probability that spreading activation returns to its
/// seeds at each step, rather than following a link.
pub const DEFAULT_TELEPORT: f64 = 0.5;

/// The default for `retrieval.min_confidence`: once a confidence model is kept, a work order is
/// given only when the model puts the chance that its task uses one of its first files at this or
/// more. Small mode asks at least 0.85, whatever the setting.
pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.70;

/// How much each kind of link counts while activation spreads, before each node's links are
/// normalized: only the ratios between them matter, so each is a number from 0 to 1, and 0 leaves
/// that kind out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeWeights {
    /// `retrieval.type_weights.structural`: imports, calls, references and tests, times the
    /// reference's confidence. Default 1.0.
    pub structural: f64,
    /// `retrieval.type_weights.cochange`: files git shows changing together, times the
    /// co-change confidence. Default 0.6.
    pub cochange: f64,
    /// `retrieval.type_weights.coaccess`: code agents used together in work that held up, times
    /// the link's normalized effective weight. Default 0.8.
    pub coaccess: f64,
    /// `retrieval.type_weights.about`: a project fact to the code it is about. Default 1.0.
    pub about: f64,
    /// `retrieval.type_weights.containment`: a folder to its files, and a file to its symbols.
    /// Default 0.1.
    pub containment: f64,
}

impl Default for TypeWeights {
    /// The documented weights.
    fn default() -> Self {
        Self {
            structural: 1.0,
            cochange: 0.6,
            coaccess: 0.8,
            about: 1.0,
            containment: 0.1,
        }
    }
}

/// The settings retrieval reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetrievalConfig {
    /// `retrieval.max_tokens`: the most estimated tokens a work order holds, from 1 to
    /// [`MAX_WORK_ORDER_TOKENS`]. Default [`DEFAULT_MAX_TOKENS`]; small mode holds at most 400.
    pub max_tokens: u32,
    /// `retrieval.max_files`: how many files a work order lists, from 1 to [`MAX_FILES_LIMIT`].
    /// Default [`DEFAULT_MAX_FILES`].
    pub max_files: u32,
    /// `retrieval.max_code_spans`: how many spans it shows as code, from 0 to
    /// [`MAX_CODE_SPANS_LIMIT`]; 0 lists every span by path and lines alone. Default
    /// [`DEFAULT_MAX_CODE_SPANS`].
    pub max_code_spans: u32,
    /// `retrieval.span_max_lines`: the most lines one span covers, from 1 to
    /// [`SPAN_MAX_LINES_LIMIT`]. Default [`DEFAULT_SPAN_MAX_LINES`].
    pub span_max_lines: u32,
    /// `retrieval.teleport`: the restart probability of spreading activation, above 0 and at most
    /// 1. Default [`DEFAULT_TELEPORT`].
    pub teleport: f64,
    /// `retrieval.type_weights`: how much each kind of link counts.
    pub type_weights: TypeWeights,
    /// `retrieval.min_confidence`: the chance, from 0 to 1, the confidence model must give a work
    /// order once one is kept. Default [`DEFAULT_MIN_CONFIDENCE`]; small mode asks at least 0.85.
    pub min_confidence: f64,
}

impl Default for RetrievalConfig {
    /// Every retrieval setting at its documented default.
    fn default() -> Self {
        Self {
            max_tokens: DEFAULT_MAX_TOKENS,
            max_files: DEFAULT_MAX_FILES,
            max_code_spans: DEFAULT_MAX_CODE_SPANS,
            span_max_lines: DEFAULT_SPAN_MAX_LINES,
            teleport: DEFAULT_TELEPORT,
            type_weights: TypeWeights::default(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
        }
    }
}

impl RetrievalConfig {
    /// Sets the retrieval key `key` (written with dots, `retrieval.` included) to `value`:
    /// `Some(Ok(()))` when set, `Some(Err(expected))` with what the key accepts when the value is
    /// unusable, `None` when the key is not a retrieval key Cortex reads.
    pub(super) fn apply(
        &mut self,
        key: &str,
        value: &toml::Value,
    ) -> Option<Result<(), &'static str>> {
        let bounded = |low: u32, high: u32, expected: &'static str| {
            super::whole_number(value)
                .and_then(|number| u32::try_from(number).ok())
                .filter(|number| (low..=high).contains(number))
                .ok_or(expected)
        };
        let weight = || super::share(value).ok_or("a number from 0 to 1");
        let set = match key {
            "retrieval.max_tokens" => {
                bounded(1, MAX_WORK_ORDER_TOKENS, "a whole number from 1 to 1200")
                    .map(|tokens| self.max_tokens = tokens)
            }
            "retrieval.max_files" => bounded(1, MAX_FILES_LIMIT, "a whole number from 1 to 20")
                .map(|files| self.max_files = files),
            "retrieval.max_code_spans" => {
                bounded(0, MAX_CODE_SPANS_LIMIT, "a whole number from 0 to 5")
                    .map(|spans| self.max_code_spans = spans)
            }
            "retrieval.span_max_lines" => {
                bounded(1, SPAN_MAX_LINES_LIMIT, "a whole number from 1 to 200")
                    .map(|lines| self.span_max_lines = lines)
            }
            "retrieval.teleport" => super::factor(value)
                .ok_or("a number above 0 and at most 1")
                .map(|teleport| self.teleport = teleport),
            "retrieval.min_confidence" => super::share(value)
                .ok_or("a number from 0 to 1")
                .map(|least| self.min_confidence = least),
            "retrieval.type_weights.structural" => {
                weight().map(|weight| self.type_weights.structural = weight)
            }
            "retrieval.type_weights.cochange" => {
                weight().map(|weight| self.type_weights.cochange = weight)
            }
            "retrieval.type_weights.coaccess" => {
                weight().map(|weight| self.type_weights.coaccess = weight)
            }
            "retrieval.type_weights.about" => {
                weight().map(|weight| self.type_weights.about = weight)
            }
            "retrieval.type_weights.containment" => {
                weight().map(|weight| self.type_weights.containment = weight)
            }
            _ => return None,
        };
        Some(set)
    }
}
