//! A file's definitions as the index records them: each with its kind, the definition it sits in,
//! its qualified name, its span and its hashes.
//!
//! Definitions nest by their spans: each sits in the smallest definition around it. A function
//! directly in a class is a method, and in a test file a function or method named the way the
//! language names tests is a test. A qualified name joins the names of the definitions around it
//! with `.`, in every language, with the definition's qualifier, if its pattern captured one (the
//! type a Go method belongs to), just before its name. A definition that cannot be recorded, because its name or its
//! qualified name is not a name Cortex can store, is left out with every definition inside it,
//! whose qualified names would contain it.

use std::cmp::Reverse;

use cortex_core::{Identifier, SymbolKind};
use cortex_store::SymbolRecord;
use tree_sitter::{Node, Point};

use crate::hashes::collapsed_hash;
use crate::languages::TagsSpec;
use crate::lines::line_number;
use crate::tags::{Role, Tag};

/// Where a definition is: from the start of its node or its name, whichever comes first, to the
/// end of whichever ends last.
#[derive(Debug, Clone, Copy)]
struct Span {
    /// The byte offset where it starts.
    start: usize,
    /// The byte offset just past its end.
    end: usize,
    /// Where it starts.
    first: Point,
    /// Where it ends.
    last: Point,
}

/// What a definition's record needs besides its tag.
struct Recorder<'a> {
    /// The file's contents.
    source: &'a [u8],
    /// The language's parsing rules.
    spec: &'a TagsSpec,
    /// Whether the file holds tests.
    is_test_file: bool,
    /// The records made so far.
    records: Vec<SymbolRecord>,
}

/// The definitions among `tags` as records, in the order of their starts, each after the record
/// of the definition it sits in.
pub(crate) fn build_symbols(
    tags: &[Tag<'_>],
    source: &[u8],
    spec: &TagsSpec,
    is_test_file: bool,
) -> Vec<SymbolRecord> {
    let mut definitions: Vec<(Span, &Tag<'_>, SymbolKind)> = tags
        .iter()
        .filter_map(|tag| match tag.role {
            Role::Definition(kind) => Some((span_of(tag), tag, kind)),
            Role::Reference(_) => None,
        })
        .collect();
    // Outer definitions before the ones inside them. The sort is stable and the tags come in the
    // order of their names, so definitions with equal spans keep that order.
    definitions.sort_by_key(|(span, _, _)| (span.start, Reverse(span.end)));
    let mut recorder = Recorder {
        source,
        spec,
        is_test_file,
        records: Vec::new(),
    };
    // The spans of the definitions around the current one, innermost last, each with the
    // position of its record, or `None` when it was left out.
    let mut around: Vec<(Span, Option<usize>)> = Vec::new();
    for (span, tag, kind) in definitions {
        while around
            .last()
            .is_some_and(|(outer, _)| !(outer.start <= span.start && span.end <= outer.end))
        {
            around.pop();
        }
        let position = match around.last() {
            Some((_, None)) => None,
            Some((_, Some(parent))) => recorder.record(tag, kind, span, Some(*parent)),
            None => recorder.record(tag, kind, span, None),
        };
        around.push((span, position));
    }
    recorder.records
}

impl Recorder<'_> {
    /// Records the definition `tag`, of `kind`, over `span`, inside the record at `parent`; the
    /// new record's position, or `None` when the definition cannot be recorded.
    fn record(
        &mut self,
        tag: &Tag<'_>,
        kind: SymbolKind,
        span: Span,
        parent: Option<usize>,
    ) -> Option<usize> {
        let name = Identifier::new(tag.name.utf8_text(self.source).ok()?).ok()?;
        let outer = parent.and_then(|position| self.records.get(position));
        let local = match tag.qualifier {
            Some(qualifier) => format!(
                "{}.{}",
                qualifier.utf8_text(self.source).ok()?,
                name.as_str()
            ),
            None => name.as_str().to_owned(),
        };
        let qual_name = match outer {
            Some(outer) => Identifier::new(&format!("{}.{local}", outer.qual_name.as_str())),
            None => Identifier::new(&local),
        }
        .ok()?;
        let kind = self.kind_of(kind, outer.map(|outer| outer.kind), &name);
        let header_end = header_end(tag.node, self.spec.body_paths, span.end);
        let record = SymbolRecord {
            parent,
            kind,
            name,
            qual_name,
            start_byte: span.start as u64,
            end_byte: span.end as u64,
            start_line: line_number(span.first.row)?,
            end_line: last_line(span)?,
            sig_hash: collapsed_hash(self.source.get(span.start..header_end)?),
            body_hash: collapsed_hash(self.source.get(span.start..span.end)?),
        };
        self.records.push(record);
        Some(self.records.len() - 1)
    }

    /// The kind a definition tagged `kind`, named `name`, is recorded as, when the definition
    /// around it is of kind `outer`.
    fn kind_of(
        &self,
        kind: SymbolKind,
        outer: Option<SymbolKind>,
        name: &Identifier,
    ) -> SymbolKind {
        let kind = if kind == SymbolKind::Function && outer == Some(SymbolKind::Class) {
            SymbolKind::Method
        } else {
            kind
        };
        let is_test = self.is_test_file
            && matches!(kind, SymbolKind::Function | SymbolKind::Method)
            && self
                .spec
                .test_functions
                .iter()
                .any(|pattern| pattern.matches(name.as_str()));
        if is_test { SymbolKind::Test } else { kind }
    }
}

/// The span of the definition `tag` names.
fn span_of(tag: &Tag<'_>) -> Span {
    let (node, name) = (tag.node, tag.name);
    let (start, first) = if name.start_byte() < node.start_byte() {
        (name.start_byte(), name.start_position())
    } else {
        (node.start_byte(), node.start_position())
    };
    let (end, last) = if name.end_byte() > node.end_byte() {
        (name.end_byte(), name.end_position())
    } else {
        (node.end_byte(), node.end_position())
    };
    Span {
        start,
        end,
        first,
        last,
    }
}

/// Where the header of the definition at `node` ends: at the end of the last part before its
/// body, which the first of `body_paths` that leads to one finds, so that comments between the
/// header and the body, which the grammar places beside the body, are not part of it; at
/// `span_end` when no path leads to a body.
fn header_end(node: Node<'_>, body_paths: &[&[&str]], span_end: usize) -> usize {
    let Some((holder, body)) = body_paths.iter().find_map(|path| follow(node, path)) else {
        return span_end;
    };
    let mut cursor = holder.walk();
    holder
        .children(&mut cursor)
        .take_while(|child| *child != body)
        .filter(|child| !child.is_extra())
        .last()
        .map_or(body.start_byte(), |part| part.end_byte())
}

/// The node the field names in `path` lead to from `node`, with the node that holds it.
fn follow<'tree>(node: Node<'tree>, path: &[&str]) -> Option<(Node<'tree>, Node<'tree>)> {
    let (last, steps) = path.split_last()?;
    let holder = steps
        .iter()
        .try_fold(node, |current, field| current.child_by_field_name(field))?;
    Some((holder, holder.child_by_field_name(last)?))
}

/// The 1-based line holding the span's last byte. A span that ends with a line break ends, as
/// tree-sitter places it, at the start of the next line, which holds none of it.
fn last_line(span: Span) -> Option<u32> {
    let row = if span.last.column == 0 && span.last.row > span.first.row {
        span.last.row - 1
    } else {
        span.last.row
    };
    line_number(row)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::test_support::{PLAIN_IMPORTS, python_spec, with_tags};

    /// Each symbol's qualified name, parent and byte span.
    type Found = Vec<(String, Option<usize>, u64, u64)>;

    /// The symbols `spec` finds in `source`.
    fn symbols_found(spec: &'static TagsSpec, source: &str) -> Result<Found, Box<dyn Error>> {
        with_tags(spec, source, |tags| {
            build_symbols(tags, source.as_bytes(), spec, false)
                .into_iter()
                .map(|symbol| {
                    let qual_name = symbol.qual_name.as_str().to_owned();
                    (qual_name, symbol.parent, symbol.start_byte, symbol.end_byte)
                })
                .collect()
        })
    }

    /// A span reaches from the earlier of the node and the name to the later of the two.
    #[test]
    fn spans_join_the_node_and_the_name() -> Result<(), Box<dyn Error>> {
        // In `x = f(1)`, `x` is bytes 0 to 1, `f` 4 to 5 and the call `f(1)` 4 to 8.
        static NAME_FIRST: TagsSpec = python_spec(
            &["(assignment left: (identifier) @name right: (call) @definition.function)"],
            PLAIN_IMPORTS,
        );
        static NAME_LAST: TagsSpec = python_spec(
            &[
                "(assignment left: (identifier) @definition.function right: (call function: (identifier) @name))",
            ],
            PLAIN_IMPORTS,
        );
        assert_eq!(
            symbols_found(&NAME_FIRST, "x = f(1)\n")?,
            [("x".to_owned(), None, 0, 8)]
        );
        assert_eq!(
            symbols_found(&NAME_LAST, "x = f(1)\n")?,
            [("f".to_owned(), None, 0, 5)]
        );
        Ok(())
    }

    /// Of two definitions that start together, the longer one holds the shorter.
    #[test]
    fn outer_definitions_come_first() -> Result<(), Box<dyn Error>> {
        // In `f(x) + 1`, the sum is bytes 0 to 8 and the call `f(x)` 0 to 4.
        static NESTED: TagsSpec = python_spec(
            &[
                "(binary_operator left: (call arguments: (argument_list (identifier) @name))) @definition.class",
                "(call function: (identifier) @name) @definition.function",
            ],
            PLAIN_IMPORTS,
        );
        assert_eq!(
            symbols_found(&NESTED, "f(x) + 1\n")?,
            [
                ("x".to_owned(), None, 0, 8),
                ("x.f".to_owned(), Some(0), 0, 4)
            ]
        );
        Ok(())
    }

    /// A qualifier comes between the definition around it and the name.
    #[test]
    fn qualifiers_join_qualified_names() -> Result<(), Box<dyn Error>> {
        static QUALIFIED: TagsSpec = python_spec(
            &[
                "(class_definition name: (identifier) @name) @definition.class",
                "(function_definition name: (identifier) @name \
                 parameters: (parameters . (identifier) @qualifier)) @definition.function",
            ],
            PLAIN_IMPORTS,
        );
        // `f` takes `owner` first and sits in nothing: `owner.f`. `g` takes `item` first and sits
        // in the class `C`: `C.item.g`.
        let source = "def f(owner): pass\nclass C:\n    def g(item): pass\n";
        let found = symbols_found(&QUALIFIED, source)?;
        let names: Vec<&str> = found.iter().map(|(name, ..)| name.as_str()).collect();
        assert_eq!(names, ["owner.f", "C", "C.item.g"]);
        Ok(())
    }

    /// A span that ends just after a line break ends on the line before.
    #[test]
    fn a_final_line_break_is_not_a_line() {
        let span = |last: Point| Span {
            start: 0,
            end: 30,
            first: Point { row: 0, column: 0 },
            last,
        };
        assert_eq!(last_line(span(Point { row: 3, column: 0 })), Some(3));
        assert_eq!(last_line(span(Point { row: 3, column: 1 })), Some(4));
        let empty = Span {
            start: 5,
            end: 5,
            first: Point { row: 2, column: 0 },
            last: Point { row: 2, column: 0 },
        };
        assert_eq!(last_line(empty), Some(3));
    }
}
