//! What `cfg` predicates and test attributes say about whether code compiles outside tests.
//!
//! A predicate is evaluated with `test` off and every other option unknown, in three values: it
//! holds in every build, in none without `test`, or it depends on the platform, a feature or
//! another option. Code compiles only for tests when an attribute named `test` or `bench` sits on
//! it, or a `cfg` on it is false whenever `test` is off. `cfg_attr` never makes code test-only,
//! but it can choose a module's file, so its `path` values are read here too.

use anyhow::{Context, anyhow, bail};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;

/// The value a `cfg` predicate takes while `test` is off and every other option is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Truth {
    /// Holds in every build.
    True,
    /// Holds in no build without `test`.
    False,
    /// Depends on the platform, a feature or another option.
    Unknown,
}

/// One `cfg` predicate: a boolean literal or an option such as `test`, `unix` or `all(...)`.
pub(crate) enum Predicate {
    /// `true` or `false`.
    Literal(bool),
    /// An option name, `key = "value"` pair or combinator.
    Option(Box<syn::Meta>),
}

impl Parse for Predicate {
    /// Reads a boolean literal or a meta item.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(syn::LitBool) {
            Ok(Self::Literal(input.parse::<syn::LitBool>()?.value))
        } else {
            Ok(Self::Option(Box::new(input.parse()?)))
        }
    }
}

/// The arguments of `cfg_attr`: a predicate, then the attributes it applies.
struct CfgAttr {
    /// When the attributes apply.
    predicate: Predicate,
    /// The attributes applied when the predicate holds.
    attributes: Punctuated<syn::Meta, syn::Token![,]>,
}

impl Parse for CfgAttr {
    /// Reads `predicate, attribute, attribute...`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let predicate = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        let attributes = Punctuated::parse_terminated(input)?;
        Ok(Self {
            predicate,
            attributes,
        })
    }
}

/// The `#[path]` values that may choose a module's file, unconditional ones first, and whether
/// one is unconditional (then the default `name.rs` is never used).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ModulePaths {
    /// Candidate paths, relative to the folder the compiler resolves them from.
    pub(crate) paths: Vec<String>,
    /// Whether some `#[path]` applies in every build.
    pub(crate) is_unconditional: bool,
}

/// Whether attributes make the item they sit on compile only for tests.
pub(crate) fn is_test_only(attributes: &[syn::Attribute]) -> anyhow::Result<bool> {
    for attribute in attributes {
        if is_test_only_meta(&attribute.meta)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether one attribute's content makes its item compile only for tests.
pub(crate) fn is_test_only_meta(meta: &syn::Meta) -> anyhow::Result<bool> {
    let path = meta.path();
    if path.is_ident("cfg") {
        let syn::Meta::List(list) = meta else {
            bail!("a cfg attribute without a predicate");
        };
        let predicate: Predicate = syn::parse2(list.tokens.clone())?;
        return Ok(evaluate(&predicate)? == Truth::False);
    }
    Ok(path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "test" || segment.ident == "bench"))
}

/// The value of a predicate while `test` is off.
pub(crate) fn evaluate(predicate: &Predicate) -> anyhow::Result<Truth> {
    let meta = match predicate {
        Predicate::Literal(true) => return Ok(Truth::True),
        Predicate::Literal(false) => return Ok(Truth::False),
        Predicate::Option(meta) => meta.as_ref(),
    };
    let list = match meta {
        syn::Meta::Path(path) if path.is_ident("test") => return Ok(Truth::False),
        syn::Meta::Path(_) | syn::Meta::NameValue(_) => return Ok(Truth::Unknown),
        syn::Meta::List(list) => list,
    };
    // Other predicates with arguments (`version("1.80")`, `target(...)`) depend on the build.
    if !["all", "any", "not"]
        .iter()
        .any(|name| list.path.is_ident(name))
    {
        return Ok(Truth::Unknown);
    }
    let operands =
        list.parse_args_with(Punctuated::<Predicate, syn::Token![,]>::parse_terminated)?;
    let values = operands
        .iter()
        .map(evaluate)
        .collect::<anyhow::Result<Vec<_>>>()?;
    if list.path.is_ident("all") {
        Ok(if values.contains(&Truth::False) {
            Truth::False
        } else if values.iter().all(|value| *value == Truth::True) {
            Truth::True
        } else {
            Truth::Unknown
        })
    } else if list.path.is_ident("any") {
        Ok(if values.contains(&Truth::True) {
            Truth::True
        } else if values.iter().all(|value| *value == Truth::False) {
            Truth::False
        } else {
            Truth::Unknown
        })
    } else {
        match values.as_slice() {
            [Truth::True] => Ok(Truth::False),
            [Truth::False] => Ok(Truth::True),
            [Truth::Unknown] => Ok(Truth::Unknown),
            _ => Err(anyhow!("`not` takes exactly one predicate")),
        }
    }
}

/// The `#[path]` values among `attributes`, including those inside `cfg_attr` whose predicate
/// can hold without `test`.
pub(crate) fn module_paths(attributes: &[syn::Attribute]) -> anyhow::Result<ModulePaths> {
    let mut unconditional = Vec::new();
    let mut conditional = Vec::new();
    for attribute in attributes {
        collect_paths(
            &attribute.meta,
            Truth::True,
            &mut unconditional,
            &mut conditional,
        )?;
    }
    let is_unconditional = !unconditional.is_empty();
    unconditional.extend(conditional);
    Ok(ModulePaths {
        paths: unconditional,
        is_unconditional,
    })
}

/// Adds the `path` values in `meta`, applied under `condition`, to the matching list.
fn collect_paths(
    meta: &syn::Meta,
    condition: Truth,
    unconditional: &mut Vec<String>,
    conditional: &mut Vec<String>,
) -> anyhow::Result<()> {
    match meta {
        syn::Meta::NameValue(pair) if pair.path.is_ident("path") => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(value),
                ..
            }) = &pair.value
            else {
                bail!("a path attribute whose value is not a string");
            };
            if condition == Truth::True {
                unconditional.push(value.value());
            } else {
                conditional.push(value.value());
            }
        }
        syn::Meta::List(list) if list.path.is_ident("cfg_attr") => {
            let cfg_attr: CfgAttr = list
                .parse_args()
                .context("reading the arguments of cfg_attr")?;
            let truth = evaluate(&cfg_attr.predicate)?;
            if truth == Truth::False {
                return Ok(());
            }
            let inner = if condition == Truth::True && truth == Truth::True {
                Truth::True
            } else {
                Truth::Unknown
            };
            for attribute in &cfg_attr.attributes {
                collect_paths(attribute, inner, unconditional, conditional)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value of the predicate written in `source`.
    fn truth(source: &str) -> anyhow::Result<Truth> {
        evaluate(&syn::parse_str::<Predicate>(source)?)
    }

    /// Each combinator, worked out with `test` false and every other option unknown.
    #[test]
    fn predicates_have_three_values() -> anyhow::Result<()> {
        let cases = [
            ("test", Truth::False),
            ("unix", Truth::Unknown),
            ("feature = \"x\"", Truth::Unknown),
            ("true", Truth::True),
            ("false", Truth::False),
            ("not(test)", Truth::True),
            ("all(test, unix)", Truth::False),
            ("all(unix, not(test))", Truth::Unknown),
            ("all()", Truth::True),
            ("any(test, unix)", Truth::Unknown),
            ("any(test, false)", Truth::False),
            ("any()", Truth::False),
            ("version(\"1.80\")", Truth::Unknown),
        ];
        for (source, expected) in cases {
            assert_eq!(truth(source)?, expected, "{source}");
        }
        assert!(truth("not(test, unix)").is_err());
        Ok(())
    }

    /// `path` values directly and inside `cfg_attr`, with the test-only ones left out.
    #[test]
    fn module_paths_include_cfg_attr() -> anyhow::Result<()> {
        let item: syn::ItemMod = syn::parse_str(
            "#[cfg_attr(windows, path = \"pipe.rs\")] #[cfg_attr(unix, path = \"socket.rs\")] #[cfg_attr(test, path = \"fake.rs\")] mod transport;",
        )?;
        assert_eq!(
            module_paths(&item.attrs)?,
            ModulePaths {
                paths: vec!["pipe.rs".to_owned(), "socket.rs".to_owned()],
                is_unconditional: false,
            }
        );
        let item: syn::ItemMod =
            syn::parse_str("#[cfg_attr(unix, path = \"u.rs\")] #[path = \"p.rs\"] mod m;")?;
        assert_eq!(
            module_paths(&item.attrs)?,
            ModulePaths {
                paths: vec!["p.rs".to_owned(), "u.rs".to_owned()],
                is_unconditional: true,
            }
        );
        Ok(())
    }
}
