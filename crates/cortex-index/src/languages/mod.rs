//! The languages Cortex parses, one file each: how a file is recognised as one of them, and how
//! it is parsed.
//!
//! Adding a language means adding its file and one line in [`ALL`].

use cortex_core::{Language, SymbolKind};

use crate::path_rules::NamePattern;

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

#[cfg(test)]
pub(crate) use python::grammar as python_grammar;

/// How files of one language are recognised and parsed.
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
    /// How the language's files are parsed into definitions, references and imports.
    pub tags: TagsSpec,
}

/// How a language's files are parsed into definitions, references and imports.
#[derive(Debug)]
pub struct TagsSpec {
    /// The tree-sitter grammar.
    pub grammar: fn() -> tree_sitter::Language,
    /// The tags queries, compiled as one in this order: the grammar's own and Cortex's additions.
    /// Where two patterns find the same name the earlier one wins, so additions that should
    /// override the grammar's own patterns come before them.
    pub queries: &'static [&'static str],
    /// The query that finds each module a file imports, with what it takes and the name it binds.
    pub imports: &'static str,
    /// The grammar's forms that reach a name through something written before it, each as the
    /// kind of node holding both and the field of the part before the name: a `member_expression`
    /// reaches its `property` through its `object`. A use whose name follows that part in one of
    /// these is qualified by it, so an import that binds the bare name does not bind it. Every
    /// language whose imports name files lists its forms; one whose imports name none may list
    /// none, since its uses are tied by name alone.
    pub qualified_by: &'static [(&'static str, &'static str)],
    /// The kinds of definition whose bodies see none of the names of the scopes around them, only
    /// their own and what they import: Rust's modules, which reach the names around them through
    /// paths such as `super::`. A path an import reads from the module it is made in starts from
    /// the innermost of these around it.
    pub closed_scopes: &'static [SymbolKind],
    /// The paths of grammar fields that lead from a definition's node to its body, tried in order.
    /// A definition's header, which its signature hash covers, runs up to the last part before its
    /// body, leaving out comments in between; a definition no path leads to a body from is all
    /// header.
    pub body_paths: &'static [&'static [&'static str]],
    /// Names of files that hold tests, besides every file in a test folder.
    pub test_files: &'static [NamePattern],
    /// Names of the functions and methods in a test file that are tests.
    pub test_functions: &'static [NamePattern],
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

#[cfg(test)]
mod tests {
    use cortex_core::{PathError, RepoPath};

    use super::*;
    use crate::path_rules::is_test_file;

    /// Each language's names for test files, besides the test folders every language shares.
    #[test]
    fn test_file_names() -> Result<(), PathError> {
        let cases = [
            (Language::Python, "shop/test_orders.py", true),
            (Language::Python, "shop/orders_test.py", true),
            (Language::Python, "shop/orders.py", false),
            (Language::JavaScript, "web/cart.test.js", true),
            (Language::JavaScript, "web/cart.spec.jsx", true),
            (Language::JavaScript, "web/cart.test.mjs", true),
            (Language::JavaScript, "web/cart.spec.cjs", true),
            (Language::JavaScript, "web/cart.js", false),
            (Language::JavaScript, "web/test.js", false),
            (Language::TypeScript, "web/cart.test.ts", true),
            (Language::TypeScript, "web/cart.spec.mts", true),
            (Language::TypeScript, "web/cart.test.js", false),
            (Language::TypeScript, "web/cart.ts", false),
            (Language::Tsx, "web/Cart.test.tsx", true),
            (Language::Tsx, "web/Cart.spec.tsx", true),
            (Language::Tsx, "web/Cart.test.ts", false),
            (Language::Tsx, "web/Cart.tsx", false),
            (Language::Go, "shop/pay_test.go", true),
            (Language::Go, "shop/pay.go", false),
            (Language::Go, "shop/test_pay.go", false),
            (Language::Rust, "tests/orders.rs", true),
            (Language::Rust, "src/orders_test.rs", false),
            (Language::Java, "shop/OrderTest.java", true),
            (Language::Java, "shop/OrderTests.java", true),
            (Language::Java, "shop/OrderIT.java", true),
            (Language::Java, "shop/Order.java", false),
            (Language::Java, "shop/TestOrder.java", false),
            (Language::Ruby, "lib/order_test.rb", true),
            (Language::Ruby, "lib/order_spec.rb", true),
            (Language::Ruby, "lib/test_order.rb", true),
            (Language::Ruby, "lib/order.rb", false),
            (Language::Ruby, "lib/order_tests.rb", false),
            (Language::C, "src/test_order.c", true),
            (Language::C, "src/order_test.c", true),
            (Language::C, "include/test_order.h", true),
            (Language::C, "src/order.c", false),
            (Language::C, "src/order_tests.c", false),
            (Language::Cpp, "src/order_test.cc", true),
            (Language::Cpp, "src/order_test.cpp", true),
            (Language::Cpp, "src/order_unittest.cc", true),
            (Language::Cpp, "src/order_test.cxx", true),
            (Language::Cpp, "src/order_tests.cpp", true),
            (Language::Cpp, "src/test_order.cpp", true),
            (Language::Cpp, "src/order.cpp", false),
            (Language::Cpp, "src/OrderTest.cpp", false),
            (Language::CSharp, "src/OrderTests.cs", true),
            (Language::CSharp, "src/OrderTest.cs", true),
            (Language::CSharp, "src/Order.cs", false),
            (Language::CSharp, "src/TestOrder.cs", false),
            (Language::Php, "src/OrderTest.php", true),
            (Language::Php, "src/Order.php", false),
            (Language::Php, "src/TestOrder.php", false),
            (Language::Kotlin, "src/OrderTest.kt", true),
            (Language::Kotlin, "src/OrderTests.kt", true),
            (Language::Kotlin, "src/OrderIT.kt", true),
            (Language::Kotlin, "src/Order.kt", false),
            (Language::Kotlin, "src/TestOrder.kt", false),
            (Language::Kotlin, "build.gradle.kts", false),
            (Language::Swift, "Sources/OrderTests.swift", true),
            (Language::Swift, "Sources/OrderTest.swift", true),
            (Language::Swift, "Sources/Order.swift", false),
            (Language::Swift, "Sources/TestOrder.swift", false),
            (Language::Scala, "src/OrderSpec.scala", true),
            (Language::Scala, "src/OrderTest.scala", true),
            (Language::Scala, "src/OrderSuite.scala", true),
            (Language::Scala, "src/OrderTests.scala", true),
            (Language::Scala, "src/OrderIT.scala", true),
            (Language::Scala, "src/Order.scala", false),
            (Language::Scala, "src/TestOrder.scala", false),
            (Language::Bash, "scripts/deploy_test.sh", true),
            (Language::Bash, "scripts/deploy_test.bash", true),
            (Language::Bash, "scripts/deploy_test.zsh", true),
            (Language::Bash, "scripts/deploy_test.ksh", true),
            (Language::Bash, "scripts/test_deploy.sh", true),
            (Language::Bash, "scripts/deploy.sh", false),
            (Language::Bash, "scripts/deploy_tests.sh", false),
        ];
        for (language, path, expected) in cases {
            let names = ALL
                .iter()
                .find(|spec| spec.language == language)
                .map_or(&[][..], |spec| spec.tags.test_files);
            assert_eq!(
                is_test_file(&RepoPath::new(path)?, names),
                expected,
                "{path}"
            );
        }
        Ok(())
    }
}
