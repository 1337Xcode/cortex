//! The definitions and references a tags query finds in a tree, read as tree-sitter's tags format
//! defines them: each match names something and says what it is, or that nothing may be recorded
//! for it (`@ignore`); where several patterns find the same name, the earliest pattern wins; a
//! name that holds a syntax error is skipped.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use cortex_core::{RefKind, SymbolKind};
use tree_sitter::{Node, QueryCapture, QueryCursor, QueryCursorOptions, StreamingIterator};

use crate::deadline::Deadline;
use crate::grammar::{Grammar, TagCapture};

/// What a tag says about its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The name is defined here, as this kind.
    Definition(SymbolKind),
    /// The name is used here, in this way.
    Reference(RefKind),
}

/// A name the tags query found, and what it is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tag<'tree> {
    /// The node of the whole definition or reference.
    pub(crate) node: Node<'tree>,
    /// The node of the name.
    pub(crate) name: Node<'tree>,
    /// What the tag says about the name.
    pub(crate) role: Role,
    /// The name a definition's qualified name starts with, inside the definition around it.
    pub(crate) qualifier: Option<Node<'tree>>,
}

/// What one match says about a name: a tag to record, or nothing to record (`None`), and the
/// pattern that says it.
type Verdict<'tree> = (Option<Tag<'tree>>, usize);

/// The tags in the tree under `root`, in the order their names appear; `None` when the deadline
/// passed before the query finished.
pub(crate) fn read_tags<'tree>(
    cursor: &mut QueryCursor,
    grammar: &Grammar,
    root: Node<'tree>,
    source: &[u8],
    deadline: &Deadline,
) -> Option<Vec<Tag<'tree>>> {
    // INVARIANT: deterministic. Tags are kept in the order of their names in the source, however
    // the cursor orders its matches.
    let mut by_name: BTreeMap<(usize, usize), Verdict<'tree>> = BTreeMap::new();
    let mut check = |_: &tree_sitter::QueryCursorState| deadline.check();
    let options = QueryCursorOptions::new().progress_callback(&mut check);
    let mut matches = cursor.matches_with_options(&grammar.tags, root, source, options);
    while let Some(found) = matches.next() {
        let Some((name, tag)) = verdict_of(found.captures(), &grammar.tag_captures) else {
            continue;
        };
        let pattern = found.pattern_index;
        match by_name.entry((name.start_byte(), name.end_byte())) {
            Entry::Vacant(entry) => {
                entry.insert((tag, pattern));
            }
            Entry::Occupied(mut entry) => {
                if pattern < entry.get().1 {
                    entry.insert((tag, pattern));
                }
            }
        }
    }
    if deadline.has_passed() {
        return None;
    }
    Some(by_name.into_values().filter_map(|(tag, _)| tag).collect())
}

/// The name one match's `captures` are about, with the tag to record for it, or `None` for an
/// `@ignore`; nothing when the match records nothing: a name alone, or a tag whose name cannot be
/// one.
fn verdict_of<'tree>(
    captures: &[QueryCapture<'tree>],
    meanings: &[TagCapture],
) -> Option<(Node<'tree>, Option<Tag<'tree>>)> {
    let (mut name, mut tagged, mut qualifier) = (None, None, None);
    for capture in captures {
        match meanings.get(usize::try_from(capture.index).ok()?)? {
            TagCapture::Ignore => return Some((capture.node, None)),
            TagCapture::Name => name = Some(capture.node),
            TagCapture::Definition(kind) => tagged = Some((capture.node, Role::Definition(*kind))),
            TagCapture::Reference(kind) => tagged = Some((capture.node, Role::Reference(*kind))),
            TagCapture::Qualifier => qualifier = Some(capture.node),
            TagCapture::Doc | TagCapture::Helper => {}
        }
    }
    let (node, role) = tagged?;
    let tag = Tag {
        node,
        name: name?,
        role,
        qualifier,
    };
    // INVARIANT: no-content-stored. A name is the text of a name capture alone, never the whole
    // of a definition or an expression.
    if tag.name.has_error() || names_whole_node(&tag) {
        return None;
    }
    Some((tag.name, Some(tag)))
}

/// Whether the tag's name covers the whole of its node while the name holds other nodes, as when
/// a pattern captures a whole definition or call as the name. Such a pattern is mistaken, and the
/// text of a definition or an expression must never become a name. A name that is a single token
/// may cover its node: the identifier of a call, or a Go constant written as its name alone.
fn names_whole_node(tag: &Tag<'_>) -> bool {
    let (name, node) = (tag.name.byte_range(), tag.node.byte_range());
    name.start <= node.start && node.end <= name.end && tag.name.named_child_count() > 0
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use cortex_core::Language;

    use super::*;
    use crate::languages::TagsSpec;
    use crate::test_support::{
        PLAIN_IMPORTS, many_functions, parse_python, python_spec, with_tags,
    };

    /// The names and roles `spec` finds in `source`.
    fn tags_found(
        spec: &'static TagsSpec,
        source: &str,
    ) -> Result<Vec<(String, Role)>, Box<dyn Error>> {
        with_tags(spec, source, |tags| {
            tags.iter()
                .map(|tag| {
                    let name = tag.name.utf8_text(source.as_bytes()).unwrap_or("?");
                    (name.to_owned(), tag.role)
                })
                .collect()
        })
    }

    /// A pattern whose name is the whole definition finds nothing, beside one that finds the
    /// name alone.
    #[test]
    fn whole_definitions_are_never_names() -> Result<(), Box<dyn Error>> {
        static WHOLE: TagsSpec = python_spec(
            &[
                "(function_definition) @name @definition.function",
                "(class_definition name: (identifier) @name) @definition.class",
            ],
            PLAIN_IMPORTS,
        );
        assert_eq!(
            tags_found(&WHOLE, "def f(): pass\nclass C: pass\n")?,
            [("C".to_owned(), Role::Definition(SymbolKind::Class))]
        );
        Ok(())
    }

    /// A name covering a whole call is refused for a reference too; a name that is the whole of
    /// a single token is not.
    #[test]
    fn whole_expressions_are_never_names() -> Result<(), Box<dyn Error>> {
        static WHOLE_CALL: TagsSpec = python_spec(&["(call) @name @reference.call"], PLAIN_IMPORTS);
        static TOKEN: TagsSpec = python_spec(
            &["(call function: (identifier) @name @reference.call)"],
            PLAIN_IMPORTS,
        );
        assert_eq!(tags_found(&WHOLE_CALL, "f(x)\n")?, Vec::new());
        assert_eq!(
            tags_found(&TOKEN, "f(x)\n")?,
            [("f".to_owned(), Role::Reference(RefKind::Call))]
        );
        Ok(())
    }

    /// A definition whose node holds nothing but its name is named, as a Go constant written as
    /// its name alone is: the line `X` is an expression statement spanning the identifier `X`.
    #[test]
    fn a_definition_that_is_only_its_name_is_named() -> Result<(), Box<dyn Error>> {
        static BARE: TagsSpec = python_spec(
            &["(expression_statement (identifier) @name) @definition.constant"],
            PLAIN_IMPORTS,
        );
        assert_eq!(
            tags_found(&BARE, "X\n")?,
            [("X".to_owned(), Role::Definition(SymbolKind::Constant))]
        );
        Ok(())
    }

    /// An `@ignore` keeps later patterns from recording a name, not earlier ones; a name
    /// alone records nothing.
    #[test]
    fn ignored_names_and_names_alone_record_nothing() -> Result<(), Box<dyn Error>> {
        static IGNORE_FIRST: TagsSpec = python_spec(
            &[
                "((identifier) @ignore (#eq? @ignore \"len\"))",
                "(call function: (identifier) @name) @reference.call",
            ],
            PLAIN_IMPORTS,
        );
        static IGNORE_LAST: TagsSpec = python_spec(
            &[
                "(call function: (identifier) @name) @reference.call",
                "((identifier) @ignore (#eq? @ignore \"len\"))",
            ],
            PLAIN_IMPORTS,
        );
        static NAME_ALONE: TagsSpec =
            python_spec(&["(call function: (identifier) @name)"], PLAIN_IMPORTS);
        let call = |name: &str| (name.to_owned(), Role::Reference(RefKind::Call));
        let source = "len(x)\nf(x)\n";
        assert_eq!(tags_found(&IGNORE_FIRST, source)?, [call("f")]);
        assert_eq!(tags_found(&IGNORE_LAST, source)?, [call("len"), call("f")]);
        assert_eq!(tags_found(&NAME_ALONE, source)?, Vec::new());
        Ok(())
    }

    /// Where two patterns find the same name, the earlier one says what it is.
    #[test]
    fn the_earlier_pattern_wins() -> Result<(), Box<dyn Error>> {
        static CALL_FIRST: TagsSpec = python_spec(
            &[
                "(call function: (identifier) @name) @reference.call",
                "(call function: (identifier) @name) @reference.type",
            ],
            PLAIN_IMPORTS,
        );
        static TYPE_FIRST: TagsSpec = python_spec(
            &[
                "(call function: (identifier) @name) @reference.type",
                "(call function: (identifier) @name) @reference.call",
            ],
            PLAIN_IMPORTS,
        );
        assert_eq!(
            tags_found(&CALL_FIRST, "f()\n")?,
            [("f".to_owned(), Role::Reference(RefKind::Call))]
        );
        assert_eq!(
            tags_found(&TYPE_FIRST, "f()\n")?,
            [("f".to_owned(), Role::Reference(RefKind::TypeRef))]
        );
        Ok(())
    }

    /// A name holding a syntax error is skipped: here a dotted name broken by a stray character.
    #[test]
    fn names_with_errors_are_skipped() -> Result<(), Box<dyn Error>> {
        static IMPORTED: TagsSpec = python_spec(
            &["(import_statement name: (dotted_name) @name) @reference.call"],
            PLAIN_IMPORTS,
        );
        assert_eq!(
            tags_found(&IMPORTED, "import a.$b\nimport c\n")?,
            [("c".to_owned(), Role::Reference(RefKind::Call))]
        );
        Ok(())
    }

    /// Once the deadline has passed, the query stops and reports it instead of a partial list.
    #[test]
    fn queries_stop_at_the_deadline() -> Result<(), Box<dyn Error>> {
        static CALLS: TagsSpec = python_spec(
            &["(call function: (identifier) @name) @reference.call"],
            PLAIN_IMPORTS,
        );
        let grammar = Grammar::compile(Language::Python, &CALLS)?;
        let source = many_functions(5000);
        let tree = parse_python(&source)?;
        let read = |allowance| {
            read_tags(
                &mut QueryCursor::new(),
                &grammar,
                tree.root_node(),
                source.as_bytes(),
                &Deadline::after(allowance),
            )
            .map(|tags| tags.len())
        };
        assert_eq!(read(Duration::ZERO), None);
        assert_eq!(read(Duration::from_secs(60)), Some(5000));
        Ok(())
    }
}
