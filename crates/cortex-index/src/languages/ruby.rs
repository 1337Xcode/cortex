//! Ruby: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Ruby files are recognised.
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
};
