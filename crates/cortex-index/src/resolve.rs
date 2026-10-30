//! Tying references to definitions by name, when the name alone settles it: the one definition
//! with that name in the reference's own file, or else the one definition with that name among
//! the files of the reference's language family. Anything less certain stays unresolved.
//!
//! A family is a group of languages whose code calls each other's definitions directly:
//! TypeScript, TSX and JavaScript; C and C++; Java, Kotlin and Scala. Every other language is a
//! family of its own, so a Python call is never tied to a function of another language that
//! happens to share its name.

use std::collections::BTreeMap;

use cortex_core::{Language, Resolution};
use cortex_store::{FileId, RefName, Resolved, StoreError, SymbolId, SymbolName, Target, Writer};

/// How many references are written per transaction.
const RESOLUTIONS_PER_TRANSACTION: usize = 4096;

/// What a name finds among some definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    /// Exactly one definition.
    One(SymbolId),
    /// More than one.
    Several,
}

/// The definitions of each name, per file and per language family.
struct Definitions<'a> {
    /// By file and name.
    in_file: BTreeMap<(FileId, &'a str), Found>,
    /// By language family and name.
    in_family: BTreeMap<(Language, &'a str), Found>,
}

impl<'a> Definitions<'a> {
    /// The definitions among `symbols`.
    fn of(symbols: &'a [SymbolName]) -> Self {
        let mut definitions = Self {
            in_file: BTreeMap::new(),
            in_family: BTreeMap::new(),
        };
        for symbol in symbols {
            let name = symbol.name.as_str();
            count(&mut definitions.in_file, (symbol.file, name), symbol.id);
            if let Some(language) = symbol.language {
                count(
                    &mut definitions.in_family,
                    (family(language), name),
                    symbol.id,
                );
            }
        }
        definitions
    }

    /// What `reference` is tied to: the one definition with its name in its file, else the one
    /// with its name in its language family, else nothing.
    fn target(&self, reference: &RefName) -> Option<Target> {
        let name = reference.name.as_str();
        // A name defined more than once in the file is defined more than once in its family too.
        if let Some(found) = self.in_file.get(&(reference.file, name)) {
            return tie(*found, Resolution::SameFile);
        }
        let family = family(reference.language?);
        tie(*self.in_family.get(&(family, name))?, Resolution::NameOnly)
    }
}

/// Adds the definition `symbol` under `key`.
fn count<K: Ord>(found: &mut BTreeMap<K, Found>, key: K, symbol: SymbolId) {
    found
        .entry(key)
        .and_modify(|found| *found = Found::Several)
        .or_insert(Found::One(symbol));
}

/// The target `found` gives with `resolution`: one, or none when the name is ambiguous.
fn tie(found: Found, resolution: Resolution) -> Option<Target> {
    match found {
        Found::One(symbol) => Some(Target { symbol, resolution }),
        Found::Several => None,
    }
}

/// The language family `language` belongs to, named by its first member.
fn family(language: Language) -> Language {
    match language {
        Language::Tsx | Language::JavaScript => Language::TypeScript,
        Language::Cpp => Language::C,
        Language::Kotlin | Language::Scala => Language::Java,
        other => other,
    }
}

/// Ties every stored reference to the definition its name settles, or marks it unresolved, and
/// writes the references whose target changed, in transactions of
/// [`RESOLUTIONS_PER_TRANSACTION`].
///
/// # Errors
///
/// Store errors; the transactions committed before stay written.
pub(crate) fn resolve_names(writer: &mut Writer) -> Result<(), StoreError> {
    let queries = writer.queries();
    let symbols = queries.symbol_names()?;
    let refs = queries.ref_names()?;
    let definitions = Definitions::of(&symbols);
    let changed: Vec<Resolved> = refs
        .iter()
        .filter_map(|reference| {
            let target = definitions.target(reference);
            (target != reference.target).then_some(Resolved {
                reference: reference.id,
                target,
            })
        })
        .collect();
    for batch in changed.chunks(RESOLUTIONS_PER_TRANSACTION) {
        let mut transaction = writer.transaction()?;
        transaction.set_resolutions(batch)?;
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Languages that call each other's definitions directly share a family; the others stand
    /// alone.
    #[test]
    fn families_join_languages_that_call_each_other() {
        let cases = [
            (Language::TypeScript, Language::TypeScript),
            (Language::Tsx, Language::TypeScript),
            (Language::JavaScript, Language::TypeScript),
            (Language::C, Language::C),
            (Language::Cpp, Language::C),
            (Language::Java, Language::Java),
            (Language::Kotlin, Language::Java),
            (Language::Scala, Language::Java),
            (Language::Python, Language::Python),
            (Language::Go, Language::Go),
            (Language::Sql, Language::Sql),
        ];
        for (language, expected) in cases {
            assert_eq!(family(language), expected, "{language:?}");
        }
    }
}
