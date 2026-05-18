// Language-specific extractors
//
// This module provides access to tree-sitter grammars for 26 supported languages.
// All grammars are compiled into the binary at build time via their respective crates.
//
// Note on Kotlin: The tree-sitter-kotlin crate (0.3.x) depends on tree-sitter 0.20.x
// which causes duplicate C symbol linker errors when combined with tree-sitter 0.24.x.
// Kotlin remains regex-based until a compatible grammar crate is available.
// SQL and Perl also remain regex-based.

pub mod bash;
pub mod c_lang;
pub mod cpp;
pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod go;
pub mod haskell;
pub mod java;
pub mod julia;
pub mod kotlin;
pub mod lua;
pub mod objc;
pub mod ocaml;
pub mod perl;
pub mod php;
pub mod python;
pub mod r_lang;
pub mod ruby;
pub mod rust_lang;
pub mod scala;
pub mod sql;
pub mod swift;
pub mod terraform;
pub mod typescript;
pub mod yaml;
pub mod zig;

use tree_sitter::Language;

/// Returns the tree-sitter Language for Python.
pub fn python() -> Language {
    tree_sitter_python::LANGUAGE.into()
}

/// Returns the tree-sitter Language for TypeScript.
pub fn typescript() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}

/// Returns the tree-sitter Language for TSX.
pub fn tsx() -> Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}

/// Returns the tree-sitter Language for JavaScript.
pub fn javascript() -> Language {
    tree_sitter_javascript::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Go.
pub fn go() -> Language {
    tree_sitter_go::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Rust.
pub fn rust() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Java.
pub fn java() -> Language {
    tree_sitter_java::LANGUAGE.into()
}

/// Returns the tree-sitter Language for C#.
pub fn csharp() -> Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}

/// Returns the tree-sitter Language for C++.
pub fn cpp() -> Language {
    tree_sitter_cpp::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Ruby.
pub fn ruby() -> Language {
    tree_sitter_ruby::LANGUAGE.into()
}

/// Returns the tree-sitter Language for C.
pub fn c() -> Language {
    tree_sitter_c::LANGUAGE.into()
}

/// Returns the tree-sitter Language for PHP.
pub fn php() -> Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}

/// Returns the tree-sitter Language for Scala.
pub fn scala() -> Language {
    tree_sitter_scala::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Swift.
pub fn swift() -> Language {
    tree_sitter_swift::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Dart.
pub fn dart() -> Language {
    tree_sitter_dart::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Elixir.
pub fn elixir() -> Language {
    tree_sitter_elixir::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Haskell.
pub fn haskell() -> Language {
    tree_sitter_haskell::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Lua.
pub fn lua() -> Language {
    tree_sitter_lua::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Zig.
pub fn zig() -> Language {
    tree_sitter_zig::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Bash.
pub fn bash() -> Language {
    tree_sitter_bash::LANGUAGE.into()
}

/// Returns the tree-sitter Language for R.
pub fn r() -> Language {
    tree_sitter_r::LANGUAGE.into()
}

/// Returns the tree-sitter Language for Objective-C.
pub fn objc() -> Language {
    tree_sitter_objc::LANGUAGE.into()
}

/// Returns the tree-sitter Language for OCaml.
pub fn ocaml() -> Language {
    tree_sitter_ocaml::LANGUAGE_OCAML.into()
}

/// Returns the tree-sitter Language for OCaml interface files (.mli).
pub fn ocaml_interface() -> Language {
    tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into()
}

/// Returns the tree-sitter Language for Julia.
pub fn julia() -> Language {
    tree_sitter_julia::LANGUAGE.into()
}

/// Returns the tree-sitter Language for HCL (Terraform).
pub fn hcl() -> Language {
    tree_sitter_hcl::LANGUAGE.into()
}

/// Returns the tree-sitter Language for YAML.
pub fn yaml() -> Language {
    tree_sitter_yaml::LANGUAGE.into()
}

/// All supported language names.
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "python",
    "typescript",
    "tsx",
    "javascript",
    "go",
    "rust",
    "java",
    "csharp",
    "cpp",
    "ruby",
    "c",
    "php",
    "scala",
    "swift",
    "dart",
    "elixir",
    "haskell",
    "lua",
    "zig",
    "bash",
    "r",
    "objc",
    "ocaml",
    "julia",
    "terraform",
    "yaml",
];

/// Returns the tree-sitter Language for a given language name.
/// Returns None if the language is not supported or uses regex-based extraction
/// (Kotlin, SQL, Perl remain regex-based and return None here).
pub fn language_for_name(name: &str) -> Option<Language> {
    match name {
        "python" | "py" => Some(python()),
        "typescript" | "ts" => Some(typescript()),
        "tsx" => Some(tsx()),
        "javascript" | "js" => Some(javascript()),
        "go" => Some(go()),
        "rust" | "rs" => Some(rust()),
        "java" => Some(java()),
        "csharp" | "c_sharp" | "c#" | "cs" => Some(csharp()),
        "cpp" | "c++" | "cxx" => Some(cpp()),
        "ruby" | "rb" => Some(ruby()),
        "c" => Some(c()),
        "php" => Some(php()),
        "scala" => Some(scala()),
        "swift" => Some(swift()),
        "dart" => Some(dart()),
        "elixir" => Some(elixir()),
        "haskell" | "hs" => Some(haskell()),
        "lua" => Some(lua()),
        "zig" => Some(zig()),
        "bash" | "sh" => Some(bash()),
        "r" => Some(r()),
        "objc" | "objective-c" => Some(objc()),
        "ocaml" | "ml" => Some(ocaml()),
        "julia" | "jl" => Some(julia()),
        "terraform" | "hcl" | "tf" => Some(hcl()),
        "yaml" | "yml" => Some(yaml()),
        _ => None,
    }
}

/// Maps a file extension to a language name.
/// Returns None if the extension is not recognized.
pub fn language_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "py" | "pyi" => Some("python"),
        "ts" => Some("typescript"),
        "tsx" | "astro" => Some("tsx"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("tsx"),
        "go" => Some("go"),
        "rs" => Some("rust"),
        "java" => Some("java"),
        "cs" => Some("csharp"),
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => Some("cpp"),
        "rb" => Some("ruby"),
        "c" | "h" => Some("c"),
        "scala" | "sc" => Some("scala"),
        "swift" => Some("swift"),
        "php" => Some("php"),
        "sql" => Some("sql"),
        "kt" | "kts" => Some("kotlin"),
        "dart" => Some("dart"),
        "ex" | "exs" => Some("elixir"),
        "hs" | "lhs" => Some("haskell"),
        "lua" => Some("lua"),
        "zig" => Some("zig"),
        "sh" | "bash" | "zsh" => Some("bash"),
        "pl" | "pm" => Some("perl"),
        "r" | "R" => Some("r"),
        "m" => Some("objc"),
        "ml" | "mli" => Some("ocaml"),
        "jl" => Some("julia"),
        "tf" | "hcl" => Some("terraform"),
        "yml" | "yaml" => Some("yaml"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_languages_load() {
        // Verify all 26 language grammars are compiled and accessible
        let mut parser = tree_sitter::Parser::new();

        // Original 11 languages
        parser.set_language(&python()).expect("Python grammar should load");
        parser.set_language(&typescript()).expect("TypeScript grammar should load");
        parser.set_language(&tsx()).expect("TSX grammar should load");
        parser.set_language(&javascript()).expect("JavaScript grammar should load");
        parser.set_language(&go()).expect("Go grammar should load");
        parser.set_language(&rust()).expect("Rust grammar should load");
        parser.set_language(&java()).expect("Java grammar should load");
        parser.set_language(&csharp()).expect("C# grammar should load");
        parser.set_language(&cpp()).expect("C++ grammar should load");
        parser.set_language(&ruby()).expect("Ruby grammar should load");
        parser.set_language(&c()).expect("C grammar should load");

        // 15 new languages
        parser.set_language(&php()).expect("PHP grammar should load");
        parser.set_language(&scala()).expect("Scala grammar should load");
        parser.set_language(&swift()).expect("Swift grammar should load");
        parser.set_language(&dart()).expect("Dart grammar should load");
        parser.set_language(&elixir()).expect("Elixir grammar should load");
        parser.set_language(&haskell()).expect("Haskell grammar should load");
        parser.set_language(&lua()).expect("Lua grammar should load");
        parser.set_language(&zig()).expect("Zig grammar should load");
        parser.set_language(&bash()).expect("Bash grammar should load");
        parser.set_language(&r()).expect("R grammar should load");
        parser.set_language(&objc()).expect("Objective-C grammar should load");
        parser.set_language(&ocaml()).expect("OCaml grammar should load");
        parser.set_language(&ocaml_interface()).expect("OCaml interface grammar should load");
        parser.set_language(&julia()).expect("Julia grammar should load");
        parser.set_language(&hcl()).expect("HCL grammar should load");
        parser.set_language(&yaml()).expect("YAML grammar should load");
    }

    #[test]
    fn test_language_for_name() {
        // Original 11 languages
        assert!(language_for_name("python").is_some());
        assert!(language_for_name("py").is_some());
        assert!(language_for_name("typescript").is_some());
        assert!(language_for_name("ts").is_some());
        assert!(language_for_name("tsx").is_some());
        assert!(language_for_name("javascript").is_some());
        assert!(language_for_name("js").is_some());
        assert!(language_for_name("go").is_some());
        assert!(language_for_name("rust").is_some());
        assert!(language_for_name("rs").is_some());
        assert!(language_for_name("java").is_some());
        assert!(language_for_name("csharp").is_some());
        assert!(language_for_name("c_sharp").is_some());
        assert!(language_for_name("c#").is_some());
        assert!(language_for_name("cs").is_some());
        assert!(language_for_name("cpp").is_some());
        assert!(language_for_name("c++").is_some());
        assert!(language_for_name("cxx").is_some());
        assert!(language_for_name("ruby").is_some());
        assert!(language_for_name("rb").is_some());
        assert!(language_for_name("c").is_some());

        // 15 new tree-sitter languages
        assert!(language_for_name("php").is_some());
        assert!(language_for_name("scala").is_some());
        assert!(language_for_name("swift").is_some());
        assert!(language_for_name("dart").is_some());
        assert!(language_for_name("elixir").is_some());
        assert!(language_for_name("haskell").is_some());
        assert!(language_for_name("hs").is_some());
        assert!(language_for_name("lua").is_some());
        assert!(language_for_name("zig").is_some());
        assert!(language_for_name("bash").is_some());
        assert!(language_for_name("sh").is_some());
        assert!(language_for_name("r").is_some());
        assert!(language_for_name("objc").is_some());
        assert!(language_for_name("objective-c").is_some());
        assert!(language_for_name("ocaml").is_some());
        assert!(language_for_name("ml").is_some());
        assert!(language_for_name("julia").is_some());
        assert!(language_for_name("jl").is_some());
        assert!(language_for_name("terraform").is_some());
        assert!(language_for_name("hcl").is_some());
        assert!(language_for_name("tf").is_some());
        assert!(language_for_name("yaml").is_some());
        assert!(language_for_name("yml").is_some());

        // Regex-based languages return None (triggers regex fallback)
        assert!(language_for_name("kotlin").is_none());
        assert!(language_for_name("sql").is_none());
        assert!(language_for_name("perl").is_none());

        // Unknown languages return None
        assert!(language_for_name("unknown").is_none());
    }

    #[test]
    fn test_language_for_extension() {
        assert_eq!(language_for_extension("py"), Some("python"));
        assert_eq!(language_for_extension("ts"), Some("typescript"));
        assert_eq!(language_for_extension("tsx"), Some("tsx"));
        assert_eq!(language_for_extension("js"), Some("javascript"));
        assert_eq!(language_for_extension("go"), Some("go"));
        assert_eq!(language_for_extension("rs"), Some("rust"));
        assert_eq!(language_for_extension("java"), Some("java"));
        assert_eq!(language_for_extension("cs"), Some("csharp"));
        assert_eq!(language_for_extension("cpp"), Some("cpp"));
        assert_eq!(language_for_extension("rb"), Some("ruby"));
        assert_eq!(language_for_extension("c"), Some("c"));
        assert_eq!(language_for_extension("h"), Some("c"));
        // Regex-based languages
        assert_eq!(language_for_extension("scala"), Some("scala"));
        assert_eq!(language_for_extension("sc"), Some("scala"));
        assert_eq!(language_for_extension("swift"), Some("swift"));
        assert_eq!(language_for_extension("php"), Some("php"));
        assert_eq!(language_for_extension("sql"), Some("sql"));
        assert_eq!(language_for_extension("kt"), Some("kotlin"));
        assert_eq!(language_for_extension("kts"), Some("kotlin"));
        assert_eq!(language_for_extension("dart"), Some("dart"));
        assert_eq!(language_for_extension("ex"), Some("elixir"));
        assert_eq!(language_for_extension("exs"), Some("elixir"));
        assert_eq!(language_for_extension("hs"), Some("haskell"));
        assert_eq!(language_for_extension("lua"), Some("lua"));
        assert_eq!(language_for_extension("zig"), Some("zig"));
        assert_eq!(language_for_extension("sh"), Some("bash"));
        assert_eq!(language_for_extension("bash"), Some("bash"));
        assert_eq!(language_for_extension("pl"), Some("perl"));
        assert_eq!(language_for_extension("pm"), Some("perl"));
        assert_eq!(language_for_extension("r"), Some("r"));
        assert_eq!(language_for_extension("R"), Some("r"));
        assert_eq!(language_for_extension("m"), Some("objc"));
        assert_eq!(language_for_extension("ml"), Some("ocaml"));
        assert_eq!(language_for_extension("mli"), Some("ocaml"));
        assert_eq!(language_for_extension("jl"), Some("julia"));
        assert_eq!(language_for_extension("tf"), Some("terraform"));
        assert_eq!(language_for_extension("hcl"), Some("terraform"));
        assert_eq!(language_for_extension("yml"), Some("yaml"));
        assert_eq!(language_for_extension("yaml"), Some("yaml"));
        assert_eq!(language_for_extension("unknown"), None);
    }

    #[test]
    fn test_python_parses_code() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&python()).unwrap();
        let tree = parser.parse("def hello(): pass", None).unwrap();
        let root = tree.root_node();
        assert_eq!(root.kind(), "module");
        assert!(root.child_count() > 0);
    }

    #[test]
    fn test_typescript_parses_code() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&typescript()).unwrap();
        let tree = parser.parse("function hello(): void {}", None).unwrap();
        let root = tree.root_node();
        assert_eq!(root.kind(), "program");
        assert!(root.child_count() > 0);
    }

    #[test]
    fn test_rust_parses_code() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&rust()).unwrap();
        let tree = parser.parse("fn main() {}", None).unwrap();
        let root = tree.root_node();
        assert_eq!(root.kind(), "source_file");
        assert!(root.child_count() > 0);
    }

    #[test]
    fn test_c_parses_code() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c()).unwrap();
        let tree = parser.parse("int main() { return 0; }", None).unwrap();
        let root = tree.root_node();
        assert_eq!(root.kind(), "translation_unit");
        assert!(root.child_count() > 0);
    }

    #[test]
    fn test_java_parses_code() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&java()).unwrap();
        let tree = parser
            .parse("class Hello { void greet() {} }", None)
            .unwrap();
        let root = tree.root_node();
        assert_eq!(root.kind(), "program");
        assert!(root.child_count() > 0);
    }

    #[test]
    fn test_go_parses_code() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go()).unwrap();
        let tree = parser
            .parse("package main\nfunc main() {}", None)
            .unwrap();
        let root = tree.root_node();
        assert_eq!(root.kind(), "source_file");
        assert!(root.child_count() > 0);
    }
}
