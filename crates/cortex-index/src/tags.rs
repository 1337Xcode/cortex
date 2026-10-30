//! The definitions and references a tags query finds in a tree, read as tree-sitter's tags format
//! defines them: each match names something and says what it is, or that nothing may be recorded
//! for it (`@ignore`); where several patterns find the same name, the earliest pattern wins; a
//! name that holds a syntax error is skipped; and a name a pattern marks `#is-not? local` is
//! skipped where a scope around it defines it. Where one pattern finds the same name more than
//! once with different qualifiers, as a declaration after two PHP `namespace` statements is, the
//! qualifier nearest the name wins.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use cortex_core::{RefKind, SymbolKind};
use tree_sitter::{Node, QueryCapture, QueryCursor, QueryCursorOptions, StreamingIterator};

use crate::deadline::Deadline;
use crate::grammar::{Grammar, TagCapture};
use crate::locals::Scopes;
use crate::name_text::{name_text, prose_text};
use crate::pattern_rules::PatternRule;

/// What a tag says about its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The name is defined here, as this kind.
    Definition(SymbolKind),
    /// The name is defined here as a macro.
    Macro,
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
    /// Whether the name is prose, read as written rather than token by token.
    pub(crate) is_prose: bool,
    /// What the tag says about the name.
    pub(crate) role: Role,
    /// The name a definition's qualified name starts with, inside the definition around it.
    pub(crate) qualifier: Option<Node<'tree>>,
    /// A definition's body, when its pattern captures it.
    pub(crate) body: Option<Node<'tree>>,
}

impl Tag<'_> {
    /// The text of the tag's name in `source`: as written when its pattern marks it as prose,
    /// token by token otherwise. `None` when its text is not UTF-8.
    pub(crate) fn name_text(&self, source: &[u8]) -> Option<String> {
        if self.is_prose {
            prose_text(self.name, source)
        } else {
            name_text(self.name, source)
        }
    }
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
    let mut scopes = Scopes::new(source);
    let mut check = |_: &tree_sitter::QueryCursorState| deadline.check();
    let options = QueryCursorOptions::new().progress_callback(&mut check);
    let mut matches = cursor.matches_with_options(&grammar.tags, root, source, options);
    while let Some(found) = matches.next() {
        let pattern = found.pattern_index;
        let is_not_local = match grammar.pattern_rules.get(pattern) {
            Some(PatternRule::Locals { scope_inherits }) => {
                scopes.note(found.captures(), &grammar.tag_captures, *scope_inherits);
                continue;
            }
            Some(PatternRule::Tags { is_not_local }) => *is_not_local,
            None => continue,
        };
        let Some((name, tag)) = verdict_of(found.captures(), &grammar.tag_captures) else {
            continue;
        };
        if is_not_local && tag.is_some() && scopes.is_local(name) {
            continue;
        }
        match by_name.entry((name.start_byte(), name.end_byte())) {
            Entry::Vacant(entry) => {
                entry.insert((tag, pattern));
            }
            Entry::Occupied(mut entry) => {
                let (kept, kept_pattern) = entry.get();
                if pattern < *kept_pattern || (pattern == *kept_pattern && is_nearer(tag, *kept)) {
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

/// Whether `tag` has a qualifier that comes after the one `kept` has, so nearer the name both
/// qualify; the qualifier always comes before the name, as the pattern places it.
fn is_nearer(tag: Option<Tag<'_>>, kept: Option<Tag<'_>>) -> bool {
    match (
        tag.and_then(|tag| tag.qualifier),
        kept.and_then(|kept| kept.qualifier),
    ) {
        (Some(new), Some(old)) => new.start_byte() > old.start_byte(),
        _ => false,
    }
}

/// The name one match's `captures` are about, with the tag to record for it, or `None` for an
/// `@ignore`; nothing when the match records nothing: a name alone, or a tag whose name cannot be
/// one.
fn verdict_of<'tree>(
    captures: &[QueryCapture<'tree>],
    meanings: &[TagCapture],
) -> Option<(Node<'tree>, Option<Tag<'tree>>)> {
    let (mut name, mut tagged, mut qualifier, mut body) = (None, None, None, None);
    let mut is_prose = false;
    for capture in captures {
        match meanings.get(usize::try_from(capture.index).ok()?)? {
            TagCapture::Ignore => return Some((capture.node, None)),
            TagCapture::Name => name = Some(capture.node),
            TagCapture::ProseName => {
                name = Some(capture.node);
                is_prose = true;
            }
            TagCapture::Definition(kind) => tagged = Some((capture.node, Role::Definition(*kind))),
            TagCapture::Macro => tagged = Some((capture.node, Role::Macro)),
            TagCapture::Reference(kind) => tagged = Some((capture.node, Role::Reference(*kind))),
            TagCapture::Qualifier => qualifier = Some(capture.node),
            TagCapture::Body => body = Some(capture.node),
            TagCapture::Doc
            | TagCapture::Helper
            | TagCapture::LocalScope
            | TagCapture::LocalDefinition
            | TagCapture::LocalReference => {}
        }
    }
    let (node, role) = tagged?;
    let tag = Tag {
        node,
        name: name?,
        is_prose,
        role,
        qualifier,
        body,
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

    /// The names `spec` finds in `source`, each with the line it starts on, counted from 1.
    fn names_and_lines(
        spec: &'static TagsSpec,
        source: &str,
    ) -> Result<Vec<(String, usize)>, Box<dyn Error>> {
        with_tags(spec, source, |tags| {
            tags.iter()
                .map(|tag| {
                    let name = tag.name.utf8_text(source.as_bytes()).unwrap_or("?");
                    (name.to_owned(), tag.name.start_position().row + 1)
                })
                .collect()
        })
    }

    /// A pattern marked `#is-not? local` skips a name defined earlier in a scope around it, or in
    /// one around that while the scopes inherit. In `SOURCE`, `y` is defined in `f`, `x` in the
    /// file around `f`, and `z` nowhere: so `x` is local in `f` only while `f` inherits, and
    /// without the mark all three are recorded.
    #[test]
    fn local_names_are_skipped() -> Result<(), Box<dyn Error>> {
        static INHERITING: TagsSpec = python_spec(
            &[
                "(function_definition) @local.scope",
                "(assignment left: (identifier) @local.definition)",
                "((expression_statement (identifier) @name @reference.call) (#is-not? local))",
            ],
            PLAIN_IMPORTS,
        );
        static ISOLATED: TagsSpec = python_spec(
            &[
                "((function_definition) @local.scope (#set! local.scope-inherits false))",
                "(assignment left: (identifier) @local.definition)",
                "((expression_statement (identifier) @name @reference.call) (#is-not? local))",
            ],
            PLAIN_IMPORTS,
        );
        static UNMARKED: TagsSpec = python_spec(
            &[
                "(function_definition) @local.scope",
                "(assignment left: (identifier) @local.definition)",
                "(expression_statement (identifier) @name @reference.call)",
            ],
            PLAIN_IMPORTS,
        );
        const SOURCE: &str = "x = 1\ndef f():\n    y = 2\n    y\n    x\nz\n";
        let at = |name: &str, line| (name.to_owned(), line);
        assert_eq!(names_and_lines(&INHERITING, SOURCE)?, [at("z", 6)]);
        assert_eq!(
            names_and_lines(&ISOLATED, SOURCE)?,
            [at("x", 5), at("z", 6)]
        );
        assert_eq!(
            names_and_lines(&UNMARKED, SOURCE)?,
            [at("y", 4), at("x", 5), at("z", 6)]
        );
        // A name is local only after its definition: `a` on line 1 comes before `a = 1`.
        assert_eq!(names_and_lines(&INHERITING, "a\na = 1\na\n")?, [at("a", 1)]);
        // A definition belongs to the innermost scope around it: `w = 1` is defined in `g`,
        // which ends before the `w` on line 4, so that `w` is not local.
        assert_eq!(
            names_and_lines(
                &INHERITING,
                "def f():\n    def g():\n        w = 1\n    w\n"
            )?,
            [at("w", 4)]
        );
        // A scope that ended does not hide the one around it: `v` on line 5 comes after `g` and
        // is defined in `f`, which holds both.
        assert_eq!(
            names_and_lines(
                &INHERITING,
                "def f():\n    v = 1\n    def g():\n        pass\n    v\nz\n"
            )?,
            [at("z", 6)]
        );
        Ok(())
    }

    /// The search for the scopes around a name jumps over a scope's earlier siblings only where
    /// that cannot change the answer a search of every scope gives. In `OUT_OF_ORDER` the second
    /// pattern notes `y`'s scope again only when it reaches the `a` on line 5, inside `x`, where
    /// `a` is already defined: a scope out of the file's order turns the jumps off, or a jump from
    /// that late `y` would skip `x` and record `a`. In `LATE_NAME` the pattern for `v` ends inside
    /// the lambda's body, after the lambda's scope is noted, and `v` is defined in the parentheses,
    /// which end before the lambda: a jump from the lambda would skip them and record `v`.
    #[test]
    fn jumps_never_change_the_answer() -> Result<(), Box<dyn Error>> {
        static OUT_OF_ORDER: TagsSpec = python_spec(
            &[
                "(function_definition) @local.scope",
                "((function_definition) @local.scope . \
                 (function_definition body: (block . (_) . (expression_statement (identifier)))))",
                "(assignment left: (identifier) @local.definition)",
                "((expression_statement (identifier) @name @reference.call) (#is-not? local))",
            ],
            PLAIN_IMPORTS,
        );
        static LATE_NAME: TagsSpec = python_spec(
            &[
                "(parenthesized_expression) @local.scope",
                "(lambda) @local.scope",
                "(named_expression name: (identifier) @local.definition)",
                "((call function: (parenthesized_expression (named_expression name: (identifier) @name)) \
                  arguments: (argument_list (lambda body: (_)))) @reference.call (#is-not? local))",
                "((expression_statement (identifier) @name @reference.call) (#is-not? local))",
            ],
            PLAIN_IMPORTS,
        );
        let source = "def y():\n    pass\ndef x():\n    a = 1\n    a\nb\n";
        assert_eq!(
            names_and_lines(&OUT_OF_ORDER, source)?,
            [("b".to_owned(), 6)]
        );
        assert_eq!(
            names_and_lines(&LATE_NAME, "(v := 1)(lambda: 0)\nw\n")?,
            [("w".to_owned(), 2)]
        );
        Ok(())
    }

    /// `#is-not? local` applies only to a pattern that records a tag, as in tree-sitter-tags. A
    /// local name it skips is left to later patterns: in `SKIPPED`, `x` is local, so the second
    /// pattern reads it as a type, while `y` is not and the first reads it as a call. An `@ignore`
    /// marked `#is-not? local` ignores a local name all the same: in `IGNORED`, nothing is read.
    #[test]
    fn locality_applies_only_to_tags() -> Result<(), Box<dyn Error>> {
        static SKIPPED: TagsSpec = python_spec(
            &[
                "(assignment left: (identifier) @local.definition)",
                "((expression_statement (identifier) @name @reference.call) (#is-not? local))",
                "(expression_statement (identifier) @name @reference.type)",
            ],
            PLAIN_IMPORTS,
        );
        static IGNORED: TagsSpec = python_spec(
            &[
                "(assignment left: (identifier) @local.definition)",
                "((expression_statement (identifier) @ignore) (#is-not? local))",
                "(expression_statement (identifier) @name @reference.call)",
            ],
            PLAIN_IMPORTS,
        );
        let source = "x = 1\nx\ny\n";
        assert_eq!(
            tags_found(&SKIPPED, source)?,
            [
                ("x".to_owned(), Role::Reference(RefKind::TypeRef)),
                ("y".to_owned(), Role::Reference(RefKind::Call))
            ]
        );
        assert_eq!(tags_found(&IGNORED, source)?, Vec::new());
        Ok(())
    }

    /// Where one pattern finds a name more than once with different qualifiers, the qualifier
    /// nearest the name wins: `f` follows only `a`, while `g` follows `a` and then `b`.
    #[test]
    fn the_nearest_qualifier_wins() -> Result<(), Box<dyn Error>> {
        static SIBLINGS: TagsSpec = python_spec(
            &["(module (expression_statement (identifier) @qualifier) \
               (function_definition name: (identifier) @name) @definition.function)"],
            PLAIN_IMPORTS,
        );
        let source = "a\ndef f(): pass\nb\ndef g(): pass\n";
        let qualified = with_tags(&SIBLINGS, source, |tags| {
            tags.iter()
                .map(|tag| {
                    let text = |node: Node<'_>| node.utf8_text(source.as_bytes()).unwrap_or("?");
                    (
                        text(tag.name).to_owned(),
                        tag.qualifier.map(text).map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>()
        })?;
        assert_eq!(
            qualified,
            [
                ("f".to_owned(), Some("a".to_owned())),
                ("g".to_owned(), Some("b".to_owned()))
            ]
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
