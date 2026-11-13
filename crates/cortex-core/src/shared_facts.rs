//! The form of `.cortex/shared/facts.json`: the project rules a team shares, as `cortex export`
//! writes them and `cortex import` reads them. A fact is its words, its kind and what it is about,
//! as `cortex note --at` names it once made repository-relative: a file's repository path, that and
//! `:` and a symbol's name, or `.` for the whole repository. No ids, times, counts or hashes: the
//! same facts give the same bytes, so a team's diff shows only what changed.
//!
//! The file is a teammate's, and what it says may end up in front of an agent once a person
//! confirms it, so it is read strictly: a field Cortex does not know, another version, more than
//! [`MAX_SHARED_FACTS`] facts or [`MAX_FACT_ANCHORS`] anchors to one, a text longer than a fact
//! may be, a text or an anchor holding a control or other invisible character, or an anchor that
//! is not a repository path refuses the whole file, and nothing of it is imported.
//!
//! This module is the form alone, text in and facts out, so that it can be read and tested without
//! a file system. Finding the file, and refusing one reached through a link, is the command's.

use serde::{Deserialize, Serialize};

use crate::ipc::GONE_ANCHOR;
use crate::{FactKind, MAX_FACT_ANCHORS, MAX_FACT_CHARS, PathError, RepoPath, is_plain};

/// The folder in `.cortex` that a team commits.
pub const SHARED_FOLDER: &str = "shared";

/// The file's name in the shared folder.
pub const FACTS_FILE: &str = "facts.json";

/// The version of the file's form this build writes and reads.
pub const FACTS_VERSION: u32 = 1;

/// The most bytes the file may hold: a thousand facts of the longest text, each with a few
/// anchors, fit in well under half of it.
pub const MAX_FACTS_FILE_BYTES: u64 = 1_048_576;

/// The most facts the file may hold. A repository keeps at most ten facts about itself as a whole,
/// and a team that writes down more than a thousand rules about files has other trouble.
pub const MAX_SHARED_FACTS: usize = 1_000;

/// The most characters a symbol's name in an anchor may hold.
const MAX_SYMBOL_CHARS: usize = 512;

/// The file as written.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactsFile {
    /// The form's version, [`FACTS_VERSION`].
    version: u32,
    /// The facts, in the order [`facts_file`] sorts them.
    facts: Vec<SharedFact>,
}

/// One shared fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedFact {
    /// The rule, in words.
    pub text: String,
    /// What kind of rule it is.
    pub kind: FactKind,
    /// What it is about, each as `cortex note --at` names it repository-relative.
    pub at: Vec<String>,
}

/// Why a shared facts file was refused, or could not be written.
#[derive(Debug, thiserror::Error)]
pub enum SharedFactsError {
    /// The text is not JSON of the file's form, or names a field or a kind Cortex does not know.
    #[error("not a shared facts file")]
    NotTheForm(#[source] serde_json::Error),
    /// Another version of the form.
    #[error("version {found} of the shared facts file; this Cortex reads version {FACTS_VERSION}")]
    Version {
        /// The version the file says it is.
        found: u32,
    },
    /// More facts than a file may hold.
    #[error("{count} facts; a shared facts file holds at most {MAX_SHARED_FACTS}")]
    TooMany {
        /// How many the file holds.
        count: usize,
    },
    /// One fact is not one a shared file may hold.
    #[error("fact {place} of the file")]
    Fact {
        /// Its place in the file, counting from 1.
        place: usize,
        /// What is wrong with it.
        #[source]
        why: FactRefused,
    },
    /// The facts could not be written as JSON, which plain strings and kinds always can.
    #[error("writing the shared facts")]
    Writing(#[source] serde_json::Error),
}

/// Why one fact may not be in a shared file.
#[derive(Debug, thiserror::Error)]
pub enum FactRefused {
    /// Its text is nothing but white space.
    #[error("its text is empty")]
    EmptyText,
    /// Its text is longer than a fact may be.
    #[error("its text holds {chars} characters; a fact holds at most {MAX_FACT_CHARS}")]
    TooLong {
        /// How many characters it holds.
        chars: usize,
    },
    /// Its text holds a control or other invisible character.
    #[error("its text holds a control character")]
    ControlCharacter,
    /// It names no anchor, or more than a fact may.
    #[error("it names {count} anchors; a fact names 1 to {MAX_FACT_ANCHORS}")]
    AnchorCount {
        /// How many it names.
        count: usize,
    },
    /// One of its anchors is not one a fact may name.
    #[error("anchor {anchor:?}")]
    Anchor {
        /// The anchor as the file holds it.
        anchor: String,
        /// What is wrong with it.
        #[source]
        why: AnchorRefused,
    },
}

/// Why an anchor may not be named by a shared fact.
#[derive(Debug, thiserror::Error)]
pub enum AnchorRefused {
    /// It is the name a fact's list gives code that has gone.
    #[error("its code is gone")]
    Gone,
    /// It holds a control or invisible character.
    #[error("it holds a control or invisible character")]
    Hidden,
    /// Its path is not a repository path.
    #[error("not a repository path")]
    NotAPath(#[source] PathError),
    /// It names a symbol and no file.
    #[error("no file named")]
    NoFile,
    /// Its symbol's name is empty, too long or not plain.
    #[error("a symbol's name is 1 to {MAX_SYMBOL_CHARS} plain characters")]
    Symbol,
}

/// The file's text for `facts`, at most `most` of them: sorted by what they are about, then their
/// words and kind, the first `most` kept, written with two spaces of indent and a line break at
/// the end.
///
/// # Errors
///
/// [`SharedFactsError::Writing`], which plain strings and kinds never cause.
pub fn facts_file(facts: &[SharedFact], most: usize) -> Result<String, SharedFactsError> {
    let mut facts = facts.to_vec();
    for fact in &mut facts {
        fact.at.sort();
    }
    facts.sort_by(|left, right| {
        (&left.at, &left.text, left.kind.name()).cmp(&(&right.at, &right.text, right.kind.name()))
    });
    facts.truncate(most);
    let file = FactsFile {
        version: FACTS_VERSION,
        facts,
    };
    let mut text = serde_json::to_string_pretty(&file).map_err(SharedFactsError::Writing)?;
    text.push('\n');
    Ok(text)
}

/// The facts `text`, a shared facts file, holds, read strictly as the module says.
///
/// # Errors
///
/// What makes the file refused, naming the fact by its place in the file when one is at fault.
pub fn read_facts_file(text: &str) -> Result<Vec<SharedFact>, SharedFactsError> {
    let file: FactsFile = serde_json::from_str(text).map_err(SharedFactsError::NotTheForm)?;
    if file.version != FACTS_VERSION {
        return Err(SharedFactsError::Version {
            found: file.version,
        });
    }
    if file.facts.len() > MAX_SHARED_FACTS {
        return Err(SharedFactsError::TooMany {
            count: file.facts.len(),
        });
    }
    for (at, fact) in file.facts.iter().enumerate() {
        shareable(fact).map_err(|why| SharedFactsError::Fact { place: at + 1, why })?;
    }
    Ok(file.facts)
}

/// Whether `fact` is one a shared file may hold: words that fit and are plain, and between one and
/// [`MAX_FACT_ANCHORS`] anchors, each a repository path or `.`, optionally followed by `:` and a
/// plain symbol name, all of it plain.
///
/// # Errors
///
/// What is wrong with it.
pub fn shareable(fact: &SharedFact) -> Result<(), FactRefused> {
    if fact.text.trim().is_empty() {
        return Err(FactRefused::EmptyText);
    }
    let chars = fact.text.chars().count();
    if chars > MAX_FACT_CHARS {
        return Err(FactRefused::TooLong { chars });
    }
    if !is_plain(&fact.text) {
        return Err(FactRefused::ControlCharacter);
    }
    if fact.at.is_empty() || fact.at.len() > MAX_FACT_ANCHORS {
        return Err(FactRefused::AnchorCount {
            count: fact.at.len(),
        });
    }
    for anchor in &fact.at {
        anchor_checked(anchor).map_err(|why| FactRefused::Anchor {
            anchor: anchor.clone(),
            why,
        })?;
    }
    Ok(())
}

/// Whether `anchor` is `.`, or a file's repository path, optionally followed by `:` and a plain
/// symbol name of at most [`MAX_SYMBOL_CHARS`] characters.
///
/// # Errors
///
/// What is wrong with it.
fn anchor_checked(anchor: &str) -> Result<(), AnchorRefused> {
    if anchor == "." {
        return Ok(());
    }
    if anchor == GONE_ANCHOR {
        return Err(AnchorRefused::Gone);
    }
    if !is_plain(anchor) {
        return Err(AnchorRefused::Hidden);
    }
    let (path, symbol) = match anchor.split_once(':') {
        Some((path, symbol)) => (path, Some(symbol)),
        None => (anchor, None),
    };
    let path = RepoPath::new(path).map_err(AnchorRefused::NotAPath)?;
    if path.is_root() {
        return Err(AnchorRefused::NoFile);
    }
    if let Some(symbol) = symbol {
        let chars = symbol.chars().count();
        if symbol.is_empty() || chars > MAX_SYMBOL_CHARS || !is_plain(symbol) {
            return Err(AnchorRefused::Symbol);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::with_causes;

    /// A fact typed by hand.
    fn fact(text: &str, kind: FactKind, at: &[&str]) -> SharedFact {
        SharedFact {
            text: text.to_owned(),
            kind,
            at: at.iter().map(|anchor| (*anchor).to_owned()).collect(),
        }
    }

    /// The file sorts its facts by what they are about, then by their words, each fact's anchors
    /// sorted too, whatever order they come in; the text is typed out by hand, and reading it back
    /// gives the facts in that order.
    #[test]
    fn the_file_is_the_same_bytes_for_the_same_facts() -> Result<(), SharedFactsError> {
        let money = fact(
            "Money is integer cents",
            FactKind::Invariant,
            &["src/money.ts:Money"],
        );
        let whole = fact("Every change has a test", FactKind::Convention, &["."]);
        let both = fact(
            "Orders and payments change together",
            FactKind::Decision,
            &["src/pay.ts", "src/orders.ts"],
        );
        let expected = "{\n  \"version\": 1,\n  \"facts\": [\n    {\n      \"text\": \"Every change has a test\",\n      \"kind\": \"convention\",\n      \"at\": [\n        \".\"\n      ]\n    },\n    {\n      \"text\": \"Money is integer cents\",\n      \"kind\": \"invariant\",\n      \"at\": [\n        \"src/money.ts:Money\"\n      ]\n    },\n    {\n      \"text\": \"Orders and payments change together\",\n      \"kind\": \"decision\",\n      \"at\": [\n        \"src/orders.ts\",\n        \"src/pay.ts\"\n      ]\n    }\n  ]\n}\n";
        let written = facts_file(&[both.clone(), money.clone(), whole.clone()], 1_000)?;
        assert_eq!(written, expected);
        assert_eq!(
            facts_file(&[whole.clone(), money.clone(), both.clone()], 1_000)?,
            expected
        );
        let first_two = facts_file(&[both, money.clone(), whole.clone()], 2)?;
        assert_eq!(read_facts_file(&first_two)?, [whole.clone(), money.clone()]);
        let sorted_both = fact(
            "Orders and payments change together",
            FactKind::Decision,
            &["src/orders.ts", "src/pay.ts"],
        );
        assert_eq!(read_facts_file(expected)?, [whole, money, sorted_both]);
        Ok(())
    }

    /// Each way a file is refused whole, with what is wrong, in the words a person reads once the
    /// causes are joined: not JSON, a field Cortex does not know, another version, an unknown
    /// kind, a text of 241 characters or none, a control character, no anchor or seventeen, a path
    /// that leaves the repository, an absolute path, a folder alone, a path holding a right-to-left
    /// override, the name the list gives code that is gone, an empty symbol name, and 1,001 facts.
    #[test]
    fn a_file_that_is_not_strictly_right_is_refused_whole() {
        let one = |fact: &str| format!("{{\"version\": 1, \"facts\": [{fact}]}}");
        let long = "x".repeat(241);
        let many_anchors: Vec<String> = (0..17).map(|at| format!("\"f{at}.py\"")).collect();
        let many_facts: Vec<String> = (0..1_001)
            .map(|_| "{\"text\": \"t\", \"kind\": \"convention\", \"at\": [\"a.py\"]}".to_owned())
            .collect();
        let cases = [
            ("{".to_owned(), "not a shared facts file"),
            (
                "{\"version\": 1, \"facts\": [], \"owner\": \"me\"}".to_owned(),
                "not a shared facts file",
            ),
            (
                "{\"version\": 2, \"facts\": []}".to_owned(),
                "version 2 of the shared facts file; this Cortex reads version 1",
            ),
            (
                one("{\"text\": \"t\", \"kind\": \"wish\", \"at\": [\"a.py\"]}"),
                "not a shared facts file",
            ),
            (
                one(&format!(
                    "{{\"text\": \"{long}\", \"kind\": \"convention\", \"at\": [\"a.py\"]}}"
                )),
                "fact 1 of the file: its text holds 241 characters; a fact holds at most 240",
            ),
            (
                one("{\"text\": \" \", \"kind\": \"convention\", \"at\": [\"a.py\"]}"),
                "fact 1 of the file: its text is empty",
            ),
            (
                one("{\"text\": \"run\\u0007 it\", \"kind\": \"convention\", \"at\": [\"a.py\"]}"),
                "fact 1 of the file: its text holds a control character",
            ),
            (
                one("{\"text\": \"t\", \"kind\": \"convention\", \"at\": []}"),
                "fact 1 of the file: it names 0 anchors; a fact names 1 to 16",
            ),
            (
                one(&format!(
                    "{{\"text\": \"t\", \"kind\": \"convention\", \"at\": [{}]}}",
                    many_anchors.join(", ")
                )),
                "fact 1 of the file: it names 17 anchors; a fact names 1 to 16",
            ),
            (
                one("{\"text\": \"t\", \"kind\": \"convention\", \"at\": [\"../x.py\"]}"),
                "fact 1 of the file: anchor \"../x.py\": not a repository path",
            ),
            (
                one("{\"text\": \"t\", \"kind\": \"convention\", \"at\": [\"/etc/x\"]}"),
                "fact 1 of the file: anchor \"/etc/x\": not a repository path",
            ),
            (
                one("{\"text\": \"t\", \"kind\": \"convention\", \"at\": [\":Money\"]}"),
                "fact 1 of the file: anchor \":Money\": no file named",
            ),
            (
                one("{\"text\": \"t\", \"kind\": \"convention\", \"at\": [\"a\\u202e.py\"]}"),
                "fact 1 of the file: anchor \"a\\u{202e}.py\": it holds a control or invisible \
                 character",
            ),
            (
                one("{\"text\": \"t\", \"kind\": \"convention\", \"at\": [\"(gone)\"]}"),
                "fact 1 of the file: anchor \"(gone)\": its code is gone",
            ),
            (
                one("{\"text\": \"t\", \"kind\": \"convention\", \"at\": [\"a.py:\"]}"),
                "fact 1 of the file: anchor \"a.py:\": a symbol's name is 1 to 512 plain \
                 characters",
            ),
            (
                format!("{{\"version\": 1, \"facts\": [{}]}}", many_facts.join(", ")),
                "1001 facts; a shared facts file holds at most 1000",
            ),
        ];
        for (text, expected) in cases {
            let message = read_facts_file(&text)
                .err()
                .map(|error| with_causes(&error))
                .unwrap_or_default();
            assert!(message.starts_with(expected), "{message} <- {text:.80}");
        }
    }
}
