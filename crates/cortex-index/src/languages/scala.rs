//! Scala: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Scala files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Scala,
    extensions: &["scala", "sc", "sbt"],
    file_names: &[],
    interpreters: &["scala"],
    linguist_names: &["scala"],
    tags: None,
};
