// Parser dispatcher for source files.
//
// Determines the language from the file extension, configures a tree-sitter
// parser, and returns the parsed syntax tree along with the identified language.

use std::path::Path;

use tree_sitter::Tree;

use crate::error::ParseError;
use crate::indexer::languages;

/// Supported programming languages for parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupportedLanguage {
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Go,
    Rust,
    Java,
    CSharp,
    Cpp,
    Ruby,
    C,
    Scala,
    Swift,
    Php,
    Sql,
    Kotlin,
    Dart,
    Elixir,
    Haskell,
    Lua,
    Zig,
    Bash,
    Perl,
    R,
    ObjectiveC,
    OCaml,
    Julia,
    Terraform,
    Yaml,
}

impl SupportedLanguage {
    /// Returns the canonical name string for this language.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::Cpp => "cpp",
            Self::Ruby => "ruby",
            Self::C => "c",
            Self::Scala => "scala",
            Self::Swift => "swift",
            Self::Php => "php",
            Self::Sql => "sql",
            Self::Kotlin => "kotlin",
            Self::Dart => "dart",
            Self::Elixir => "elixir",
            Self::Haskell => "haskell",
            Self::Lua => "lua",
            Self::Zig => "zig",
            Self::Bash => "bash",
            Self::Perl => "perl",
            Self::R => "r",
            Self::ObjectiveC => "objc",
            Self::OCaml => "ocaml",
            Self::Julia => "julia",
            Self::Terraform => "terraform",
            Self::Yaml => "yaml",
        }
    }

    /// Returns true if this language has a tree-sitter grammar available.
    pub fn has_grammar(&self) -> bool {
        matches!(
            self,
            Self::Python
                | Self::TypeScript
                | Self::Tsx
                | Self::JavaScript
                | Self::Go
                | Self::Rust
                | Self::Java
                | Self::CSharp
                | Self::Cpp
                | Self::Ruby
                | Self::C
                | Self::Php
                | Self::Scala
                | Self::Swift
                | Self::Dart
                | Self::Elixir
                | Self::Haskell
                | Self::Lua
                | Self::Zig
                | Self::Bash
                | Self::R
                | Self::ObjectiveC
                | Self::OCaml
                | Self::Julia
                | Self::Terraform
                | Self::Yaml
        )
    }

    /// Returns true if this language uses regex-based extraction.
    /// Only Kotlin, SQL, and Perl remain regex-based (no compatible tree-sitter grammar).
    pub fn uses_regex_extraction(&self) -> bool {
        matches!(self, Self::Kotlin | Self::Sql | Self::Perl)
    }
}

/// Maps a language name (as returned by `languages::language_for_extension`) to
/// the corresponding `SupportedLanguage` enum variant.
fn language_name_to_enum(name: &str) -> Option<SupportedLanguage> {
    match name {
        "python" => Some(SupportedLanguage::Python),
        "typescript" => Some(SupportedLanguage::TypeScript),
        "tsx" => Some(SupportedLanguage::Tsx),
        "javascript" => Some(SupportedLanguage::JavaScript),
        "go" => Some(SupportedLanguage::Go),
        "rust" => Some(SupportedLanguage::Rust),
        "java" => Some(SupportedLanguage::Java),
        "csharp" => Some(SupportedLanguage::CSharp),
        "cpp" => Some(SupportedLanguage::Cpp),
        "ruby" => Some(SupportedLanguage::Ruby),
        "c" => Some(SupportedLanguage::C),
        "scala" => Some(SupportedLanguage::Scala),
        "swift" => Some(SupportedLanguage::Swift),
        "php" => Some(SupportedLanguage::Php),
        "sql" => Some(SupportedLanguage::Sql),
        "kotlin" => Some(SupportedLanguage::Kotlin),
        "dart" => Some(SupportedLanguage::Dart),
        "elixir" => Some(SupportedLanguage::Elixir),
        "haskell" => Some(SupportedLanguage::Haskell),
        "lua" => Some(SupportedLanguage::Lua),
        "zig" => Some(SupportedLanguage::Zig),
        "bash" => Some(SupportedLanguage::Bash),
        "perl" => Some(SupportedLanguage::Perl),
        "r" => Some(SupportedLanguage::R),
        "objc" => Some(SupportedLanguage::ObjectiveC),
        "ocaml" => Some(SupportedLanguage::OCaml),
        "julia" => Some(SupportedLanguage::Julia),
        "terraform" => Some(SupportedLanguage::Terraform),
        "yaml" => Some(SupportedLanguage::Yaml),
        _ => None,
    }
}

/// Parse a source file, returning the identified language and the syntax tree.
///
/// The language is determined from the file extension. Tree-sitter always produces
/// a tree even for syntactically invalid files (with ERROR nodes in the tree).
/// `ParseError::ParseFailed` is only returned if tree-sitter's `Parser::parse()`
/// returns `None`, which is rare (usually only for cancellation or timeout).
///
/// # Errors
///
/// - `ParseError::UnsupportedLanguage` if the file extension is not recognized.
/// - `ParseError::ParseFailed` if tree-sitter fails to produce a tree at all.
pub fn parse(path: &Path, source: &str) -> Result<(SupportedLanguage, Tree), ParseError> {
    // Extract extension from path
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    // Map extension to language name
    let lang_name = languages::language_for_extension(extension).ok_or_else(|| {
        ParseError::UnsupportedLanguage {
            extension: extension.to_string(),
        }
    })?;

    // Map language name to enum
    let supported_lang =
        language_name_to_enum(lang_name).ok_or_else(|| ParseError::UnsupportedLanguage {
            extension: extension.to_string(),
        })?;

    // For regex-based languages without tree-sitter grammars, return UnsupportedLanguage
    // (they are handled directly in the pipeline via regex extraction)
    if !supported_lang.has_grammar() {
        return Err(ParseError::UnsupportedLanguage {
            extension: extension.to_string(),
        });
    }

    // Get the tree-sitter Language grammar
    let ts_language =
        languages::language_for_name(lang_name).ok_or_else(|| ParseError::UnsupportedLanguage {
            extension: extension.to_string(),
        })?;

    // Configure parser and parse
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&ts_language)
        .map_err(|_| ParseError::ParseFailed {
            path: path.display().to_string(),
            partial_tree: false,
        })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| ParseError::ParseFailed {
            path: path.display().to_string(),
            partial_tree: false,
        })?;

    Ok((supported_lang, tree))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // Extension mapping tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_python() {
        let path = PathBuf::from("src/main.py");
        let (lang, tree) = parse(&path, "def hello(): pass").unwrap();
        assert_eq!(lang, SupportedLanguage::Python);
        assert_eq!(tree.root_node().kind(), "module");
    }

    #[test]
    fn test_parse_typescript() {
        let path = PathBuf::from("src/app.ts");
        let (lang, tree) = parse(&path, "function greet(): void {}").unwrap();
        assert_eq!(lang, SupportedLanguage::TypeScript);
        assert_eq!(tree.root_node().kind(), "program");
    }

    #[test]
    fn test_parse_tsx() {
        let path = PathBuf::from("src/App.tsx");
        let (lang, tree) = parse(&path, "const App = () => <div/>;").unwrap();
        assert_eq!(lang, SupportedLanguage::Tsx);
        assert_eq!(tree.root_node().kind(), "program");
    }

    #[test]
    fn test_parse_javascript() {
        let path = PathBuf::from("src/index.js");
        let (lang, tree) = parse(&path, "function main() {}").unwrap();
        assert_eq!(lang, SupportedLanguage::JavaScript);
        assert_eq!(tree.root_node().kind(), "program");
    }

    #[test]
    fn test_parse_jsx() {
        // .jsx maps to tsx grammar (JavaScript with JSX support)
        let path = PathBuf::from("src/Component.jsx");
        let (lang, tree) = parse(&path, "const C = () => <div/>;").unwrap();
        assert_eq!(lang, SupportedLanguage::Tsx);
        assert_eq!(tree.root_node().kind(), "program");
    }

    #[test]
    fn test_parse_go() {
        let path = PathBuf::from("main.go");
        let (lang, tree) = parse(&path, "package main\nfunc main() {}").unwrap();
        assert_eq!(lang, SupportedLanguage::Go);
        assert_eq!(tree.root_node().kind(), "source_file");
    }

    #[test]
    fn test_parse_rust() {
        let path = PathBuf::from("src/lib.rs");
        let (lang, tree) = parse(&path, "fn main() {}").unwrap();
        assert_eq!(lang, SupportedLanguage::Rust);
        assert_eq!(tree.root_node().kind(), "source_file");
    }

    #[test]
    fn test_parse_java() {
        let path = PathBuf::from("src/Main.java");
        let (lang, tree) = parse(&path, "class Main { void run() {} }").unwrap();
        assert_eq!(lang, SupportedLanguage::Java);
        assert_eq!(tree.root_node().kind(), "program");
    }

    #[test]
    fn test_parse_csharp() {
        let path = PathBuf::from("src/Program.cs");
        let (lang, tree) = parse(&path, "class Program { static void Main() {} }").unwrap();
        assert_eq!(lang, SupportedLanguage::CSharp);
        assert_eq!(tree.root_node().kind(), "compilation_unit");
    }

    #[test]
    fn test_parse_cpp() {
        let path = PathBuf::from("src/main.cpp");
        let (lang, tree) = parse(&path, "int main() { return 0; }").unwrap();
        assert_eq!(lang, SupportedLanguage::Cpp);
        assert_eq!(tree.root_node().kind(), "translation_unit");
    }

    #[test]
    fn test_parse_cpp_cc_extension() {
        let path = PathBuf::from("src/util.cc");
        let (lang, _tree) = parse(&path, "void foo() {}").unwrap();
        assert_eq!(lang, SupportedLanguage::Cpp);
    }

    #[test]
    fn test_parse_cpp_cxx_extension() {
        let path = PathBuf::from("src/util.cxx");
        let (lang, _tree) = parse(&path, "void bar() {}").unwrap();
        assert_eq!(lang, SupportedLanguage::Cpp);
    }

    #[test]
    fn test_parse_cpp_h_extension() {
        // .h maps to C, not C++
        let path = PathBuf::from("include/header.h");
        let (lang, _tree) = parse(&path, "int x;").unwrap();
        assert_eq!(lang, SupportedLanguage::C);
    }

    #[test]
    fn test_parse_cpp_hpp_extension() {
        let path = PathBuf::from("include/header.hpp");
        let (lang, _tree) = parse(&path, "class Foo {};").unwrap();
        assert_eq!(lang, SupportedLanguage::Cpp);
    }

    #[test]
    fn test_parse_ruby() {
        let path = PathBuf::from("app/main.rb");
        let (lang, tree) = parse(&path, "def hello; end").unwrap();
        assert_eq!(lang, SupportedLanguage::Ruby);
        assert_eq!(tree.root_node().kind(), "program");
    }

    #[test]
    fn test_parse_c() {
        let path = PathBuf::from("src/main.c");
        let (lang, tree) = parse(&path, "int main() { return 0; }").unwrap();
        assert_eq!(lang, SupportedLanguage::C);
        assert_eq!(tree.root_node().kind(), "translation_unit");
    }

    // -----------------------------------------------------------------------
    // Error case tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_unsupported_extension_returns_error() {
        let path = PathBuf::from("data/config.toml");
        let result = parse(&path, "key = \"value\"");
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::UnsupportedLanguage { extension } => {
                assert_eq!(extension, "toml");
            }
            other => panic!("expected UnsupportedLanguage, got: {other:?}"),
        }
    }

    #[test]
    fn test_unsupported_extension_txt() {
        let path = PathBuf::from("README.txt");
        let result = parse(&path, "hello world");
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::UnsupportedLanguage { extension } => {
                assert_eq!(extension, "txt");
            }
            other => panic!("expected UnsupportedLanguage, got: {other:?}"),
        }
    }

    #[test]
    fn test_no_extension_returns_error() {
        let path = PathBuf::from("Makefile");
        let result = parse(&path, "all: build");
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::UnsupportedLanguage { extension } => {
                assert_eq!(extension, "");
            }
            other => panic!("expected UnsupportedLanguage, got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Invalid syntax tests - tree-sitter still returns a tree with ERROR nodes
    // -----------------------------------------------------------------------

    #[test]
    fn test_broken_python_returns_tree_with_errors() {
        let path = PathBuf::from("broken.py");
        let source = "def foo(:\n    pass pass pass @@@ !!!";
        let (lang, tree) = parse(&path, source).unwrap();
        assert_eq!(lang, SupportedLanguage::Python);
        // Tree is returned even for invalid syntax
        let root = tree.root_node();
        assert_eq!(root.kind(), "module");
        // The tree should have ERROR nodes
        assert!(root.has_error());
    }

    #[test]
    fn test_broken_typescript_returns_tree_with_errors() {
        let path = PathBuf::from("broken.ts");
        let source = "function {{{ invalid syntax !!!";
        let (lang, tree) = parse(&path, source).unwrap();
        assert_eq!(lang, SupportedLanguage::TypeScript);
        let root = tree.root_node();
        assert!(root.has_error());
    }

    #[test]
    fn test_broken_rust_returns_tree_with_errors() {
        let path = PathBuf::from("broken.rs");
        let source = "fn main( { let x = @@@; }";
        let (lang, tree) = parse(&path, source).unwrap();
        assert_eq!(lang, SupportedLanguage::Rust);
        let root = tree.root_node();
        assert!(root.has_error());
    }

    // -----------------------------------------------------------------------
    // SupportedLanguage enum tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_supported_language_as_str() {
        assert_eq!(SupportedLanguage::Python.as_str(), "python");
        assert_eq!(SupportedLanguage::TypeScript.as_str(), "typescript");
        assert_eq!(SupportedLanguage::Tsx.as_str(), "tsx");
        assert_eq!(SupportedLanguage::JavaScript.as_str(), "javascript");
        assert_eq!(SupportedLanguage::Go.as_str(), "go");
        assert_eq!(SupportedLanguage::Rust.as_str(), "rust");
        assert_eq!(SupportedLanguage::Java.as_str(), "java");
        assert_eq!(SupportedLanguage::CSharp.as_str(), "csharp");
        assert_eq!(SupportedLanguage::Cpp.as_str(), "cpp");
        assert_eq!(SupportedLanguage::Ruby.as_str(), "ruby");
        assert_eq!(SupportedLanguage::C.as_str(), "c");
        assert_eq!(SupportedLanguage::Scala.as_str(), "scala");
        assert_eq!(SupportedLanguage::Swift.as_str(), "swift");
        assert_eq!(SupportedLanguage::Php.as_str(), "php");
        assert_eq!(SupportedLanguage::Sql.as_str(), "sql");
        assert_eq!(SupportedLanguage::Kotlin.as_str(), "kotlin");
        assert_eq!(SupportedLanguage::Dart.as_str(), "dart");
        assert_eq!(SupportedLanguage::Elixir.as_str(), "elixir");
        assert_eq!(SupportedLanguage::Haskell.as_str(), "haskell");
        assert_eq!(SupportedLanguage::Lua.as_str(), "lua");
        assert_eq!(SupportedLanguage::Zig.as_str(), "zig");
        assert_eq!(SupportedLanguage::Bash.as_str(), "bash");
        assert_eq!(SupportedLanguage::Perl.as_str(), "perl");
        assert_eq!(SupportedLanguage::R.as_str(), "r");
        assert_eq!(SupportedLanguage::ObjectiveC.as_str(), "objc");
        assert_eq!(SupportedLanguage::OCaml.as_str(), "ocaml");
        assert_eq!(SupportedLanguage::Julia.as_str(), "julia");
        assert_eq!(SupportedLanguage::Terraform.as_str(), "terraform");
        assert_eq!(SupportedLanguage::Yaml.as_str(), "yaml");
    }

    #[test]
    fn test_stub_languages_return_unsupported() {
        // Only Kotlin, SQL, and Perl remain regex-based and return UnsupportedLanguage
        // (they are handled via regex extraction in the pipeline)
        let kotlin_path = PathBuf::from("src/Main.kt");
        let result = parse(&kotlin_path, "fun main() {}");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::UnsupportedLanguage { .. }
        ));

        let sql_path = PathBuf::from("migrations/001.sql");
        let result = parse(&sql_path, "CREATE TABLE users (id INT);");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::UnsupportedLanguage { .. }
        ));

        let perl_path = PathBuf::from("lib/App.pm");
        let result = parse(&perl_path, "package App;");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::UnsupportedLanguage { .. }
        ));
    }

    // -----------------------------------------------------------------------
    // Tree-sitter parsing tests for newly supported languages
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_php() {
        let path = PathBuf::from("src/index.php");
        let (lang, tree) = parse(&path, "<?php echo 'hello'; ?>").unwrap();
        assert_eq!(lang, SupportedLanguage::Php);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_scala() {
        let path = PathBuf::from("src/Main.scala");
        let (lang, tree) = parse(&path, "object Main { def main(): Unit = {} }").unwrap();
        assert_eq!(lang, SupportedLanguage::Scala);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_swift() {
        let path = PathBuf::from("Sources/App.swift");
        let (lang, tree) = parse(&path, "class App { func run() {} }").unwrap();
        assert_eq!(lang, SupportedLanguage::Swift);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_dart() {
        let path = PathBuf::from("lib/main.dart");
        let (lang, tree) = parse(&path, "void main() {}").unwrap();
        assert_eq!(lang, SupportedLanguage::Dart);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_elixir() {
        let path = PathBuf::from("lib/app.ex");
        let (lang, tree) = parse(&path, "defmodule App do\n  def hello, do: :world\nend").unwrap();
        assert_eq!(lang, SupportedLanguage::Elixir);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_haskell() {
        let path = PathBuf::from("src/Main.hs");
        let (lang, tree) = parse(&path, "main :: IO ()\nmain = putStrLn \"hello\"").unwrap();
        assert_eq!(lang, SupportedLanguage::Haskell);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_lua() {
        let path = PathBuf::from("src/main.lua");
        let (lang, tree) = parse(&path, "function main()\n  print(\"hello\")\nend").unwrap();
        assert_eq!(lang, SupportedLanguage::Lua);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_zig() {
        let path = PathBuf::from("src/main.zig");
        let (lang, tree) = parse(&path, "const std = @import(\"std\");").unwrap();
        assert_eq!(lang, SupportedLanguage::Zig);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_bash() {
        let path = PathBuf::from("scripts/deploy.sh");
        let (lang, tree) = parse(&path, "#!/bin/bash\necho hello").unwrap();
        assert_eq!(lang, SupportedLanguage::Bash);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_r() {
        let path = PathBuf::from("analysis/script.r");
        let (lang, tree) = parse(&path, "hello <- function() { print(\"hi\") }").unwrap();
        assert_eq!(lang, SupportedLanguage::R);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_objc() {
        let path = PathBuf::from("src/App.m");
        let (lang, tree) = parse(&path, "@implementation App\n- (void)run {}\n@end").unwrap();
        assert_eq!(lang, SupportedLanguage::ObjectiveC);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_ocaml() {
        let path = PathBuf::from("src/main.ml");
        let (lang, tree) = parse(&path, "let () = print_endline \"hello\"").unwrap();
        assert_eq!(lang, SupportedLanguage::OCaml);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_julia() {
        let path = PathBuf::from("src/main.jl");
        let (lang, tree) = parse(&path, "function hello()\n  println(\"hi\")\nend").unwrap();
        assert_eq!(lang, SupportedLanguage::Julia);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_terraform() {
        let path = PathBuf::from("main.tf");
        let (lang, tree) = parse(
            &path,
            "resource \"aws_instance\" \"web\" {\n  ami = \"abc\"\n}",
        )
        .unwrap();
        assert_eq!(lang, SupportedLanguage::Terraform);
        assert!(!tree.root_node().kind().is_empty());
    }

    #[test]
    fn test_parse_yaml() {
        let path = PathBuf::from("config.yml");
        let (lang, tree) = parse(&path, "key: value\nlist:\n  - item1").unwrap();
        assert_eq!(lang, SupportedLanguage::Yaml);
        assert!(!tree.root_node().kind().is_empty());
    }
}
