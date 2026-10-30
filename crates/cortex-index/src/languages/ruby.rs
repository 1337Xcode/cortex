//! Ruby: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How Ruby files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Ruby,
    extensions: &["rb", "rake", "gemspec", "ru", "rbw"],
    file_names: &[
        "Gemfile",
        "Rakefile",
        "Vagrantfile",
        "Guardfile",
        "Podfile",
        "Brewfile",
        "Capfile",
    ],
    interpreters: &["ruby", "jruby"],
    linguist_names: &["ruby", "rb", "jruby"],
    tags: TagsSpec {
        grammar,
        // The grammar's locals patterns first, as tree-sitter's tags format reads them: they find
        // the local variables that keep a bare name such as `total` from being read as a call.
        // Then Cortex's patterns, which read functions, constants, base classes and mixins, and
        // last the grammar's tags query.
        queries: &[
            tree_sitter_ruby::LOCALS_QUERY,
            include_str!("../../queries/ruby-tags.scm"),
            tree_sitter_ruby::TAGS_QUERY,
        ],
        imports: include_str!("../../queries/ruby-imports.scm"),
        // Classes, modules and methods have a body, a constant its value.
        body_paths: &[&["body"], &["right"]],
        // Minitest's and Test::Unit's file names, and RSpec's.
        test_files: &[
            NamePattern::Suffix("_test.rb"),
            NamePattern::Suffix("_spec.rb"),
            NamePattern::Prefix("test_"),
        ],
        // The methods Minitest and Test::Unit run as tests; RSpec's examples are blocks, not
        // methods, and are marked by their file alone.
        test_functions: &[NamePattern::Prefix("test_")],
    },
};

/// The Ruby grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_ruby::LANGUAGE.into()
}
