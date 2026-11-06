//! What a prompt points at: the words a search looks up, and the paths and names it writes the way
//! code and tools write them.
//!
//! Words are the prompt's identifier tokens, split as the index splits names and paths, so that
//! `createOrder`, `create_order` and "create order" look up the same rows. Words that say what to do
//! or join a sentence rather than where (`fix`, `the`, `make`, `better`) are left out, as are digits
//! alone and single letters: they name no place, and a prompt made only of them looks nothing up.
//! So are the folders of a path the prompt names (`src` and `flask` of `src/flask/helpers.py`) and
//! its extension when it has a folder (`py` of `calc/ops.py`): the path itself is a mention, looked
//! up exactly, and as words its folders would match every file beside it and its extension every
//! file of that language, outranking the file the prompt names. A word the prompt also writes
//! outside the path stays.
//!
//! A mention is one word that may name a file or a symbol exactly. A word with a folder separator
//! is a path, with the line after it when the prompt gives one (`src/app.ts:12`, a stack frame's
//! `(/repo/src/app.ts:12:5)`, a traceback's `File "app.py", line 12`). A word written as code is a
//! name: with an underscore, a capital inside it, digits among its letters, parts joined by `::`, or
//! inside backticks. A word like `service.ts` or `Config.load` could be either, so it is both, and
//! whoever looks it up keeps whichever the repository holds.

use std::collections::BTreeMap;

use cortex_core::identifier_tokens;

/// Words that say what to do, join a sentence or judge, and name no place in a repository, sorted
/// so that a word is found by binary search.
const STOP_WORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "actually",
    "add",
    "after",
    "again",
    "all",
    "also",
    "am",
    "an",
    "and",
    "any",
    "anything",
    "are",
    "as",
    "ask",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "best",
    "better",
    "between",
    "both",
    "but",
    "by",
    "can",
    "change",
    "check",
    "clean",
    "cleaner",
    "code",
    "could",
    "did",
    "do",
    "does",
    "doing",
    "done",
    "down",
    "each",
    "else",
    "even",
    "every",
    "few",
    "file",
    "files",
    "find",
    "fine",
    "first",
    "fix",
    "for",
    "from",
    "get",
    "give",
    "go",
    "good",
    "got",
    "had",
    "has",
    "have",
    "having",
    "he",
    "help",
    "her",
    "here",
    "him",
    "his",
    "how",
    "however",
    "i",
    "if",
    "implement",
    "improve",
    "in",
    "into",
    "is",
    "it",
    "its",
    "just",
    "know",
    "let",
    "lets",
    "like",
    "line",
    "lines",
    "look",
    "make",
    "many",
    "may",
    "me",
    "might",
    "more",
    "most",
    "much",
    "must",
    "my",
    "need",
    "needs",
    "new",
    "nice",
    "nicer",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "ok",
    "okay",
    "on",
    "once",
    "one",
    "only",
    "or",
    "other",
    "our",
    "out",
    "over",
    "own",
    "please",
    "really",
    "same",
    "see",
    "should",
    "show",
    "simpler",
    "so",
    "some",
    "something",
    "stuff",
    "such",
    "sure",
    "take",
    "tell",
    "than",
    "thanks",
    "that",
    "the",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "thing",
    "things",
    "this",
    "those",
    "through",
    "to",
    "too",
    "try",
    "under",
    "until",
    "up",
    "update",
    "us",
    "use",
    "very",
    "want",
    "was",
    "way",
    "we",
    "well",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "why",
    "will",
    "with",
    "work",
    "would",
    "yes",
    "yet",
    "you",
    "your",
];

/// The longest extension a word may end in to be taken for a file name.
const MAX_EXTENSION_CHARS: usize = 10;

/// Characters trimmed from the start of a word before it is read: opening quotes and brackets. A
/// dot is not among them, so `./src` keeps its folder.
const LEADING: &[char] = &['"', '\'', '`', '(', '[', '{', '<'];

/// Characters trimmed from the end of a word before it is read: closing quotes and brackets, and
/// the punctuation that ends a clause.
const TRAILING: &[char] = &[
    '"', '\'', '`', ')', ']', '}', '>', ',', ';', '!', '?', '.', ':',
];

/// A word that may name a file or a symbol exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    /// The file it may name, with `/` between its parts and no leading `./`.
    pub path: Option<String>,
    /// The line it names in that file, when it names one.
    pub line: Option<u32>,
    /// The symbol it may name: the word, or for parts joined by `.` or `::`, the last part.
    pub name: Option<String>,
}

/// What a prompt points at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptTerms {
    /// The words a search looks up, lower case, each once, in the order they first appear.
    pub words: Vec<String>,
    /// The words that may name a file or a symbol exactly, each once, in the order they appear.
    pub mentions: Vec<Mention>,
    /// How many times the prompt writes each of `mentions`, in the same order.
    pub mention_counts: Vec<u32>,
    /// How many times the prompt writes each of `words`.
    pub word_counts: BTreeMap<String, u32>,
}

impl PromptTerms {
    /// What `prompt` points at.
    #[must_use]
    pub fn of(prompt: &str) -> Self {
        let mut terms = Self::default();
        let chunks: Vec<&str> = prompt.split_whitespace().collect();
        // The prompt's text with each path written with a folder cut to its file's name, so that
        // the folders give no words.
        let mut worded = Vec::with_capacity(chunks.len());
        for (at, chunk) in chunks.iter().enumerate() {
            let mention = mention_of(chunk, chunks.get(at + 1..).unwrap_or_default());
            let file_name = mention
                .as_ref()
                .and_then(|mention| mention.path.as_deref())
                .and_then(|path| path.rsplit_once('/'))
                .map(|(_, file)| file.to_owned());
            worded.push(file_name.unwrap_or_else(|| (*chunk).to_owned()));
            if let Some(mention) = mention {
                let held = terms.mentions.iter().position(|held| *held == mention);
                if let Some(count) = held.and_then(|at| terms.mention_counts.get_mut(at)) {
                    *count = count.saturating_add(1);
                } else {
                    terms.mentions.push(mention);
                    terms.mention_counts.push(1);
                }
            }
        }
        for token in identifier_tokens(&worded.join(" ")) {
            if !is_place_word(&token) {
                continue;
            }
            let count = terms.word_counts.entry(token.clone()).or_insert(0);
            *count = count.saturating_add(1);
            if *count == 1 {
                terms.words.push(token);
            }
        }
        let extensions: Vec<String> = terms
            .mentions
            .iter()
            .filter_map(|mention| mention.path.as_deref())
            .filter_map(folder_path_extension)
            .collect();
        terms.words.retain(|word| !extensions.contains(word));
        terms
    }

    /// The `max` words the prompt writes most (among those written as often, the first written),
    /// in the order the prompt first writes them: all of them for any prompt short of `max` words,
    /// so only a long text gives up the words it writes least.
    #[must_use]
    pub fn frequent_words(&self, max: usize) -> Vec<&str> {
        let mut ranked: Vec<(usize, u32)> = self
            .words
            .iter()
            .enumerate()
            .map(|(at, word)| (at, self.word_counts.get(word).copied().unwrap_or(1)))
            .collect();
        ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        ranked.truncate(max);
        ranked.sort_unstable_by_key(|(at, _)| *at);
        ranked
            .into_iter()
            .filter_map(|(at, _)| self.words.get(at).map(String::as_str))
            .collect()
    }

    /// The mentions the prompt writes most, at most `max`, the most written first and, among those
    /// written as often, the first written first; each with its prominence, how often it is written
    /// against the most written one, from 0 to 1. In a short prompt, which writes each once, every
    /// mention has a prominence of 1; in a long one, a name the text keeps coming back to stands
    /// above one it writes in passing.
    #[must_use]
    pub fn prominent(&self, max: usize) -> Vec<(&Mention, f64)> {
        let most = self.mention_counts.iter().copied().max().unwrap_or(0);
        let mut ranked: Vec<(usize, u32)> =
            self.mention_counts.iter().copied().enumerate().collect();
        ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        ranked
            .into_iter()
            .take(max)
            .filter_map(|(at, count)| {
                let mention = self.mentions.get(at)?;
                Some((mention, f64::from(count) / f64::from(most.max(1))))
            })
            .collect()
    }

    /// Whether it points at nothing: no word and no mention.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.mentions.is_empty()
    }
}

/// The extension of `path`, lower case, when it names a folder and a file with one: `py` of
/// `calc/ops.py`, none of `ops.py` or `src/Makefile`. A path with no folder could be a name joined
/// by a dot (`Config.load`), whose last part is a word.
fn folder_path_extension(path: &str) -> Option<String> {
    let (_, file) = path.rsplit_once('/')?;
    let (stem, extension) = file.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then(|| extension.to_ascii_lowercase())
}

/// Whether `token`, a lower-case identifier token, can name a place: not a stop word, not digits
/// alone, not one character.
fn is_place_word(token: &str) -> bool {
    token.chars().count() > 1
        && !token.chars().all(|c| c.is_ascii_digit())
        && STOP_WORDS.binary_search(&token).is_err()
}

/// What `chunk`, one whitespace-separated piece of a prompt, may name, the pieces after it being
/// `following`: a path and its line, a name, both, or nothing.
fn mention_of(chunk: &str, following: &[&str]) -> Option<Mention> {
    let inner = chunk
        .trim_start_matches(|c: char| c != '`' && LEADING.contains(&c))
        .trim_end_matches(|c: char| c != '`' && TRAILING.contains(&c));
    let is_backticked = inner.len() > 2 && inner.starts_with('`') && inner.ends_with('`');
    let word = trimmed(chunk);
    if word.is_empty() {
        return None;
    }
    let (path_text, line) = split_line(word);
    let has_separator = path_text.contains(['/', '\\']);
    let path = (has_separator || is_file_name(path_text))
        .then(|| path_text.replace('\\', "/"))
        .filter(|path| {
            path.split('/')
                .any(|part| part.chars().any(char::is_alphabetic))
        })
        .map(|path| path.trim_start_matches("./").to_owned());
    let line = path.as_ref().and(line.or_else(|| line_after(following)));
    let name = (!has_separator)
        .then(|| code_name(path_text, is_backticked))
        .flatten()
        .filter(|name| name.chars().count() > 1);
    (path.is_some() || name.is_some()).then_some(Mention {
        path,
        line: line.filter(|line| *line > 0),
        name,
    })
}

/// `word` without a line written after it, and that line: `path:12` and `path:12:5` give 12.
/// Only a colon followed by a digit starts a line, so `std::fs` is left whole, and a drive
/// letter's colon, as in `C:\repo\app.rs:12`, is part of the path.
fn split_line(word: &str) -> (&str, Option<u32>) {
    let drive = match word.as_bytes() {
        [letter, b':', ..] if letter.is_ascii_alphabetic() => 2,
        _ => 0,
    };
    let bytes = word.as_bytes();
    let colon = (drive..bytes.len()).find(|at| {
        bytes.get(*at) == Some(&b':') && bytes.get(at + 1).is_some_and(u8::is_ascii_digit)
    });
    let Some(colon) = colon else {
        return (word, None);
    };
    let after = word.get(colon + 1..).unwrap_or_default();
    let line = after
        .split(':')
        .next()
        .and_then(|number| number.parse().ok());
    (word.get(..colon).unwrap_or(word), line)
}

/// `chunk` without the quotes, brackets and punctuation around it.
fn trimmed(chunk: &str) -> &str {
    chunk.trim_start_matches(LEADING).trim_end_matches(TRAILING)
}

/// The line a traceback names after a path: `line`, then a number.
fn line_after(following: &[&str]) -> Option<u32> {
    match following {
        [said, number, ..] if trimmed(said).eq_ignore_ascii_case("line") => {
            trimmed(number).parse().ok()
        }
        _ => None,
    }
}

/// Whether `word` reads as a file's name: characters a path holds, and an extension of letters and
/// digits after a stem with a letter in it, so that `1.5` is no file.
fn is_file_name(word: &str) -> bool {
    let is_path_char = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '@' | '+');
    let Some((stem, extension)) = word.rsplit_once('.') else {
        return false;
    };
    word.chars().all(is_path_char)
        && stem.chars().any(char::is_alphabetic)
        && !extension.is_empty()
        && extension.len() <= MAX_EXTENSION_CHARS
        && extension.chars().all(|c| c.is_ascii_alphanumeric())
        && extension.chars().any(|c| c.is_ascii_alphabetic())
}

/// The name `word` writes as code, if it does: its last part when parts are joined by `.` or `::`.
/// Backticks make any identifier a name.
fn code_name(word: &str, is_backticked: bool) -> Option<String> {
    let parts: Vec<&str> = word.split("::").flat_map(|part| part.split('.')).collect();
    if parts.iter().any(|part| !is_identifier(part)) {
        return None;
    }
    let last = parts.last()?;
    let is_code = is_backticked || parts.len() > 1 || looks_like_code(last);
    is_code.then(|| (*last).to_owned())
}

/// Whether `part` is one identifier: letters, digits, `_` and `$`, not starting with a digit.
fn is_identifier(part: &str) -> bool {
    let mut chars = part.chars();
    chars
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Whether one identifier is written the way code writes names and prose does not: with an
/// underscore, a capital after a small letter or a run of capitals before a small one, or digits
/// among letters.
fn looks_like_code(name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    let has_underscore = name.contains('_');
    let has_inner_capital = chars
        .windows(2)
        .any(|pair| matches!(pair, [left, right] if left.is_lowercase() && right.is_uppercase()))
        || chars.windows(3).any(|three| {
            matches!(three, [first, second, third]
                if first.is_uppercase() && second.is_uppercase() && third.is_lowercase())
        });
    let has_digit =
        chars.iter().any(char::is_ascii_digit) && chars.iter().any(|c| c.is_alphabetic());
    has_underscore || has_inner_capital || has_digit
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mention of a path, with its line, that names nothing else.
    fn path(path: &str, line: Option<u32>) -> Mention {
        Mention {
            path: Some(path.to_owned()),
            line,
            name: None,
        }
    }

    /// A mention of a name only.
    fn name(name: &str) -> Mention {
        Mention {
            path: None,
            line: None,
            name: Some(name.to_owned()),
        }
    }

    /// How often a prompt writes each mention and word, counted by hand: `create_order` four times,
    /// `charge` once, `ship` twice. The mentions by prominence: `create_order` 4/4 = 1, `ship` 2/4 =
    /// 0.5, `charge` 1/4 = 0.25, the first two kept at most two. The words written most, at most
    /// three, in the prompt's order: `create` and `order` (four each) and `ship` (two), `charge` (one)
    /// left out; with room for all, every word in the prompt's order.
    #[test]
    fn a_long_prompt_is_read_by_what_it_writes_most() {
        let terms = PromptTerms::of(
            "`create_order` then `charge` then `create_order` `ship` `create_order` `ship` \
             `create_order`",
        );
        assert_eq!(terms.mention_counts, [4, 1, 2]);
        let prominent: Vec<(String, f64)> = terms
            .prominent(3)
            .into_iter()
            .map(|(mention, prominence)| (mention.name.clone().unwrap_or_default(), prominence))
            .collect();
        assert_eq!(
            prominent,
            [
                ("create_order".to_owned(), 1.0),
                ("ship".to_owned(), 0.5),
                ("charge".to_owned(), 0.25)
            ]
        );
        assert_eq!(terms.prominent(2).len(), 2);
        assert_eq!(terms.frequent_words(3), ["create", "order", "ship"]);
        assert_eq!(
            terms.frequent_words(10),
            ["create", "order", "charge", "ship"]
        );
    }

    /// The words and mentions of each prompt, read off it by hand. Words are its identifier tokens
    /// less the stop words, digits alone, single letters, and the folders and the extension of a
    /// path named with a folder (`calc` and `py` of `calc/ops.py`), each once; a path keeps its
    /// line; a backticked word is a
    /// name, and so is one with a capital inside, a run of capitals before a small letter, an
    /// underscore or digits among letters.
    #[test]
    fn prompts_are_read_as_worked_out() {
        let terms =
            PromptTerms::of("Fix the divide-by-zero in calc/ops.py:12 where `divide` returns None");
        assert_eq!(terms.words, ["divide", "zero", "ops", "returns", "none"]);
        assert_eq!(
            terms.mentions,
            [path("calc/ops.py", Some(12)), name("divide")]
        );

        let terms = PromptTerms::of(
            "rename createOrder in OrderService to placeOrder, and HTTPRequest too",
        );
        assert_eq!(
            terms.words,
            [
                "rename", "create", "order", "service", "place", "http", "request"
            ]
        );
        assert_eq!(
            terms.mentions,
            [
                name("createOrder"),
                name("OrderService"),
                name("placeOrder"),
                name("HTTPRequest")
            ]
        );
    }

    /// A path named with a folder gives no word for its folders, nor for its extension whatever
    /// its case, while a file with no folder keeps its extension, being perhaps a name joined by a
    /// dot: `lib/Util.RS` gives `util`, `notes.md` gives `notes` and `md`, `src/Makefile` gives
    /// `makefile`. A folder's word the prompt also writes outside the path stays: the second `flask`
    /// of the last prompt.
    #[test]
    fn a_named_path_gives_no_word_for_its_folders_or_extension() {
        assert_eq!(
            PromptTerms::of("rename helper in lib/Util.RS").words,
            ["rename", "helper", "util"]
        );
        assert_eq!(PromptTerms::of("see notes.md").words, ["notes", "md"]);
        assert_eq!(
            PromptTerms::of("why does src/Makefile fail").words,
            ["makefile", "fail"]
        );
        let flask = PromptTerms::of(
            "In src/flask/helpers.py, make get_debug_flag read 'yes' as flask does",
        );
        assert_eq!(flask.words, ["helpers", "debug", "flag", "read", "flask"]);
        assert_eq!(
            flask.mentions,
            [path("src/flask/helpers.py", None), name("get_debug_flag")]
        );
    }

    /// A word joined by `::` is a name, its last part; a word joined by `.` could name a file or a
    /// symbol, so it is both, the name its last part; digits among letters make a name.
    #[test]
    fn joined_words_may_be_names_or_files() {
        let terms = PromptTerms::of(
            "why does parse_config fail on sha256 keys? see Config.load and std::fs::read",
        );
        assert_eq!(
            terms.words,
            [
                "parse", "config", "fail", "sha256", "keys", "load", "std", "fs", "read"
            ]
        );
        assert_eq!(
            terms.mentions,
            [
                name("parse_config"),
                name("sha256"),
                Mention {
                    path: Some("Config.load".to_owned()),
                    line: None,
                    name: Some("load".to_owned()),
                },
                name("read"),
            ]
        );
    }

    /// Stack frames and tracebacks name a path and its line: a JavaScript frame, a Python
    /// traceback and a Windows path with its drive. `./` is dropped and `\` becomes `/`; a file name
    /// with an extension and no folder could also be a name.
    #[test]
    fn frames_name_paths_and_lines() {
        let terms = PromptTerms::of("at createOrder (/home/dev/shop/src/orders/service.ts:112:7)");
        assert_eq!(
            terms.mentions,
            [
                name("createOrder"),
                path("/home/dev/shop/src/orders/service.ts", Some(112)),
            ]
        );
        let terms = PromptTerms::of("File \"./app/main.py\", line 42, in handler");
        assert_eq!(terms.mentions, [path("app/main.py", Some(42))]);
        let terms = PromptTerms::of("C:\\repo\\src\\main.rs:5 panicked; see README.md");
        assert_eq!(
            terms.mentions,
            [
                path("C:/repo/src/main.rs", Some(5)),
                Mention {
                    path: Some("README.md".to_owned()),
                    line: None,
                    name: Some("md".to_owned()),
                },
            ]
        );
    }

    /// Prompts made of words that name no place point at nothing: every word is a stop word, digits
    /// alone or one letter, and none is written as code. A number with a point is neither a file
    /// nor a name.
    #[test]
    fn vague_prompts_point_at_nothing() {
        for prompt in [
            "make it better",
            "Fix it please!",
            "can you look at this and make it work?",
            "improve the code",
            "do 1.5 or 2 of these",
            "ok",
            "",
        ] {
            let terms = PromptTerms::of(prompt);
            assert!(terms.is_empty(), "{prompt:?} gave {terms:?}");
        }
    }

    /// The stop words are sorted, so that binary search finds every one, and lower case, as the
    /// tokens they are compared with are.
    #[test]
    fn the_stop_words_are_sorted_and_lower_case() {
        assert!(
            STOP_WORDS
                .windows(2)
                .all(|pair| matches!(pair, [left, right] if left < right))
        );
        assert!(
            STOP_WORDS
                .iter()
                .all(|word| word.chars().all(char::is_lowercase))
        );
        assert!(STOP_WORDS.iter().all(|word| !is_place_word(word)));
    }
}
