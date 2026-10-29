//! Each language's parsing rules compiled for use: its grammar, its tags query and its imports
//! query, with the meaning of every capture worked out and checked once, before any file is read.
//!
//! A tags query follows tree-sitter's tags format: every pattern captures one `@name` and one
//! `@definition.<kind>` or `@reference.<kind>`. An imports query captures the statement as
//! `@import` and the module as written as `@import.module`, and may capture the name taken from it
//! (`@import.name`), the name it is bound to instead (`@import.alias`) and the mark that every
//! name is taken (`@import.all`). A query that uses any other capture, or a predicate Cortex does
//! not evaluate, is refused: a mistake in a query fails the tests instead of being read some other
//! way.

use cortex_core::{Language, RefKind, SymbolKind};
use thiserror::Error;
use tree_sitter::{CaptureQuantifier, Query};

use crate::languages::{self, TagsSpec};

/// What each `@definition.<kind>` capture records.
const DEFINITION_KINDS: &[(&str, SymbolKind)] = &[
    ("class", SymbolKind::Class),
    ("constant", SymbolKind::Constant),
    ("function", SymbolKind::Function),
    ("type", SymbolKind::Type),
];

/// What each `@reference.<kind>` capture records.
const REFERENCE_KINDS: &[(&str, RefKind)] = &[
    ("call", RefKind::Call),
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
    /// A definition of this kind.
    Definition(SymbolKind),
    /// A use of a name, of this kind.
    Reference(RefKind),
}

/// What a capture in an imports query marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportCapture {
    /// The whole import statement.
    Statement,
    /// The module, as written.
    Module,
    /// A name taken from the module.
    Name,
    /// The name the import is bound to instead of its own.
    Alias,
    /// The mark that every public name of the module is taken.
    All,
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
    /// The imports query.
    pub(crate) imports: Query,
    /// What each of the imports query's captures marks, by capture index.
    pub(crate) import_captures: Vec<ImportCapture>,
}

/// Every language's parsing rules, compiled once and shared by every [`crate::Extractor`].
#[derive(Debug)]
pub struct Grammars {
    /// One entry per language Cortex parses, in [`Language::ALL`] order.
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
            .filter_map(|spec| {
                let tags = spec.tags.as_ref()?;
                Some(Grammar::compile(spec.language, tags))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { grammars })
    }

    /// The compiled rules for `language`, when Cortex parses it.
    pub(crate) fn get(&self, language: Language) -> Option<&Grammar> {
        self.grammars
            .iter()
            .find(|grammar| grammar.language == language)
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
        for pattern in 0..tags.pattern_count() {
            check_tag_pattern(&tags, pattern, &tag_captures)
                .map_err(|problem| pattern_error(language, TAGS, pattern, problem))?;
        }
        let imports = compile_query(language, IMPORTS, &grammar, spec.imports)?;
        let import_captures = captures_of(language, IMPORTS, &imports, import_capture)?;
        for pattern in 0..imports.pattern_count() {
            check_import_pattern(&imports, pattern, &import_captures)
                .map_err(|problem| pattern_error(language, IMPORTS, pattern, problem))?;
        }
        Ok(Self {
            language,
            spec,
            tree_sitter: grammar,
            tags,
            tag_captures,
            imports,
            import_captures,
        })
    }
}

/// `source` compiled against `grammar`, refused if it uses a predicate Cortex does not evaluate.
/// The text predicates (`#eq?`, `#match?`, `#any-of?` and their negations) are fine: tree-sitter
/// applies them while matching.
fn compile_query(
    language: Language,
    query: &'static str,
    grammar: &tree_sitter::Language,
    source: &str,
) -> Result<Query, GrammarError> {
    let compiled = Query::new(grammar, source).map_err(|error| GrammarError::Query {
        language,
        query,
        message: error.to_string(),
    })?;
    for pattern in 0..compiled.pattern_count() {
        let operator = if let Some(predicate) = compiled.general_predicates(pattern).first() {
            Some(predicate.operator.to_string())
        } else if let Some((_, is_positive)) = compiled.property_predicates(pattern).first() {
            Some(if *is_positive { "is?" } else { "is-not?" }.to_owned())
        } else if compiled.property_settings(pattern).is_empty() {
            None
        } else {
            Some("set!".to_owned())
        };
        if let Some(operator) = operator {
            return Err(GrammarError::Predicate {
                language,
                query,
                operator,
            });
        }
    }
    Ok(compiled)
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
    if name == "name" {
        return Some(TagCapture::Name);
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
        "import.name" => Some(ImportCapture::Name),
        "import.alias" => Some(ImportCapture::Alias),
        "import.all" => Some(ImportCapture::All),
        _ => None,
    }
}

/// How often a pattern uses the captures of one sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    /// Not at all.
    Never,
    /// Exactly once in every match.
    Once,
    /// At most once, and not in every match.
    Maybe,
    /// More than once, or any number of times.
    Many,
}

/// How often `pattern` of `compiled` uses the captures that `is_sort` picks from `captures`.
fn use_of<C: Copy>(
    compiled: &Query,
    pattern: usize,
    captures: &[C],
    is_sort: impl Fn(C) -> bool,
) -> Use {
    let mut used = compiled
        .capture_quantifiers(pattern)
        .iter()
        .zip(captures)
        .filter(|(quantifier, capture)| {
            **quantifier != CaptureQuantifier::Zero && is_sort(**capture)
        })
        .map(|(quantifier, _)| *quantifier);
    match (used.next(), used.next()) {
        (None, _) => Use::Never,
        (Some(CaptureQuantifier::One), None) => Use::Once,
        (Some(CaptureQuantifier::ZeroOrOne), None) => Use::Maybe,
        _ => Use::Many,
    }
}

/// Whether `pattern` of a tags query captures exactly one name and one definition or reference.
fn check_tag_pattern(
    compiled: &Query,
    pattern: usize,
    captures: &[TagCapture],
) -> Result<(), &'static str> {
    if use_of(compiled, pattern, captures, |capture| {
        capture == TagCapture::Name
    }) != Use::Once
    {
        return Err("needs exactly one @name");
    }
    let is_tag = |capture: TagCapture| {
        matches!(
            capture,
            TagCapture::Definition(_) | TagCapture::Reference(_)
        )
    };
    if use_of(compiled, pattern, captures, is_tag) != Use::Once {
        return Err("needs exactly one @definition or @reference capture");
    }
    Ok(())
}

/// Whether `pattern` of an imports query captures exactly one statement and one module, and at
/// most one of each of the other captures.
fn check_import_pattern(
    compiled: &Query,
    pattern: usize,
    captures: &[ImportCapture],
) -> Result<(), &'static str> {
    let use_of_sort =
        |sort: ImportCapture| use_of(compiled, pattern, captures, |capture| capture == sort);
    if use_of_sort(ImportCapture::Statement) != Use::Once {
        return Err("needs exactly one @import");
    }
    if use_of_sort(ImportCapture::Module) != Use::Once {
        return Err("needs exactly one @import.module");
    }
    let optional = [
        ImportCapture::Name,
        ImportCapture::Alias,
        ImportCapture::All,
    ];
    if optional
        .into_iter()
        .any(|sort| use_of_sort(sort) == Use::Many)
    {
        return Err("may capture @import.name, @import.alias and @import.all once each at most");
    }
    Ok(())
}

/// The error for `problem` in `pattern` of `query`.
fn pattern_error(
    language: Language,
    query: &'static str,
    pattern: usize,
    problem: &'static str,
) -> GrammarError {
    GrammarError::Pattern {
        language,
        query,
        pattern,
        problem,
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

    /// Every language's queries compile, and each language with parsing rules has a grammar.
    #[test]
    fn every_language_compiles() -> Result<(), GrammarError> {
        let grammars = Grammars::load()?;
        for spec in languages::ALL {
            assert_eq!(
                grammars.get(spec.language).is_some(),
                spec.tags.is_some(),
                "{}",
                spec.language.name()
            );
        }
        Ok(())
    }

    /// Captures and their meaning: the tags format's names, Cortex's imports names, nothing else.
    #[test]
    fn capture_names_have_one_meaning() {
        assert_eq!(tag_capture("name"), Some(TagCapture::Name));
        assert_eq!(
            tag_capture("definition.class"),
            Some(TagCapture::Definition(SymbolKind::Class))
        );
        assert_eq!(
            tag_capture("reference.type"),
            Some(TagCapture::Reference(RefKind::TypeRef))
        );
        for unknown in [
            "definition.widget",
            "reference.",
            "doc",
            "names",
            "local.scope",
        ] {
            assert_eq!(tag_capture(unknown), None, "{unknown}");
        }
        assert_eq!(import_capture("import.alias"), Some(ImportCapture::Alias));
        assert_eq!(import_capture("import.names"), None);
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
        let refused = |query, capture: &str| GrammarError::Capture {
            language: Language::Python,
            query,
            capture: capture.to_owned(),
        };
        assert_eq!(error_of(&EXTRA), Some(refused(TAGS, "extra")));
        assert_eq!(error_of(&KIND), Some(refused(TAGS, "definition.widget")));
        assert_eq!(error_of(&IMPORT), Some(refused(IMPORTS, "import.source")));
    }

    /// A tags pattern needs exactly one name and one definition or reference.
    #[test]
    fn tag_patterns_need_a_name_and_a_kind() {
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
        static NO_KIND: TagsSpec = spec(
            &[
                "(class_definition name: (identifier) @name) @definition.class",
                "(function_definition name: (identifier) @name)",
            ],
            IMPORTS_QUERY,
        );
        static MAYBE_NAME: TagsSpec = spec(
            &["(call function: [(identifier) @name (attribute)]) @reference.call"],
            IMPORTS_QUERY,
        );
        let problem = |pattern, problem| GrammarError::Pattern {
            language: Language::Python,
            query: TAGS,
            pattern,
            problem,
        };
        assert_eq!(
            error_of(&NO_NAME),
            Some(problem(0, "needs exactly one @name"))
        );
        assert_eq!(
            error_of(&TWO_NAMES),
            Some(problem(0, "needs exactly one @name"))
        );
        assert_eq!(
            error_of(&MAYBE_NAME),
            Some(problem(0, "needs exactly one @name"))
        );
        assert_eq!(
            error_of(&NO_KIND),
            Some(problem(
                1,
                "needs exactly one @definition or @reference capture"
            ))
        );
    }

    /// An imports pattern needs the statement and the module once, and the rest at most once:
    /// a name or the mark for all names in one match and not the other is fine.
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
        assert_eq!(error_of(&NAME_OR_ALL), None);
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
        assert_eq!(
            error_of(&MANY_NAMES),
            Some(problem(
                "may capture @import.name, @import.alias and @import.all once each at most"
            ))
        );
    }

    /// Predicates Cortex does not evaluate are refused by name; text predicates are accepted.
    #[test]
    fn unevaluated_predicates_are_refused() {
        static IS_NOT: TagsSpec = spec(
            &["((identifier) @name @reference.call (#is-not? local))"],
            IMPORTS_QUERY,
        );
        static SET: TagsSpec = spec(
            &["((identifier) @name @reference.call (#set! kind \"call\"))"],
            IMPORTS_QUERY,
        );
        static STRIP: TagsSpec = spec(
            &["((identifier) @name @reference.call (#strip! @name \"_\"))"],
            IMPORTS_QUERY,
        );
        static TEXT: TagsSpec = spec(
            &["((identifier) @name @reference.call (#not-any-of? @name \"print\" \"len\"))"],
            IMPORTS_QUERY,
        );
        let refused = |operator: &str| GrammarError::Predicate {
            language: Language::Python,
            query: TAGS,
            operator: operator.to_owned(),
        };
        assert_eq!(error_of(&IS_NOT), Some(refused("is-not?")));
        assert_eq!(error_of(&SET), Some(refused("set!")));
        assert_eq!(error_of(&STRIP), Some(refused("strip!")));
        assert_eq!(error_of(&TEXT), None);
    }
}
