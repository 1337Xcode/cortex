//! The names a file uses as the index records them: each with its kind, its line and the
//! definition it sits in, and what it is reached through, if anything is written before it.

use cortex_core::{Identifier, MAX_IDENTIFIER_BYTES};
use cortex_store::{RefRecord, SymbolRecord};
use tree_sitter::Node;

use crate::lines::line_number;
use crate::name_text::name_text;
use crate::tags::{Role, Tag};

/// What a use of a name is reached through: what is written before it in a qualifying form of its
/// language, such as `orders` in `orders.Place()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Qualifier {
    /// Nothing: the name stands alone, as in `place()` or `Order`.
    None,
    /// The name or expression written before it, as text: `orders` in `orders.Place()`,
    /// `shop.models` in `shop.models.Order()`, `x.iter()` in `x.iter().map(f)`.
    Name(Identifier),
    /// Something written before it whose text is not a name Cortex can store, such as an
    /// expression spanning lines.
    Other,
}

/// The references among `tags`, which are in the order their names appear, as records, each tied
/// to the smallest of `symbols` around its name, with what each is reached through as
/// `qualified_by`, its language's qualifying forms, reads it, in the same order. `symbols` are in
/// the order of their starts, each after the one around it.
pub(crate) fn build_references(
    tags: &[Tag<'_>],
    source: &[u8],
    symbols: &[SymbolRecord],
    qualified_by: &[(&str, &str)],
) -> (Vec<RefRecord>, Vec<Qualifier>) {
    let mut refs = Vec::new();
    let mut qualifiers = Vec::new();
    // The positions of the symbols that start before the current name and may hold it, outer
    // before inner, and of the next symbol to consider.
    let mut around: Vec<usize> = Vec::new();
    let mut next = 0;
    for tag in tags {
        let Role::Reference(kind) = tag.role else {
            continue;
        };
        let (start, end) = (tag.name.start_byte() as u64, tag.name.end_byte() as u64);
        while let Some(symbol) = symbols.get(next)
            && symbol.start_byte <= start
        {
            around.push(next);
            next += 1;
        }
        leave(&mut around, symbols, start, end);
        let Some(dst_name) = tag
            .name_text(source)
            .and_then(|text| Identifier::new(&text).ok())
        else {
            continue;
        };
        let Some(line) = line_number(tag.name.start_position().row) else {
            continue;
        };
        refs.push(RefRecord {
            symbol: around.last().copied(),
            dst_name,
            kind,
            line,
        });
        qualifiers.push(qualifier_of(tag.name, qualified_by, source));
    }
    (refs, qualifiers)
}

/// Bytes of source past which the part written before a name is taken as no name without reading
/// it. A name is at most [`MAX_IDENTIFIER_BYTES`] bytes, and the comments and spacing between its
/// tokens rarely come near three times as much again; reading a longer part would cost time for
/// every name of a long chain of calls, each reached through all the calls before it.
const MAX_QUALIFIER_SOURCE_BYTES: usize = 4 * MAX_IDENTIFIER_BYTES;

/// What the name at `name` is reached through: the part written before it in the qualifying form,
/// among `qualified_by`, that holds it.
fn qualifier_of(name: Node<'_>, qualified_by: &[(&str, &str)], source: &[u8]) -> Qualifier {
    let Some(holder) = name.parent() else {
        return Qualifier::None;
    };
    let before = qualified_by
        .iter()
        .filter(|(kind, _)| holder.kind() == *kind)
        .filter_map(|(_, field)| holder.child_by_field_name(field))
        .find(|part| part.end_byte() <= name.start_byte());
    match before {
        None => Qualifier::None,
        Some(part) if part.end_byte() - part.start_byte() > MAX_QUALIFIER_SOURCE_BYTES => {
            Qualifier::Other
        }
        Some(part) => match name_text(part, source).and_then(|text| Identifier::new(&text).ok()) {
            Some(text) => Qualifier::Name(text),
            None => Qualifier::Other,
        },
    }
}

/// Drops from the end of `around` the symbols that do not hold the bytes from `start` to `end`,
/// leaving the smallest symbol that does on top. Every symbol in `around` starts before the name,
/// and names come in the order of their starts, so a symbol dropped here holds no later name.
fn leave(around: &mut Vec<usize>, symbols: &[SymbolRecord], start: u64, end: u64) {
    while let Some(&innermost) = around.last() {
        let holds = symbols
            .get(innermost)
            .is_some_and(|symbol| symbol.start_byte <= start && end <= symbol.end_byte);
        if holds {
            break;
        }
        around.pop();
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::languages::TagsSpec;
    use crate::test_support::{PLAIN_IMPORTS, python_spec, with_tags};

    /// A reference's name its pattern marks as prose is recorded as written; any other is
    /// recorded token by token.
    #[test]
    fn prose_names_of_references_read_as_written() -> Result<(), Box<dyn Error>> {
        // In `a . b()`, the callee is an attribute whose tokens are `a`, `.` and `b`.
        static PROSE: TagsSpec = python_spec(
            &["(call function: (attribute) @name.prose) @reference.call"],
            PLAIN_IMPORTS,
        );
        static TOKENS: TagsSpec = python_spec(
            &["(call function: (attribute) @name) @reference.call"],
            PLAIN_IMPORTS,
        );
        let source = "a . b()\n";
        let names = |spec: &'static TagsSpec| {
            with_tags(spec, source, |tags| {
                build_references(tags, source.as_bytes(), &[], &[])
                    .0
                    .into_iter()
                    .map(|reference| reference.dst_name.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(names(&PROSE)?, ["a . b"]);
        assert_eq!(names(&TOKENS)?, ["a.b"]);
        Ok(())
    }

    /// A name is qualified by the part a qualifying form writes before it: in `a.b()` the name
    /// `b` is reached through `a`, while `a`, which is that part itself, is reached through
    /// nothing. Without the form listed, nothing qualifies anything.
    #[test]
    fn a_name_is_qualified_by_what_is_written_before_it() -> Result<(), Box<dyn Error>> {
        static ATTRIBUTE: TagsSpec = python_spec(
            &["(call function: (attribute attribute: (identifier) @name)) @reference.call"],
            PLAIN_IMPORTS,
        );
        static OBJECT: TagsSpec = python_spec(
            &["(call function: (attribute object: (identifier) @name)) @reference.call"],
            PLAIN_IMPORTS,
        );
        let source = "a.b()\n";
        let qualifiers = |spec: &'static TagsSpec, forms: &'static [(&str, &str)]| {
            with_tags(spec, source, |tags| {
                build_references(tags, source.as_bytes(), &[], forms).1
            })
        };
        let attribute: &[(&str, &str)] = &[("attribute", "object")];
        let a = Identifier::new("a")?;
        assert_eq!(qualifiers(&ATTRIBUTE, attribute)?, [Qualifier::Name(a)]);
        assert_eq!(qualifiers(&OBJECT, attribute)?, [Qualifier::None]);
        assert_eq!(qualifiers(&ATTRIBUTE, &[])?, [Qualifier::None]);
        Ok(())
    }
}
