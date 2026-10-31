//! The name of a SCIP symbol's last descriptor, read by one scan of the symbol, and whether Cortex
//! stores definitions of its kind. The grammar is the SCIP schema's: a scheme, a package manager,
//! a package name and a version, each ended by a single space (a space inside one is doubled),
//! then descriptors: `name/` a namespace, `name#` a type, `name.` a term, `name(disambiguator).` a
//! method, `[name]` a type parameter, `(name)` a parameter, `name:` meta and `name!` a macro. A
//! name is made of letters, digits, `_`, `+`, `-` and `$`, or is anything between backticks, a
//! backtick in it doubled. A local symbol is `local` and an id.

use std::iter::Peekable;
use std::str::Chars;

use cortex_core::Identifier;

/// The name of `symbol`'s last descriptor, and whether it is a namespace, type, term or method,
/// the kinds of definition Cortex stores; `None` for a local symbol, one that does not follow the
/// grammar, or a name that cannot be stored.
pub(super) fn last_name(symbol: &str) -> Option<(Identifier, bool)> {
    if symbol.starts_with("local ") {
        return None;
    }
    let mut chars = symbol.chars().peekable();
    for _ in 0..4 {
        skip_field(&mut chars)?;
    }
    let mut last = None;
    while chars.peek().is_some() {
        last = Some(descriptor(&mut chars)?);
    }
    let (name, is_stored_kind) = last?;
    Some((Identifier::new(&name).ok()?, is_stored_kind))
}

/// Skips one field of the symbol's package and the space that ends it; `None` when the field is
/// empty or never ends.
fn skip_field(chars: &mut Peekable<Chars<'_>>) -> Option<()> {
    let mut is_empty = true;
    loop {
        match chars.next()? {
            ' ' if chars.next_if_eq(&' ').is_some() => {}
            ' ' => return (!is_empty).then_some(()),
            _ => {}
        }
        is_empty = false;
    }
}

/// Reads one descriptor: its name, and whether its kind is one Cortex stores definitions of.
fn descriptor(chars: &mut Peekable<Chars<'_>>) -> Option<(String, bool)> {
    if chars.next_if_eq(&'[').is_some() {
        let name = name(chars)?;
        chars.next_if_eq(&']')?;
        return Some((name, false));
    }
    if chars.next_if_eq(&'(').is_some() {
        let name = name(chars)?;
        chars.next_if_eq(&')')?;
        return Some((name, false));
    }
    let name = name(chars)?;
    match chars.next()? {
        '/' | '#' | '.' => Some((name, true)),
        ':' | '!' => Some((name, false)),
        '(' => {
            while chars
                .next_if(|character| is_name_character(*character))
                .is_some()
            {}
            chars.next_if_eq(&')')?;
            chars.next_if_eq(&'.')?;
            Some((name, true))
        }
        _ => None,
    }
}

/// Reads a name, plain or between backticks; `None` when it is empty or a backtick never closes.
fn name(chars: &mut Peekable<Chars<'_>>) -> Option<String> {
    let mut name = String::new();
    if chars.next_if_eq(&'`').is_some() {
        loop {
            match chars.next()? {
                '`' if chars.next_if_eq(&'`').is_some() => name.push('`'),
                '`' => break,
                other => name.push(other),
            }
        }
    } else {
        while let Some(character) = chars.next_if(|character| is_name_character(*character)) {
            name.push(character);
        }
    }
    (!name.is_empty()).then_some(name)
}

/// Whether `character` may be part of a plain name.
fn is_name_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '+' | '-' | '$')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each descriptor kind, read by hand from the grammar: namespaces, types, terms and methods
    /// are kinds Cortex stores; parameters, type parameters, meta and macros are not. Names
    /// between backticks keep their dots and a doubled backtick; a doubled space belongs to its
    /// field. A local symbol, a symbol without descriptors and one with a name and no kind have
    /// no name.
    #[test]
    fn the_last_descriptor_is_read() {
        let cases = [
            (
                "scip-python python shop 0.1 lib/helper().",
                Some(("helper", true)),
            ),
            (
                "scip-python python shop 0.1 `lib.models`/Order#",
                Some(("Order", true)),
            ),
            (
                "scip-go gomod example.com/shop v1 orders/Place().",
                Some(("Place", true)),
            ),
            (
                "rust-analyzer cargo shop 0.1 models/LIMIT.",
                Some(("LIMIT", true)),
            ),
            (
                "scip-java maven shop 0.1 m/Run#run(+1).",
                Some(("run", true)),
            ),
            (
                "scip-java maven shop 0.1 m/run().(run)",
                Some(("run", false)),
            ),
            (
                "scip-typescript npm pkg 1.0 src/Box#[T]",
                Some(("T", false)),
            ),
            ("scip-rust cargo shop 0.1 vec!", Some(("vec", false))),
            (
                "scip-java maven shop 0.1 m/Run#field:",
                Some(("field", false)),
            ),
            ("my  scheme . . . x.", Some(("x", true))),
            ("a b c d `we``ird`#", Some(("we`ird", true))),
            ("local 12", None),
            ("a b c d", None),
            ("a b c d x", None),
            ("a b c d `open", None),
            ("a  b", None),
        ];
        for (symbol, expected) in cases {
            let found = last_name(symbol);
            let found = found.as_ref().map(|(name, kind)| (name.as_str(), *kind));
            assert_eq!(found, expected, "{symbol}");
        }
    }
}
