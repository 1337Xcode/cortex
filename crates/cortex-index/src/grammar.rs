//! Each language's parsing rules compiled for use: its grammar, its tags query and its imports
//! query, with the meaning of every capture worked out and checked once, before any file is read.
//!
//! A tags query follows tree-sitter's tags format. A pattern that captures a `@name` and a
//! `@definition.<kind>` or `@reference.<kind>` records a tag; one that captures a `@name` alone
//! records nothing; one that captures `@ignore` alone keeps later patterns from recording that
//! name. A pattern may capture the comments before a definition as `@doc`, with any predicate on
//! them: Cortex keeps no comments, so those change nothing. Cortex adds two captures beside a
//! definition: `@qualifier`, a name its qualified name starts with, such as the type a Go method
//! belongs to, and `@body`, its body, where the grammar names no field for it, as Kotlin's does.
//! A name that is prose, such as a heading's text, is captured as `@name.prose` in place of
//! `@name`, to be read as written rather than token by token, and its qualifier with it.
//! A locals pattern captures only `@local.scope`, `@local.definition` and `@local.reference`,
//! and may set `local.scope-inherits` to `true` or `false`; a pattern that records tags may test
//! `#is-not? local`. No other property may be tested or set.
//! An imports query captures the statement as `@import` and the module as written
//! as `@import.module`, and may capture the name taken from it (`@import.name`), the name it is
//! bound to instead (`@import.alias`), and the mark that every name (`@import.all`) or the default
//! export (`@import.default`) is taken. A module the grammar writes as a run of sibling nodes,
//! with no node around them, runs from its `@import.module` to its `@import.module.end`. In
//! either query a capture named with a leading `_` only serves a text predicate such as `#eq?`.
//! A query that uses any other capture, or a predicate Cortex does not evaluate, is refused: a
//! mistake in a query fails the tests instead of being read some other way.

use cortex_core::{Language, RefKind, SymbolKind};
use thiserror::Error;
use tree_sitter::{Query, QueryPredicate, QueryPredicateArg};

use crate::languages::{self, TagsSpec};
use crate::pattern_rules::{PatternRule, Problem, check_import_pattern, tag_pattern_rule};

/// What each `@definition.<kind>` capture records; `@definition.macro` is read apart.
const DEFINITION_KINDS: &[(&str, SymbolKind)] = &[
    ("class", SymbolKind::Class),
    ("constant", SymbolKind::Constant),
    ("function", SymbolKind::Function),
    ("interface", SymbolKind::Interface),
    ("method", SymbolKind::Method),
    ("module", SymbolKind::Module),
    ("test", SymbolKind::Test),
    ("type", SymbolKind::Type),
];

/// What each `@reference.<kind>` capture records. A use of a class, as in `new Order()`, calls
/// its constructor, and an implementation of an interface or trait inherits from it.
const REFERENCE_KINDS: &[(&str, RefKind)] = &[
    ("call", RefKind::Call),
    ("class", RefKind::Call),
    ("implementation", RefKind::Inherit),
    ("inherit", RefKind::Inherit),
    ("type", RefKind::TypeRef),
];

/// The name of the tags query in messages.
const TAGS: &str = "tags";

/// The name of the imports query in messages.
const IMPORTS: &str = "imports";

/// What a capture in a tags query marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TagCapture {
    /// The name defined or used.
    Name,
    /// The name defined or used, written as prose, which is read as written.
    ProseName,
    /// A definition of this kind.
    Definition(SymbolKind),
    /// A definition of a macro, which is used as a function is but is never a test.
    Macro,
    /// A use of a name, of this kind.
    Reference(RefKind),
    /// The comments before a definition, which Cortex does not keep.
    Doc,
    /// A node only a text predicate looks at.
    Helper,
    /// A name no later pattern may record a tag for.
    Ignore,
    /// A name the definition's qualified name starts with, inside the definition around it.
    Qualifier,
    /// The definition's body, which its header ends before.
    Body,
    /// A scope names can be defined in, for `#is-not? local`.
    LocalScope,
    /// A name defined in the scope around it.
    LocalDefinition,
    /// A use of a local name, which the tags format reads and Cortex does not need.
    LocalReference,
}

/// What a capture in an imports query marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportCapture {
    /// The whole import statement.
    Statement,
    /// The module, as written, or the first node of it.
    Module,
    /// The last node of a module written as a run of sibling nodes.
    ModuleEnd,
    /// A name taken from the module.
    Name,
    /// The name the import is bound to instead of its own.
    Alias,
    /// The mark that every public name of the module is taken.
    All,
    /// The mark that the module's default export is taken.
    Default,
    /// A node only a text predicate looks at.
    Helper,
}

/// Why a language's parsing rules cannot be used. Each is a mistake in that language's file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GrammarError {
    /// A query does not compile against the grammar.
    #[error("the {query} query for {} does not compile: {message}", .language.name())]
    Query {
        /// The language.
        language: Language,
        /// Which query: `tags` or `imports`.
        query: &'static str,
        /// tree-sitter's description of the problem.
        message: String,
    },
    /// A query uses a capture Cortex does not read.
    #[error("the {query} query for {} uses @{capture}, which Cortex does not read", .language.name())]
    Capture {
        /// The language.
        language: Language,
        /// Which query: `tags` or `imports`.
        query: &'static str,
        /// The capture's name.
        capture: String,
    },
    /// A query uses a predicate Cortex does not evaluate.
    #[error("the {query} query for {} uses #{operator}, which Cortex does not evaluate", .language.name())]
    Predicate {
        /// The language.
        language: Language,
        /// Which query: `tags` or `imports`.
        query: &'static str,
        /// The predicate's name.
        operator: String,
    },
    /// A pattern lacks a capture every pattern needs, or repeats one.
    #[error("pattern {pattern} of the {query} query for {} {problem}", .language.name())]
    Pattern {
        /// The language.
        language: Language,
        /// Which query: `tags` or `imports`.
        query: &'static str,
        /// The pattern's position in the query, from 0.
        pattern: usize,
        /// What is wrong with it.
        problem: &'static str,
    },
}

/// One language's parsing rules, compiled.
#[derive(Debug)]
pub(crate) struct Grammar {
    /// The language.
    pub(crate) language: Language,
    /// The rules as the language's file states them.
    pub(crate) spec: &'static TagsSpec,
    /// The grammar, as tree-sitter's parser takes it.
    pub(crate) tree_sitter: tree_sitter::Language,
    /// The tags query.
    pub(crate) tags: Query,
    /// What each of the tags query's captures marks, by capture index.
    pub(crate) tag_captures: Vec<TagCapture>,
    /// What each of the tags query's patterns does, by pattern index.
    pub(crate) pattern_rules: Vec<PatternRule>,
    /// The imports query.
    pub(crate) imports: Query,
    /// What each of the imports query's captures marks, by capture index.
    pub(crate) import_captures: Vec<ImportCapture>,
}

/// Every language's parsing rules, compiled once and shared by every [`crate::Extractor`].
#[derive(Debug)]
pub struct Grammars {
    /// One entry per language, in [`Language::ALL`] order.
    grammars: Vec<Grammar>,
}

impl Grammars {
    /// Compiles every language's grammar and queries.
    ///
    /// # Errors
    ///
    /// A query that does not compile, or that uses a capture or a predicate Cortex does not read:
    /// a mistake in that language's file, which the tests catch.
    pub fn load() -> Result<Self, GrammarError> {
        let grammars = languages::ALL
            .iter()
            .map(|spec| Grammar::compile(spec.language, &spec.tags))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { grammars })
    }

    /// The compiled rules for `language`.
    #[allow(
        clippy::expect_used,
        reason = "load compiles the rules of every language in `languages::ALL`, which the \
                  detection tests hold to list every `Language`"
    )]
    pub(crate) fn get(&self, language: Language) -> &Grammar {
        self.grammars
            .iter()
            .find(|grammar| grammar.language == language)
            .expect("every language has its compiled rules")
    }
}

impl Grammar {
    /// Compiles `spec`, the rules for `language`.
    pub(crate) fn compile(
        language: Language,
        spec: &'static TagsSpec,
    ) -> Result<Self, GrammarError> {
        let grammar = (spec.grammar)();
        let tags = compile_query(language, TAGS, &grammar, &spec.queries.join("\n"))?;
        let tag_captures = captures_of(language, TAGS, &tags, tag_capture)?;
        check_general_predicates(language, TAGS, &tags, &tag_captures, is_doc_predicate)?;
        let pattern_rules = (0..tags.pattern_count())
            .map(|pattern| {
                tag_pattern_rule(&tags, pattern, &tag_captures)
                    .map_err(|problem| refusal(language, TAGS, pattern, problem))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let imports = compile_query(language, IMPORTS, &grammar, spec.imports)?;
        let import_captures = captures_of(language, IMPORTS, &imports, import_capture)?;
        check_general_predicates(language, IMPORTS, &imports, &import_captures, |_, _| false)?;
        for pattern in 0..imports.pattern_count() {
            check_import_pattern(&imports, pattern, &import_captures)
                .map_err(|problem| refusal(language, IMPORTS, pattern, problem))?;
        }
        Ok(Self {
            language,
            spec,
            tree_sitter: grammar,
            tags,
            tag_captures,
            pattern_rules,
            imports,
            import_captures,
        })
    }
}

/// `source` compiled against `grammar`.
fn compile_query(
    language: Language,
    query: &'static str,
    grammar: &tree_sitter::Language,
    source: &str,
) -> Result<Query, GrammarError> {
    Query::new(grammar, source).map_err(|error| GrammarError::Query {
        language,
        query,
        message: error.to_string(),
    })
}

/// Refuses a general predicate in `compiled` that Cortex does not evaluate: those `allows` does not
/// accept, given the meanings of the captures. The text predicates (`#eq?`, `#match?`, `#any-of?`
/// and their negations) are fine: tree-sitter applies them while matching.
fn check_general_predicates<C>(
    language: Language,
    query: &'static str,
    compiled: &Query,
    captures: &[C],
    allows: fn(&QueryPredicate, &[C]) -> bool,
) -> Result<(), GrammarError> {
    for pattern in 0..compiled.pattern_count() {
        let refused = compiled
            .general_predicates(pattern)
            .iter()
            .find(|predicate| !allows(predicate, captures));
        if let Some(predicate) = refused {
            return Err(GrammarError::Predicate {
                language,
                query,
                operator: predicate.operator.to_string(),
            });
        }
    }
    Ok(())
}

/// Whether a tags query may use `predicate`: only one on `@doc`, such as `#strip!` and
/// `#select-adjacent!`, which can only shape a definition's comments, and so change nothing
/// Cortex keeps.
fn is_doc_predicate(predicate: &QueryPredicate, captures: &[TagCapture]) -> bool {
    let first = match predicate.args.first() {
        Some(QueryPredicateArg::Capture(index)) => usize::try_from(*index)
            .ok()
            .and_then(|index| captures.get(index)),
        _ => None,
    };
    first == Some(&TagCapture::Doc)
}

/// What each of `compiled`'s captures marks, by capture index, as `meaning` reads its name.
fn captures_of<C>(
    language: Language,
    query: &'static str,
    compiled: &Query,
    meaning: fn(&str) -> Option<C>,
) -> Result<Vec<C>, GrammarError> {
    compiled
        .capture_names()
        .iter()
        .map(|name| {
            meaning(name).ok_or_else(|| GrammarError::Capture {
                language,
                query,
                capture: (*name).to_owned(),
            })
        })
        .collect()
}

/// What the tags capture called `name` marks, if Cortex reads it.
fn tag_capture(name: &str) -> Option<TagCapture> {
    match name {
        "name" => return Some(TagCapture::Name),
        "name.prose" => return Some(TagCapture::ProseName),
        "doc" => return Some(TagCapture::Doc),
        "ignore" => return Some(TagCapture::Ignore),
        "qualifier" => return Some(TagCapture::Qualifier),
        "body" => return Some(TagCapture::Body),
        "definition.macro" => return Some(TagCapture::Macro),
        "local.scope" => return Some(TagCapture::LocalScope),
        "local.definition" => return Some(TagCapture::LocalDefinition),
        "local.reference" => return Some(TagCapture::LocalReference),
        _ if is_helper(name) => return Some(TagCapture::Helper),
        _ => {}
    }
    if let Some(kind) = name.strip_prefix("definition.") {
        return DEFINITION_KINDS
            .iter()
            .find(|(known, _)| *known == kind)
            .map(|(_, kind)| TagCapture::Definition(*kind));
    }
    let kind = name.strip_prefix("reference.")?;
    REFERENCE_KINDS
        .iter()
        .find(|(known, _)| *known == kind)
        .map(|(_, kind)| TagCapture::Reference(*kind))
}

/// What the imports capture called `name` marks, if Cortex reads it.
fn import_capture(name: &str) -> Option<ImportCapture> {
    match name {
        "import" => Some(ImportCapture::Statement),
        "import.module" => Some(ImportCapture::Module),
        "import.module.end" => Some(ImportCapture::ModuleEnd),
        "import.name" => Some(ImportCapture::Name),
        "import.alias" => Some(ImportCapture::Alias),
        "import.all" => Some(ImportCapture::All),
        "import.default" => Some(ImportCapture::Default),
        _ if is_helper(name) => Some(ImportCapture::Helper),
        _ => None,
    }
}

/// Whether the capture called `name` only serves a text predicate: its name starts with `_`.
fn is_helper(name: &str) -> bool {
    name.starts_with('_')
}

/// The error for `problem` in `pattern` of `query`.
fn refusal(
    language: Language,
    query: &'static str,
    pattern: usize,
    problem: Problem,
) -> GrammarError {
    match problem {
        Problem::Shape(problem) => GrammarError::Pattern {
            language,
            query,
            pattern,
            problem,
        },
        Problem::Predicate(operator) => GrammarError::Predicate {
            language,
            query,
            operator: operator.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{PLAIN_IMPORTS as IMPORTS_QUERY, python_spec as spec};

    /// The error compiling `tags` and `imports` gives, if any.
    fn error_of(spec: &'static TagsSpec) -> Option<GrammarError> {
        Grammar::compile(Language::Python, spec).err()
    }

    /// Every language's queries compile, each into the rules for that language.
    #[test]
    fn every_language_compiles() -> Result<(), GrammarError> {
        let grammars = Grammars::load()?;
        for language in Language::ALL {
            assert_eq!(grammars.get(language).language, language);
        }
        Ok(())
    }

    /// Captures and their meaning: the tags format's names, Cortex's imports names, nothing else.
    #[test]
    fn capture_names_have_one_meaning() {
        assert_eq!(tag_capture("name"), Some(TagCapture::Name));
        assert_eq!(tag_capture("name.prose"), Some(TagCapture::ProseName));
        assert_eq!(tag_capture("doc"), Some(TagCapture::Doc));
        assert_eq!(tag_capture("_kind"), Some(TagCapture::Helper));
        assert_eq!(tag_capture("ignore"), Some(TagCapture::Ignore));
        assert_eq!(tag_capture("qualifier"), Some(TagCapture::Qualifier));
        assert_eq!(tag_capture("body"), Some(TagCapture::Body));
        assert_eq!(
            tag_capture("definition.method"),
            Some(TagCapture::Definition(SymbolKind::Method))
        );
        assert_eq!(tag_capture("definition.macro"), Some(TagCapture::Macro));
        assert_eq!(
            tag_capture("reference.class"),
            Some(TagCapture::Reference(RefKind::Call))
        );
        assert_eq!(
            tag_capture("definition.class"),
            Some(TagCapture::Definition(SymbolKind::Class))
        );
        assert_eq!(
            tag_capture("reference.type"),
            Some(TagCapture::Reference(RefKind::TypeRef))
        );
        assert_eq!(tag_capture("local.scope"), Some(TagCapture::LocalScope));
        assert_eq!(
            tag_capture("local.definition"),
            Some(TagCapture::LocalDefinition)
        );
        assert_eq!(
            tag_capture("local.reference"),
            Some(TagCapture::LocalReference)
        );
        for unknown in [
            "definition.widget",
            "reference.",
            "docs",
            "names",
            "local",
            "local.scopes",
            "bodies",
        ] {
            assert_eq!(tag_capture(unknown), None, "{unknown}");
        }
        assert_eq!(import_capture("import.alias"), Some(ImportCapture::Alias));
        assert_eq!(
            import_capture("import.module.end"),
            Some(ImportCapture::ModuleEnd)
        );
        assert_eq!(
            import_capture("import.default"),
            Some(ImportCapture::Default)
        );
        assert_eq!(import_capture("_require"), Some(ImportCapture::Helper));
        assert_eq!(import_capture("import.names"), None);
        assert_eq!(import_capture("doc"), None);
    }

    /// A query that does not compile is refused with tree-sitter's message.
    #[test]
    fn broken_queries_are_refused() {
        static BROKEN: TagsSpec = spec(
            &["(function_definition name: (identifier) @name"],
            IMPORTS_QUERY,
        );
        assert!(
            matches!(
                error_of(&BROKEN),
                Some(GrammarError::Query { query: "tags", .. })
            ),
            "{:?}",
            error_of(&BROKEN)
        );
    }

    /// A capture outside the formats is refused by name, in either query.
    #[test]
    fn unknown_captures_are_refused() {
        static EXTRA: TagsSpec = spec(
            &["(function_definition name: (identifier) @name) @definition.function @extra"],
            IMPORTS_QUERY,
        );
        static KIND: TagsSpec = spec(
            &["(function_definition name: (identifier) @name) @definition.widget"],
            IMPORTS_QUERY,
        );
        static IMPORT: TagsSpec = spec(
            &["(function_definition name: (identifier) @name) @definition.function"],
            "(import_statement name: (dotted_name) @import.module) @import @import.source",
        );
        static PROSE_IMPORT: TagsSpec = spec(
            &["(function_definition name: (identifier) @name) @definition.function"],
            "(import_statement name: (dotted_name) @import.module @name.prose) @import",
        );
        let refused = |query, capture: &str| GrammarError::Capture {
            language: Language::Python,
            query,
            capture: capture.to_owned(),
        };
        assert_eq!(error_of(&EXTRA), Some(refused(TAGS, "extra")));
        assert_eq!(error_of(&KIND), Some(refused(TAGS, "definition.widget")));
        assert_eq!(error_of(&IMPORT), Some(refused(IMPORTS, "import.source")));
        assert_eq!(
            error_of(&PROSE_IMPORT),
            Some(refused(IMPORTS, "name.prose"))
        );
    }

    /// A tags pattern records a tag with a name and one definition or reference, records
    /// nothing with a name alone, or ignores a name with `@ignore` alone; only a definition may
    /// have a qualifier. Anything else is refused.
    #[test]
    fn tag_patterns_have_the_formats_shapes() {
        static SHAPES: TagsSpec = spec(
            &[
                "(class_definition name: (identifier) @name) @definition.class",
                "(function_definition name: (identifier) @name)",
                "((identifier) @ignore (#eq? @ignore \"self\"))",
                "(function_definition name: (identifier) @name parameters: (parameters (identifier) @qualifier)) @definition.function",
            ],
            IMPORTS_QUERY,
        );
        static TWO_KINDS: TagsSpec = spec(
            &["(call function: (identifier) @name @reference.call) @definition.function"],
            IMPORTS_QUERY,
        );
        static IGNORE_AND_NAME: TagsSpec = spec(
            &[
                "(call function: (identifier) @ignore arguments: (argument_list (identifier) @name))",
            ],
            IMPORTS_QUERY,
        );
        static QUALIFIED_REFERENCE: TagsSpec = spec(
            &[
                "(call function: (attribute object: (identifier) @qualifier attribute: (identifier) @name)) @reference.call",
            ],
            IMPORTS_QUERY,
        );
        static LOCALS: TagsSpec = spec(
            &[
                "(function_definition) @local.scope",
                "(assignment left: (identifier) @local.definition)",
                "(identifier) @local.reference",
            ],
            IMPORTS_QUERY,
        );
        static LOCALS_WITH_HELPER: TagsSpec = spec(
            &[
                "((assignment left: (identifier) @local.definition right: (_) @_value) \
               (#eq? @_value \"1\"))",
            ],
            IMPORTS_QUERY,
        );
        static MIXED: TagsSpec = spec(
            &["(function_definition name: (identifier) @name) @definition.function @local.scope"],
            IMPORTS_QUERY,
        );
        let problem = |problem| GrammarError::Pattern {
            language: Language::Python,
            query: TAGS,
            pattern: 0,
            problem,
        };
        assert_eq!(error_of(&SHAPES), None);
        assert_eq!(error_of(&LOCALS), None);
        assert_eq!(error_of(&LOCALS_WITH_HELPER), None);
        assert_eq!(
            error_of(&MIXED),
            Some(problem("may not mix @local captures with others"))
        );
        assert_eq!(
            error_of(&TWO_KINDS),
            Some(problem(
                "needs at most one @definition or @reference capture"
            ))
        );
        assert_eq!(
            error_of(&IGNORE_AND_NAME),
            Some(problem("may capture nothing beside one @ignore"))
        );
        assert_eq!(
            error_of(&QUALIFIED_REFERENCE),
            Some(problem("may capture a @qualifier only with a @definition"))
        );
    }

    /// A tag's name is captured once in every match, as `@name` or, when it is prose, as
    /// `@name.prose`, never as both, and a locals pattern captures neither.
    #[test]
    fn tags_have_one_name() {
        static PROSE: TagsSpec = spec(
            &["(expression_statement (string (string_content) @name.prose)) @definition.constant"],
            IMPORTS_QUERY,
        );
        static NO_NAME: TagsSpec = spec(
            &["(function_definition) @definition.function"],
            IMPORTS_QUERY,
        );
        static TWO_NAMES: TagsSpec = spec(
            &[
                "(function_definition name: (identifier) @name body: (block (expression_statement (identifier) @name))) @definition.function",
            ],
            IMPORTS_QUERY,
        );
        static NAME_AND_PROSE: TagsSpec = spec(
            &[
                "(function_definition name: (identifier) @name body: (block (expression_statement (string) @name.prose))) @definition.function",
            ],
            IMPORTS_QUERY,
        );
        static MAYBE_NAME: TagsSpec = spec(
            &["(call function: [(identifier) @name (attribute)]) @reference.call"],
            IMPORTS_QUERY,
        );
        static LOCAL_PROSE: TagsSpec = spec(
            &["(assignment left: (identifier) @local.definition @name.prose)"],
            IMPORTS_QUERY,
        );
        let one_name = || {
            Some(GrammarError::Pattern {
                language: Language::Python,
                query: TAGS,
                pattern: 0,
                problem: "needs exactly one @name or @name.prose",
            })
        };
        assert_eq!(error_of(&PROSE), None);
        assert_eq!(error_of(&NO_NAME), one_name());
        assert_eq!(error_of(&TWO_NAMES), one_name());
        assert_eq!(error_of(&NAME_AND_PROSE), one_name());
        assert_eq!(error_of(&MAYBE_NAME), one_name());
        assert_eq!(
            error_of(&LOCAL_PROSE),
            Some(GrammarError::Pattern {
                language: Language::Python,
                query: TAGS,
                pattern: 0,
                problem: "may not mix @local captures with others",
            })
        );
    }

    /// A definition may capture its body, once, maybe not in every match; a reference or an
    /// `@ignore` may not.
    #[test]
    fn bodies_belong_to_one_definition() {
        static BODY: TagsSpec = spec(
            &["(class_definition name: (identifier) @name (block)? @body) @definition.class"],
            IMPORTS_QUERY,
        );
        static BODY_OF_REFERENCE: TagsSpec = spec(
            &[
                "(call function: (identifier) @name arguments: (argument_list) @body) @reference.call",
            ],
            IMPORTS_QUERY,
        );
        static TWO_BODIES: TagsSpec = spec(
            &[
                "(function_definition name: (identifier) @name parameters: (parameters) @body (block) @body) @definition.function",
            ],
            IMPORTS_QUERY,
        );
        static IGNORE_AND_BODY: TagsSpec = spec(
            &["(function_definition name: (identifier) @ignore (block) @body)"],
            IMPORTS_QUERY,
        );
        let problem = |problem| GrammarError::Pattern {
            language: Language::Python,
            query: TAGS,
            pattern: 0,
            problem,
        };
        assert_eq!(error_of(&BODY), None);
        assert_eq!(
            error_of(&BODY_OF_REFERENCE),
            Some(problem("may capture a @body only with a @definition"))
        );
        assert_eq!(
            error_of(&TWO_BODIES),
            Some(problem("may capture one @body at most"))
        );
        assert_eq!(
            error_of(&IGNORE_AND_BODY),
            Some(problem("may capture nothing beside one @ignore"))
        );
    }

    /// An imports pattern needs the statement and the module once, and the rest at most once:
    /// a name or the mark for all names in one match and not the other is fine, and so is a
    /// module's last node.
    #[test]
    fn import_patterns_need_a_statement_and_a_module() {
        static TAGS_QUERY: &[&str] =
            &["(function_definition name: (identifier) @name) @definition.function"];
        static NO_MODULE: TagsSpec = spec(TAGS_QUERY, "(import_statement) @import");
        static NO_STATEMENT: TagsSpec = spec(
            TAGS_QUERY,
            "(import_statement name: (dotted_name) @import.module)",
        );
        static MANY_NAMES: TagsSpec = spec(
            TAGS_QUERY,
            "(import_from_statement module_name: (_) @import.module (dotted_name)+ @import.name) @import",
        );
        static NAME_OR_ALL: TagsSpec = spec(
            TAGS_QUERY,
            "(import_from_statement module_name: (_) @import.module \
             [name: (dotted_name) @import.name (wildcard_import) @import.all]) @import",
        );
        static RUN: TagsSpec = spec(
            TAGS_QUERY,
            "(import_statement name: (dotted_name . (identifier) @import.module \
             (identifier) @import.module.end . (identifier) @import.name .)) @import",
        );
        static TWO_ENDS: TagsSpec = spec(
            TAGS_QUERY,
            "(import_statement name: (dotted_name . (identifier) @import.module \
             (identifier) @import.module.end (identifier) @import.module.end)) @import",
        );
        assert_eq!(error_of(&NAME_OR_ALL), None);
        assert_eq!(error_of(&RUN), None);
        let problem = |problem| GrammarError::Pattern {
            language: Language::Python,
            query: IMPORTS,
            pattern: 0,
            problem,
        };
        assert_eq!(
            error_of(&NO_MODULE),
            Some(problem("needs exactly one @import.module"))
        );
        assert_eq!(
            error_of(&NO_STATEMENT),
            Some(problem("needs exactly one @import"))
        );
        let at_most_once = "may capture @import.module.end, @import.name, @import.alias, @import.all \
                            and @import.default once each at most";
        assert_eq!(error_of(&MANY_NAMES), Some(problem(at_most_once)));
        assert_eq!(error_of(&TWO_ENDS), Some(problem(at_most_once)));
    }

    /// The error refusing `operator` in `query` of a Python grammar.
    fn refused(query: &'static str, operator: &str) -> GrammarError {
        GrammarError::Predicate {
            language: Language::Python,
            query,
            operator: operator.to_owned(),
        }
    }

    /// Predicates Cortex does not evaluate are refused by name. Text predicates are accepted, and
    /// so is any predicate on `@doc` in a tags query, such as `#strip!`, which only shapes
    /// comments.
    #[test]
    fn unevaluated_predicates_are_refused() {
        static STRIP: TagsSpec = spec(
            &["((identifier) @name @reference.call (#strip! @name \"_\"))"],
            IMPORTS_QUERY,
        );
        static TEXT: TagsSpec = spec(
            &["((identifier) @name @reference.call (#not-any-of? @name \"print\" \"len\"))"],
            IMPORTS_QUERY,
        );
        static DOC: TagsSpec = spec(
            &[
                "((comment)* @doc . (function_definition name: (identifier) @name) @definition.function \
               (#strip! @doc \"^#\") (#select-adjacent! @doc @definition.function))",
            ],
            IMPORTS_QUERY,
        );
        static DOC_TYPO: TagsSpec = spec(
            &[
                "((comment)* @doc . (function_definition name: (identifier) @name) @definition.function \
               (#set-adjacent! @doc @definition.function))",
            ],
            IMPORTS_QUERY,
        );
        static HELPER: TagsSpec = spec(
            &["((call function: (identifier) @_callee) @reference.call \
               (#strip! @_callee \"x\"))"],
            IMPORTS_QUERY,
        );
        static IMPORT_STRIP: TagsSpec = spec(
            &["(function_definition name: (identifier) @name) @definition.function"],
            "((import_statement name: (dotted_name) @import.module) @import \
             (#strip! @import.module \"x\"))",
        );
        assert_eq!(error_of(&STRIP), Some(refused(TAGS, "strip!")));
        assert_eq!(error_of(&TEXT), None);
        assert_eq!(error_of(&DOC), None);
        assert_eq!(error_of(&DOC_TYPO), None);
        assert_eq!(error_of(&HELPER), Some(refused(TAGS, "strip!")));
        assert_eq!(error_of(&IMPORT_STRIP), Some(refused(IMPORTS, "strip!")));
    }

    /// The only properties accepted are the tags format's locals: a pattern that records tags may
    /// test `#is-not? local`, and a locals pattern may set `local.scope-inherits` to `true` or
    /// `false`. Any other test or setting is refused by name, and an imports pattern has none.
    #[test]
    fn only_locals_properties_are_accepted() {
        static IS_NOT_LOCAL: TagsSpec = spec(
            &["((identifier) @name @reference.call (#is-not? local))"],
            IMPORTS_QUERY,
        );
        static IS_LOCAL: TagsSpec = spec(
            &["((identifier) @name @reference.call (#is? local))"],
            IMPORTS_QUERY,
        );
        static IS_NOT_GLOBAL: TagsSpec = spec(
            &["((identifier) @name @reference.call (#is-not? global))"],
            IMPORTS_QUERY,
        );
        static IS_NOT_LOCAL_VALUE: TagsSpec = spec(
            &["((identifier) @name @reference.call (#is-not? local \"x\"))"],
            IMPORTS_QUERY,
        );
        static SET: TagsSpec = spec(
            &["((identifier) @name @reference.call (#set! kind \"call\"))"],
            IMPORTS_QUERY,
        );
        static SCOPE_ISOLATED: TagsSpec = spec(
            &["((function_definition) @local.scope (#set! local.scope-inherits false))"],
            IMPORTS_QUERY,
        );
        static SCOPE_INHERITS: TagsSpec = spec(
            &["((function_definition) @local.scope (#set! local.scope-inherits true))"],
            IMPORTS_QUERY,
        );
        static SCOPE_MAYBE: TagsSpec = spec(
            &["((function_definition) @local.scope (#set! local.scope-inherits maybe))"],
            IMPORTS_QUERY,
        );
        static SCOPE_OTHER: TagsSpec = spec(
            &["((function_definition) @local.scope (#set! local.scope-kind false))"],
            IMPORTS_QUERY,
        );
        static LOCAL_TEST: TagsSpec = spec(
            &["((identifier) @local.reference (#is-not? local))"],
            IMPORTS_QUERY,
        );
        static IMPORT_IS_NOT: TagsSpec = spec(
            &["(function_definition name: (identifier) @name) @definition.function"],
            "((import_statement name: (dotted_name) @import.module) @import (#is-not? local))",
        );
        static IMPORT_SET: TagsSpec = spec(
            &["(function_definition name: (identifier) @name) @definition.function"],
            "((import_statement name: (dotted_name) @import.module) @import (#set! kind module))",
        );
        assert_eq!(error_of(&IS_NOT_LOCAL), None);
        assert_eq!(error_of(&IS_LOCAL), Some(refused(TAGS, "is?")));
        assert_eq!(error_of(&IS_NOT_GLOBAL), Some(refused(TAGS, "is-not?")));
        assert_eq!(
            error_of(&IS_NOT_LOCAL_VALUE),
            Some(refused(TAGS, "is-not?"))
        );
        assert_eq!(error_of(&SET), Some(refused(TAGS, "set!")));
        assert_eq!(error_of(&SCOPE_ISOLATED), None);
        assert_eq!(error_of(&SCOPE_INHERITS), None);
        assert_eq!(error_of(&SCOPE_MAYBE), Some(refused(TAGS, "set!")));
        assert_eq!(error_of(&SCOPE_OTHER), Some(refused(TAGS, "set!")));
        assert_eq!(error_of(&LOCAL_TEST), Some(refused(TAGS, "is-not?")));
        assert_eq!(error_of(&IMPORT_IS_NOT), Some(refused(IMPORTS, "is-not?")));
        assert_eq!(error_of(&IMPORT_SET), Some(refused(IMPORTS, "set!")));
    }
}
