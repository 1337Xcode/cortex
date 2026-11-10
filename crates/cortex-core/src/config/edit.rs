//! Writing one key's value into a configuration file's text by changing or adding one line, so
//! that the file's comments, order and spacing stay as the user wrote them.
//!
//! The workspace's TOML crate reads and writes values but keeps no comments, so the text is edited
//! a line at a time: the line that sets the key gets the new value (its trailing comment kept); a
//! key the file does not set yet goes under the nearest table header that holds it, else in a new
//! table at the end, else after the file's top-level keys (as a dotted key, written with its
//! table's name). Every candidate text is parsed again and kept only when it says exactly what the
//! file said with the one key changed; when none does, as when the key's value spans several
//! lines, the file is refused, never rewritten another way.

use std::collections::BTreeMap;

use super::set::SetError;
use super::{ConfigLayer, flatten, parsed};

/// The table a line belongs to while the text is read from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Table {
    /// A table this module can name: the empty name for the top of the file.
    Named(String),
    /// An array of tables, or a table with a name written in quotes: no key there is changed.
    Other,
}

/// How the lines a change adds are written.
struct Written<'a> {
    /// The value, as TOML writes it.
    rendered: &'a str,
    /// The file's line ending.
    ending: &'a str,
    /// A line of text to put above an added line as a comment.
    note: Option<&'a str>,
}

impl Written<'_> {
    /// The lines that set `name` (a key relative to its table), the note's comment first, each
    /// ended.
    fn block(&self, name: &str) -> String {
        self.comment() + &self.line(name)
    }

    /// The line that sets `name`, ended.
    fn line(&self, name: &str) -> String {
        [name, " = ", self.rendered, self.ending].concat()
    }

    /// The note's comment line, ended; empty without a note.
    fn comment(&self) -> String {
        self.note
            .map(|note| format!("# {note}{}", self.ending))
            .unwrap_or_default()
    }
}

/// The text of `file` with `key` set to `value`, `note` written as a comment above the line when
/// the line is added rather than changed.
///
/// # Errors
///
/// The file is not TOML, or no single-line change gives a text that says exactly what the file
/// said with `key` set to `value`.
pub(super) fn placed(
    file: &ConfigLayer<'_>,
    key: &'static str,
    value: &toml::Value,
    note: Option<&str>,
) -> Result<String, SetError> {
    let mut expected = entries(&parsed(file)?);
    expected.insert(key.to_owned(), value.clone());
    let lines: Vec<&str> = file.text.split_inclusive('\n').collect();
    let rendered = value.to_string();
    let written = Written {
        rendered: &rendered,
        ending: if file.text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        },
        note,
    };
    let matching = lines_setting(&lines, key);
    let candidates = if matching.is_empty() {
        additions(&lines, key, &written)
    } else {
        matching
            .into_iter()
            .filter_map(|index| replaced(&lines, index, &rendered))
            .collect()
    };
    candidates
        .into_iter()
        .find(|text| says(text, file.origin, &expected))
        .ok_or_else(|| SetError::CannotPlace {
            origin: file.origin.to_owned(),
            key,
        })
}

/// Each value `table` holds, by its key written with dots.
fn entries(table: &toml::Table) -> BTreeMap<String, toml::Value> {
    let mut flat = Vec::new();
    flatten("", table, &mut flat);
    flat.into_iter()
        .map(|(key, value)| (key, value.clone()))
        .collect()
}

/// Whether `text` is TOML holding exactly the values `expected`.
fn says(text: &str, origin: &str, expected: &BTreeMap<String, toml::Value>) -> bool {
    parsed(&ConfigLayer { origin, text }).is_ok_and(|table| entries(&table) == *expected)
}

/// The positions of the lines that set `key`.
fn lines_setting(lines: &[&str], key: &str) -> Vec<usize> {
    let mut table = Table::Named(String::new());
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some(header) = header_of(line) {
            table = header;
            continue;
        }
        let Table::Named(prefix) = &table else {
            continue;
        };
        if key_of(line).is_some_and(|(name, _)| joined(prefix, &name) == key) {
            found.push(index);
        }
    }
    found
}

/// The texts that add `key` as `written` says to a file whose `lines` do not set it, in the order
/// they are tried: under the longest table header that holds the key, in a new table at the end,
/// after the file's top-level keys (at its start when it has none) as a dotted key.
fn additions(lines: &[&str], key: &str, written: &Written<'_>) -> Vec<String> {
    let ending = written.ending;
    let mut texts = Vec::new();
    if let Some((index, rest)) = holding_header(lines, key) {
        texts.push(inserted(lines, index + 1, &written.block(&rest), ending));
    }
    if let Some((section, leaf)) = key.rsplit_once('.') {
        let mut text = lines.concat();
        if !text.is_empty() {
            if !text.ends_with('\n') {
                text.push_str(ending);
            }
            text.push_str(ending);
        }
        text.push_str(&written.comment());
        for piece in ["[", section, "]", ending] {
            text.push_str(piece);
        }
        text.push_str(&written.line(leaf));
        texts.push(text);
    }
    let top = lines
        .iter()
        .position(|line| header_of(line).is_some())
        .unwrap_or(lines.len());
    let after_keys = lines
        .iter()
        .take(top)
        .rposition(|line| key_of(line).is_some())
        .map_or(0, |index| index + 1);
    texts.push(inserted(lines, after_keys, &written.block(key), ending));
    texts
}

/// The position of the last line of the longest table header whose table holds `key`, with the
/// rest of the key after the table's name.
fn holding_header(lines: &[&str], key: &str) -> Option<(usize, String)> {
    let mut best: Option<(usize, String)> = None;
    for (index, line) in lines.iter().enumerate() {
        let Some(Table::Named(name)) = header_of(line) else {
            continue;
        };
        let Some(rest) = key
            .strip_prefix(name.as_str())
            .and_then(|rest| rest.strip_prefix('.'))
        else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(_, kept)| rest.len() < kept.len())
        {
            best = Some((index, rest.to_owned()));
        }
    }
    best
}

/// `lines` with `block`, whole lines each ended, inserted at position `index`, the line before it
/// ended first, in `ending`, if it was the file's last and had none.
fn inserted(lines: &[&str], index: usize, block: &str, ending: &str) -> String {
    let mut text = String::new();
    for (position, each) in lines.iter().enumerate() {
        if position == index {
            text.push_str(block);
        }
        text.push_str(each);
    }
    if index >= lines.len() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push_str(ending);
        }
        text.push_str(block);
    }
    text
}

/// `lines` with the value of the key line at `index` replaced by `rendered`, the spacing around it
/// and a trailing comment kept; `None` when the value does not end on its line.
fn replaced(lines: &[&str], index: usize, rendered: &str) -> Option<String> {
    let line = lines.get(index)?;
    let (_, equals) = key_of(line)?;
    let (before, after) = line.split_at(equals + 1);
    let body = after.trim_start();
    let lead = after.get(..after.len() - body.len())?;
    let value_end = value_end(body)?;
    let tail = body.get(value_end..)?;
    let mut text = String::new();
    for (position, each) in lines.iter().enumerate() {
        if position == index {
            text.push_str(before);
            text.push_str(lead);
            text.push_str(rendered);
            text.push_str(tail);
        } else {
            text.push_str(each);
        }
    }
    Some(text)
}

/// Where the value at the start of `body`, the rest of a key line after its `=` and spacing, ends:
/// the first place, before a `#` or at the line's end, where the text before it parses as a value.
fn value_end(body: &str) -> Option<usize> {
    let content = body.trim_end_matches(['\r', '\n']);
    let ends = content
        .match_indices('#')
        .map(|(index, _)| index)
        .chain([content.len()]);
    for end in ends {
        let value = content.get(..end)?.trim_end();
        if !value.is_empty() && format!("v = {value}").parse::<toml::Table>().is_ok() {
            return Some(value.len());
        }
    }
    None
}

/// The table a header line opens, or `None` when `line` is not a header.
fn header_of(line: &str) -> Option<Table> {
    let trimmed = line.trim();
    if trimmed.starts_with("[[") {
        return Some(Table::Other);
    }
    let inside = trimmed.strip_prefix('[')?;
    let (name, after) = inside.split_once(']')?;
    let after = after.trim_start();
    if !(after.is_empty() || after.starts_with('#')) {
        return None;
    }
    Some(dotted_name(name).map_or(Table::Other, Table::Named))
}

/// The key a key line sets, written with dots and relative to its table, and the position of its
/// `=`; `None` when `line` sets no key with bare names.
fn key_of(line: &str) -> Option<(String, usize)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with(['#', '[']) {
        return None;
    }
    let equals = line.find('=')?;
    let name = dotted_name(line.get(..equals)?)?;
    Some((name, equals))
}

/// `name`, a key or table name written with dots, spaces around them dropped; `None` when a part
/// is not a bare name (quoted, empty or holding other characters).
fn dotted_name(name: &str) -> Option<String> {
    let parts: Option<Vec<&str>> = name
        .split('.')
        .map(|part| {
            let part = part.trim();
            let is_bare = !part.is_empty()
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character));
            is_bare.then_some(part)
        })
        .collect();
    parts.map(|parts| parts.join("."))
}

/// `name` within the table `prefix`, written with dots.
fn joined(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `key` set to the TOML value `value`, written as a file named `f`.
    fn set(text: &str, key: &'static str, value: &str) -> Result<String, SetError> {
        let value: toml::Value = value.parse().map_err(|_| SetError::CannotPlace {
            origin: "test".to_owned(),
            key,
        })?;
        placed(&ConfigLayer { origin: "f", text }, key, &value, None)
    }

    /// A note goes as a comment above the line a change adds, or above the new table holding it,
    /// in the file's line ending; a line that is changed in place gets none.
    #[test]
    fn a_note_goes_above_an_added_line() -> Result<(), SetError> {
        let noted = |text: &str, key: &'static str, value: f64| {
            let file = ConfigLayer { origin: "f", text };
            placed(
                &file,
                key,
                &toml::Value::Float(value),
                Some("Set by a test."),
            )
        };
        assert_eq!(
            noted("mode = \"small\"", "holdout.share", 0.25)?,
            "mode = \"small\"\n\n# Set by a test.\n[holdout]\nshare = 0.25\n"
        );
        assert_eq!(
            noted("[holdout]\r\n", "holdout.share", 0.25)?,
            "[holdout]\r\n# Set by a test.\r\nshare = 0.25\r\n"
        );
        assert_eq!(
            noted("[holdout]\nshare = 0.5 # half\n", "holdout.share", 0.25)?,
            "[holdout]\nshare = 0.25 # half\n"
        );
        Ok(())
    }

    /// A key the file sets gets its new value on its own line, its comment and spacing kept, and
    /// nothing else changes.
    #[test]
    fn a_set_key_is_changed_in_place() -> Result<(), SetError> {
        let text = "# Mine.\n[retrieval]\nmax_files   =  5 # five is plenty\nmax_tokens = 900\n";
        assert_eq!(
            set(text, "retrieval.max_files", "7")?,
            "# Mine.\n[retrieval]\nmax_files   =  7 # five is plenty\nmax_tokens = 900\n"
        );
        assert_eq!(
            set("mode = \"a#b\" # c\n", "mode", "\"small\"")?,
            "mode = \"small\" # c\n"
        );
        assert_eq!(
            set("retrieval.max_files = 5\n", "retrieval.max_files", "9")?,
            "retrieval.max_files = 9\n"
        );
        assert_eq!(
            set(
                "[hints]\r\nnarrow_tests = false\r\n",
                "hints.narrow_tests",
                "true"
            )?,
            "[hints]\r\nnarrow_tests = true\r\n"
        );
        Ok(())
    }

    /// A key the file does not set goes under the longest header holding it, else into a new table
    /// at the end, and a top-level key after the file's top-level keys; comments stay.
    #[test]
    fn a_new_key_is_added_where_it_belongs() -> Result<(), SetError> {
        assert_eq!(
            set(
                "[retrieval] # caps\nmax_files = 5\n",
                "retrieval.max_tokens",
                "800"
            )?,
            "[retrieval] # caps\nmax_tokens = 800\nmax_files = 5\n"
        );
        assert_eq!(
            set(
                "[retrieval]\nmax_files = 5\n",
                "retrieval.type_weights.about",
                "0.5"
            )?,
            "[retrieval]\ntype_weights.about = 0.5\nmax_files = 5\n"
        );
        assert_eq!(
            set(
                "# Holdout.\n[holdout]\nshare = 0",
                "hints.narrow_tests",
                "true"
            )?,
            "# Holdout.\n[holdout]\nshare = 0\n\n[hints]\nnarrow_tests = true\n"
        );
        assert_eq!(
            set(
                "strict_mode = true\n# Holdout.\n[holdout]\nshare = 0\n",
                "mode",
                "\"small\""
            )?,
            "strict_mode = true\nmode = \"small\"\n# Holdout.\n[holdout]\nshare = 0\n"
        );
        assert_eq!(set("", "profile", "\"lean\"")?, "profile = \"lean\"\n");
        assert_eq!(
            set("", "hints.narrow_tests", "true")?,
            "[hints]\nnarrow_tests = true\n"
        );
        Ok(())
    }

    /// A table defined by dotted keys gets the new key as a dotted key too, since a header for it
    /// would define it twice.
    #[test]
    fn a_table_of_dotted_keys_gets_a_dotted_key() -> Result<(), SetError> {
        assert_eq!(
            set(
                "hints.narrow_tests = true\n[x]\n",
                "hints.runtime_grounding",
                "true"
            )?,
            "hints.narrow_tests = true\nhints.runtime_grounding = true\n[x]\n"
        );
        Ok(())
    }

    /// A value that spans lines, or a file that is not TOML, is refused and nothing is written.
    #[test]
    fn what_cannot_be_placed_is_refused() {
        assert_eq!(
            set(
                "[tests]\nextra_patterns = [\n  \"a\",\n]\n",
                "tests.extra_patterns",
                "[\"b\"]"
            ),
            Err(SetError::CannotPlace {
                origin: "f".to_owned(),
                key: "tests.extra_patterns",
            })
        );
        assert!(matches!(
            set("[hints", "hints.narrow_tests", "true"),
            Err(SetError::File(_))
        ));
    }

    /// Header and key lines are read with their spacing; quoted names and arrays of tables are not
    /// named.
    #[test]
    fn lines_are_read() {
        assert_eq!(
            header_of("  [ retrieval . type_weights ]  # w\n"),
            Some(Table::Named("retrieval.type_weights".to_owned()))
        );
        assert_eq!(header_of("[[runs]]\n"), Some(Table::Other));
        assert_eq!(header_of("[\"a.b\"]\n"), Some(Table::Other));
        assert_eq!(header_of("[a] x\n"), None);
        assert_eq!(
            key_of("  max_files = 5\n"),
            Some(("max_files".to_owned(), 12))
        );
        assert_eq!(key_of("# max_files = 5\n"), None);
        assert_eq!(key_of("\"max\" = 5\n"), None);
        assert_eq!(value_end("5 # c\n"), Some(1));
        assert_eq!(value_end("[\n"), None);
    }
}
