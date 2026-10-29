//! The languages Cortex parses, one file each, and how a file is recognised as one of them.
//!
//! Adding a language means adding its file and one line in [`ALL`].

use cortex_core::Language;

mod bash;
mod c;
mod cpp;
mod csharp;
mod go;
mod java;
mod javascript;
mod json;
mod kotlin;
mod markdown;
mod php;
mod python;
mod ruby;
mod rust;
mod scala;
mod sql;
mod swift;
mod toml;
mod tsx;
mod typescript;
mod yaml;

/// How files of one language are recognised.
#[derive(Debug)]
pub struct Spec {
    /// The language.
    pub language: Language,
    /// File name extensions, lower case and without the dot.
    pub extensions: &'static [&'static str],
    /// Whole file names that mark the language, such as `Gemfile`.
    pub file_names: &'static [&'static str],
    /// Interpreters a `#!` line may name, without version numbers, such as `python`.
    pub interpreters: &'static [&'static str],
    /// Names `linguist-language` may give in `.gitattributes`, lower case, spaces as hyphens.
    pub linguist_names: &'static [&'static str],
}

/// Every language, in the order [`Language::ALL`] lists them.
pub const ALL: &[&Spec] = &[
    &typescript::SPEC,
    &tsx::SPEC,
    &javascript::SPEC,
    &python::SPEC,
    &go::SPEC,
    &rust::SPEC,
    &java::SPEC,
    &c::SPEC,
    &cpp::SPEC,
    &csharp::SPEC,
    &ruby::SPEC,
    &php::SPEC,
    &kotlin::SPEC,
    &swift::SPEC,
    &scala::SPEC,
    &bash::SPEC,
    &json::SPEC,
    &yaml::SPEC,
    &toml::SPEC,
    &sql::SPEC,
    &markdown::SPEC,
];
