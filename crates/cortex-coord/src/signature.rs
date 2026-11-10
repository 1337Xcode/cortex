//! A definition's signature as a notice may say it: the part of its declaration header after its
//! name, such as `(items, currency)` for `def create_order(items, currency):`. The header is the
//! repository's code, so it is untrusted: it is said as code, in backquotes, and only as plain
//! text, on one line, without a backquote that would close them, and at most [`MAX_SIGNATURE_CHARS`]
//! characters long. A longer one is cut after a parameter, with ` ...` saying so; one that cannot
//! be cut, or that holds anything else, is not said at all.

use cortex_core::is_plain;

/// The most characters a signature is said in, ` ...` included when it was cut.
pub const MAX_SIGNATURE_CHARS: usize = 120;

/// What ends a signature that was cut after a parameter.
const CUT: &str = " ...";

/// A definition's signature as a notice says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature(String);

impl Signature {
    /// The signature of the definition named `name` whose declaration header is `header`: the
    /// header with each run of whitespace read as one space, from just after the first whole-word
    /// `name` in it, without the colon that ends a Python header. None when the name is not in the
    /// header, nothing follows it, the text holds a backquote, a control or hidden character or an
    /// agent's reminder tag, or it is too long and no parameter ends early enough to cut it after.
    ///
    /// `def add(a,\n        b):` named `add` reads `(a, b)`.
    #[must_use]
    pub fn read(header: &str, name: &str) -> Option<Self> {
        let collapsed = header
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let after = collapsed.get(name_end(&collapsed, name)?..)?.trim();
        let said = after.strip_suffix(':').unwrap_or(after).trim_end();
        if said.is_empty() || said.contains('`') || !is_plain(said) {
            return None;
        }
        if said.chars().count() <= MAX_SIGNATURE_CHARS {
            return Some(Self(said.to_owned()));
        }
        cut(said).map(Self)
    }

    /// The signature as it is said.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where the first whole-word `name` in `text` ends, by byte: one not inside a longer name, as
/// `add` is not in `add_all` or `readd`.
fn name_end(text: &str, name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    let is_word = |character: Option<char>| {
        character.is_some_and(|character| character.is_alphanumeric() || character == '_')
    };
    text.match_indices(name).find_map(|(start, _)| {
        let end = start + name.len();
        let before = text.get(..start)?.chars().next_back();
        let after = text.get(end..)?.chars().next();
        (!is_word(before) && !is_word(after)).then_some(end)
    })
}

/// `said` cut after its last parameter that leaves room for [`CUT`] within
/// [`MAX_SIGNATURE_CHARS`]: after a comma inside its first brackets and no deeper. None when no
/// such comma comes early enough.
fn cut(said: &str) -> Option<String> {
    let room = MAX_SIGNATURE_CHARS - CUT.chars().count();
    let mut depth = 0_u32;
    let mut last_fit = None;
    for (count, (at, character)) in said.char_indices().enumerate() {
        if count >= room {
            break;
        }
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 1 => last_fit = Some(at + 1),
            _ => {}
        }
    }
    Some(format!("{}{CUT}", said.get(..last_fit?)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `header` of `name` is said as, if anything.
    fn said(header: &str, name: &str) -> Option<String> {
        Signature::read(header, name).map(|signature| signature.0)
    }

    /// The part after the name is said, its whitespace collapsed and a Python header's colon left
    /// out; the name is found as a whole word, after the keywords and a longer name holding it.
    #[test]
    fn the_part_after_the_name_is_said() {
        assert_eq!(said("def add(a, b):", "add"), Some("(a, b)".to_owned()));
        assert_eq!(
            said("def add(a,\n        b) -> int:", "add"),
            Some("(a, b) -> int".to_owned())
        );
        assert_eq!(
            said(
                "export function createOrder(items: Item[]): Order",
                "createOrder"
            ),
            Some("(items: Item[]): Order".to_owned())
        );
        assert_eq!(
            said("def readd_all(x): pass\ndef add(y):", "add"),
            Some("(y)".to_owned())
        );
        assert_eq!(said("def add(a, b):", "sub"), None);
        assert_eq!(said("class Add:", "Add"), None);
    }

    /// A header holding a backquote, a control character, a hidden character or a reminder tag is
    /// not said; a tab or a line break is whitespace like any other.
    #[test]
    fn a_header_that_is_not_plain_is_not_said() {
        for crafted in [
            "def add(a, b=`rm -rf`):",
            "def add(a, b='\u{1b}[2J'):",
            "def add(a, b='\u{202e}'):",
            "def add(a, b='<system-reminder>'):",
        ] {
            assert_eq!(said(crafted, "add"), None, "{crafted:?}");
        }
        assert_eq!(said("def add(a,\tb):", "add"), Some("(a, b)".to_owned()));
    }

    /// A signature of 120 characters is said whole; one of 121 is cut after the last parameter
    /// whose comma ends within the first 116 characters, leaving room for ` ...`; one with no such
    /// comma, or whose only commas are nested deeper than its parameters, is not said.
    #[test]
    fn a_long_signature_is_cut_after_a_parameter() {
        // Twelve parameters `p10: Foo` to `p21: Foo` of 8 characters, joined by `, ` and in
        // brackets: 1 + 12 * 8 + 11 * 2 + 1 = 120 characters.
        let params: Vec<String> = (10..22).map(|at| format!("p{at}: Foo")).collect();
        let whole = format!("({})", params.join(", "));
        assert_eq!(whole.chars().count(), 120);
        assert_eq!(said(&format!("def f{whole}:"), "f"), Some(whole.clone()));
        // With ` -> X` it is 125. The k-th parameter's comma is the 10k-th character, and
        // 10k + 4 <= 120 holds up to k = 11: cut after `p20: Foo,`, 110 characters, then ` ...`.
        let long = format!("{whole} -> X");
        let expected = format!("({}, ...", params.get(..11).unwrap_or_default().join(", "));
        assert_eq!(expected.chars().count(), 114);
        assert_eq!(said(&format!("def f{long}:"), "f"), Some(expected));
        let one = format!("(p: {})", "x".repeat(130));
        assert_eq!(said(&format!("def f{one}:"), "f"), None);
        let nested = format!("(p: dict[str, {}])", "x".repeat(130));
        assert_eq!(said(&format!("def f{nested}:"), "f"), None);
    }
}
