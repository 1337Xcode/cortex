//! What each pattern of a tags or imports query may capture, and what a tags pattern does.
//!
//! A tags pattern is one of four shapes. A locals pattern captures only `@local.scope`,
//! `@local.definition` and `@local.reference`, and may say with `#set! local.scope-inherits false`
//! that names defined around its scope are not local inside it. Every other tags pattern captures
//! a `@name` with one definition or reference, which records a tag (a definition may also capture a
//! `@qualifier` and a `@body`), a `@name` alone, which records nothing, or an `@ignore` alone; it
//! may say with `#is-not? local` that a name defined in a scope around it is not recorded. An
//! imports pattern captures the statement and the module once and its other captures once at
//! most, and tests and sets no property.

use tree_sitter::{CaptureQuantifier, Query};

use crate::grammar::{ImportCapture, TagCapture};

/// What a pattern of a tags query does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatternRule {
    /// It finds scopes and the names defined in them, for `#is-not? local` to consult.
    Locals {
        /// Whether names defined in the scopes around this one are local in it too.
        scope_inherits: bool,
    },
    /// It records a tag, keeps a name from being recorded, or records nothing.
    Tags {
        /// Whether a name defined in a scope around it is left out: `#is-not? local`.
        is_not_local: bool,
    },
}

/// What is wrong with a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Problem {
    /// Its captures do not form a shape the query allows; the text says how.
    Shape(&'static str),
    /// It uses a property predicate or setting Cortex does not evaluate there, by name.
    Predicate(&'static str),
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

/// What `pattern` of a tags query does, given what each capture marks.
pub(crate) fn tag_pattern_rule(
    compiled: &Query,
    pattern: usize,
    captures: &[TagCapture],
) -> Result<PatternRule, Problem> {
    let is_local = |capture: TagCapture| {
        matches!(
            capture,
            TagCapture::LocalScope | TagCapture::LocalDefinition | TagCapture::LocalReference
        )
    };
    let is_other = |capture: TagCapture| {
        !is_local(capture) && !matches!(capture, TagCapture::Doc | TagCapture::Helper)
    };
    let locals = use_of(compiled, pattern, captures, is_local);
    if locals == Use::Never {
        check_tag_shape(compiled, pattern, captures).map_err(Problem::Shape)?;
        return tags_rule(compiled, pattern);
    }
    if use_of(compiled, pattern, captures, is_other) != Use::Never {
        return Err(Problem::Shape("may not mix @local captures with others"));
    }
    locals_rule(compiled, pattern)
}

/// The rule of a locals pattern: the only property it may set is `local.scope-inherits`.
fn locals_rule(compiled: &Query, pattern: usize) -> Result<PatternRule, Problem> {
    if let Some((_, is_positive)) = compiled.property_predicates(pattern).first() {
        return Err(Problem::Predicate(test_operator(*is_positive)));
    }
    let mut scope_inherits = true;
    for setting in compiled.property_settings(pattern) {
        match (setting.key.as_ref(), setting.value.as_deref()) {
            ("local.scope-inherits", Some("false")) => scope_inherits = false,
            ("local.scope-inherits", Some("true")) => scope_inherits = true,
            _ => return Err(Problem::Predicate("set!")),
        }
    }
    Ok(PatternRule::Locals { scope_inherits })
}

/// The rule of a pattern that records tags: the only property it may test is `#is-not? local`.
fn tags_rule(compiled: &Query, pattern: usize) -> Result<PatternRule, Problem> {
    if !compiled.property_settings(pattern).is_empty() {
        return Err(Problem::Predicate("set!"));
    }
    let mut is_not_local = false;
    for (property, is_positive) in compiled.property_predicates(pattern) {
        if *is_positive || property.key.as_ref() != "local" || property.value.is_some() {
            return Err(Problem::Predicate(test_operator(*is_positive)));
        }
        is_not_local = true;
    }
    Ok(PatternRule::Tags { is_not_local })
}

/// The operator of a property test: `is?`, or `is-not?` when the test is negated.
fn test_operator(is_positive: bool) -> &'static str {
    if is_positive { "is?" } else { "is-not?" }
}

/// Whether `pattern` of a tags query, not a locals one, has one of the three shapes the format
/// allows: a name with one definition or reference, a definition perhaps with a qualifier and a
/// body; a name alone; or an `@ignore` alone.
fn check_tag_shape(
    compiled: &Query,
    pattern: usize,
    captures: &[TagCapture],
) -> Result<(), &'static str> {
    let use_of_sort =
        |is_sort: fn(TagCapture) -> bool| use_of(compiled, pattern, captures, is_sort);
    let ignore = use_of_sort(|capture| capture == TagCapture::Ignore);
    let name = use_of_sort(|capture| capture == TagCapture::Name);
    let definition =
        use_of_sort(|capture| matches!(capture, TagCapture::Definition(_) | TagCapture::Macro));
    let reference = use_of_sort(|capture| matches!(capture, TagCapture::Reference(_)));
    let qualifier = use_of_sort(|capture| capture == TagCapture::Qualifier);
    let body = use_of_sort(|capture| capture == TagCapture::Body);
    if ignore != Use::Never {
        let others = [name, definition, reference, qualifier, body];
        if ignore == Use::Once && others.iter().all(|other| *other == Use::Never) {
            return Ok(());
        }
        return Err("may capture nothing beside one @ignore");
    }
    if name != Use::Once {
        return Err("needs exactly one @name");
    }
    match (definition, reference) {
        (Use::Once, Use::Never) => {}
        (Use::Never, Use::Once | Use::Never) => {
            if qualifier != Use::Never {
                return Err("may capture a @qualifier only with a @definition");
            }
            if body != Use::Never {
                return Err("may capture a @body only with a @definition");
            }
        }
        _ => return Err("needs at most one @definition or @reference capture"),
    }
    if qualifier == Use::Many {
        return Err("may capture one @qualifier at most");
    }
    if body == Use::Many {
        return Err("may capture one @body at most");
    }
    Ok(())
}

/// Whether `pattern` of an imports query captures exactly one statement and one module, at most
/// one of each of the other captures, and no property.
pub(crate) fn check_import_pattern(
    compiled: &Query,
    pattern: usize,
    captures: &[ImportCapture],
) -> Result<(), Problem> {
    if let Some((_, is_positive)) = compiled.property_predicates(pattern).first() {
        return Err(Problem::Predicate(test_operator(*is_positive)));
    }
    if !compiled.property_settings(pattern).is_empty() {
        return Err(Problem::Predicate("set!"));
    }
    let use_of_sort =
        |sort: ImportCapture| use_of(compiled, pattern, captures, |capture| capture == sort);
    if use_of_sort(ImportCapture::Statement) != Use::Once {
        return Err(Problem::Shape("needs exactly one @import"));
    }
    if use_of_sort(ImportCapture::Module) != Use::Once {
        return Err(Problem::Shape("needs exactly one @import.module"));
    }
    let optional = [
        ImportCapture::ModuleEnd,
        ImportCapture::Name,
        ImportCapture::Alias,
        ImportCapture::All,
        ImportCapture::Default,
    ];
    if optional
        .into_iter()
        .any(|sort| use_of_sort(sort) == Use::Many)
    {
        return Err(Problem::Shape(
            "may capture @import.module.end, @import.name, @import.alias, @import.all and \
             @import.default once each at most",
        ));
    }
    Ok(())
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
