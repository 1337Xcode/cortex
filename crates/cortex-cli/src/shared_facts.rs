//! `.cortex/shared/facts.json`: the project rules a team shares, as `cortex export` writes them and
//! `cortex import` reads them. A fact is its words, its kind and what it is about, as `cortex
//! note --at` names it once made repository-relative: a file's repository path, that and `:` and a
//! symbol's name, or `.` for the whole repository. No ids, times, counts or hashes: the same facts
//! give the same bytes, so a team's diff shows only what changed.
//!
//! The file is a teammate's, and what it says may end up in front of an agent once a person
//! confirms it, so it is read strictly: a field Cortex does not know, another version, more than
//! [`MAX_SHARED_FACTS`] facts or [`MAX_ANCHORS`] anchors to one, a text longer than a fact may be,
//! a text or an anchor holding a control or other invisible character, an anchor that is not a
//! repository path, or a file over [`MAX_FACTS_FILE_BYTES`] refuses the whole file, and nothing of
//! it is imported. It is read and written only as a file of its own in folders of the repository's
//! own: a link anywhere on the way is refused, so that a clone cannot point Cortex at a file
//! elsewhere on the machine.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::ipc::GONE_ANCHOR;
use cortex_core::{FactKind, MAX_FACT_CHARS, RepoPath, is_plain, read_text_file};
use serde::{Deserialize, Serialize};

/// The folder in `.cortex` that a team commits.
pub(crate) const SHARED_FOLDER: &str = "shared";

/// The file's name in the shared folder.
pub(crate) const FACTS_FILE: &str = "facts.json";

/// The version of the file's form this build writes and reads.
pub(crate) const FACTS_VERSION: u32 = 1;

/// The most bytes the file may hold: a thousand facts of the longest text, each with a few
/// anchors, fit in well under half of it.
pub(crate) const MAX_FACTS_FILE_BYTES: u64 = 1_048_576;

/// The most facts the file may hold. A repository keeps at most ten facts about itself as a whole,
/// and a team that writes down more than a thousand rules about files has other trouble.
pub(crate) const MAX_SHARED_FACTS: usize = 1_000;

/// The most anchors one fact may name.
pub(crate) const MAX_ANCHORS: usize = 16;

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
pub(crate) struct SharedFact {
    /// The rule, in words.
    pub(crate) text: String,
    /// What kind of rule it is.
    pub(crate) kind: FactKind,
    /// What it is about, each as `cortex note --at` names it repository-relative.
    pub(crate) at: Vec<String>,
}

/// The file's text for `facts`, at most `most` of them: sorted by what they are about, then their
/// words and kind, the first `most` kept, written with two spaces of indent and a line break at
/// the end.
///
/// # Errors
///
/// The facts cannot be written as JSON, which plain strings and kinds always can.
pub(crate) fn facts_file(facts: &[SharedFact], most: usize) -> anyhow::Result<String> {
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
    let mut text = serde_json::to_string_pretty(&file).context("writing the shared facts")?;
    text.push('\n');
    Ok(text)
}

/// The facts `text`, a shared facts file, holds, read strictly as the module says.
///
/// # Errors
///
/// What makes the file refused, naming the fact by its place in the file when one is at fault.
pub(crate) fn read_facts_file(text: &str) -> anyhow::Result<Vec<SharedFact>> {
    let file: FactsFile = serde_json::from_str(text).context("not a shared facts file")?;
    if file.version != FACTS_VERSION {
        bail!(
            "version {} of the shared facts file; this Cortex reads version {FACTS_VERSION}",
            file.version
        );
    }
    if file.facts.len() > MAX_SHARED_FACTS {
        bail!(
            "{} facts; a shared facts file holds at most {MAX_SHARED_FACTS}",
            file.facts.len()
        );
    }
    for (at, fact) in file.facts.iter().enumerate() {
        shareable(fact).with_context(|| format!("fact {} of the file", at + 1))?;
    }
    Ok(file.facts)
}

/// The text of the shared facts file of the repository at `root`, none when there is none, read
/// only through folders of the repository's own and only from a file of its own.
///
/// # Errors
///
/// `.cortex` or its shared folder is a link, the file is a link or not a file, or it cannot be
/// read as UTF-8 text of at most [`MAX_FACTS_FILE_BYTES`].
pub(crate) fn read_shared_facts(root: &Path) -> anyhow::Result<Option<(PathBuf, String)>> {
    let cortex = root.join(".cortex");
    let shared = cortex.join(SHARED_FOLDER);
    for folder in [&cortex, &shared] {
        if fs::symlink_metadata(folder).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!(
                "{} is a link; Cortex reads shared facts only from a folder of the repository's own",
                folder.display()
            );
        }
    }
    let path = shared.join(FACTS_FILE);
    if !is_own_file(&path)? {
        return Ok(None);
    }
    let text = read_text_file(&path, MAX_FACTS_FILE_BYTES)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(text.map(|text| (path, text)))
}

/// Whether there is a file of its own at `path`: false when nothing is there.
///
/// # Errors
///
/// A link or anything but a file is there, or it cannot be looked at.
pub(crate) fn is_own_file(path: &Path) -> anyhow::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => bail!(
            "{} is a link or not a file; Cortex reads and writes shared facts only in a file of \
             its own",
            path.display()
        ),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("finding {}", path.display())),
    }
}

/// Whether `fact` is one a shared file may hold: words that fit and are plain, and between one and
/// [`MAX_ANCHORS`] anchors, each a repository path or `.`, optionally followed by `:` and a plain
/// symbol name, all of it plain.
///
/// # Errors
///
/// What is wrong with it.
pub(crate) fn shareable(fact: &SharedFact) -> anyhow::Result<()> {
    if fact.text.trim().is_empty() {
        bail!("its text is empty");
    }
    let chars = fact.text.chars().count();
    if chars > MAX_FACT_CHARS {
        bail!("its text holds {chars} characters; a fact holds at most {MAX_FACT_CHARS}");
    }
    if !is_plain(&fact.text) {
        bail!("its text holds a control character");
    }
    if fact.at.is_empty() || fact.at.len() > MAX_ANCHORS {
        bail!(
            "it names {} anchors; a fact names 1 to {MAX_ANCHORS}",
            fact.at.len()
        );
    }
    for anchor in &fact.at {
        anchor_checked(anchor).with_context(|| format!("anchor {anchor:?}"))?;
    }
    Ok(())
}

/// Whether `anchor` is `.`, or a file's repository path, optionally followed by `:` and a plain
/// symbol name of at most [`MAX_SYMBOL_CHARS`] characters.
///
/// # Errors
///
/// What is wrong with it.
fn anchor_checked(anchor: &str) -> anyhow::Result<()> {
    if anchor == "." {
        return Ok(());
    }
    if anchor == GONE_ANCHOR {
        bail!("its code is gone");
    }
    if !is_plain(anchor) {
        bail!("it holds a control or invisible character");
    }
    let (path, symbol) = match anchor.split_once(':') {
        Some((path, symbol)) => (path, Some(symbol)),
        None => (anchor, None),
    };
    let path = RepoPath::new(path).context("not a repository path")?;
    if path.is_root() {
        bail!("no file named");
    }
    if let Some(symbol) = symbol {
        let chars = symbol.chars().count();
        if symbol.is_empty() || chars > MAX_SYMBOL_CHARS || !is_plain(symbol) {
            bail!("a symbol's name is 1 to {MAX_SYMBOL_CHARS} plain characters");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_file_is_the_same_bytes_for_the_same_facts() -> anyhow::Result<()> {
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

    /// Each way a file is refused whole, with what is wrong: not JSON, a field Cortex does not
    /// know, another version, an unknown kind, a text of 241 characters or none, a control
    /// character, no anchor or seventeen, a path that leaves the repository, an absolute path, a
    /// folder alone, a path holding a right-to-left override, the name the list gives code that is
    /// gone, an empty symbol name, and 1,001 facts.
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
            let refused = read_facts_file(&text);
            let message = refused
                .as_ref()
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_default();
            assert!(message.starts_with(expected), "{message} <- {:.80}", text);
        }
    }
}
