//! Which language a file is in.
//!
//! `linguist-language` in `.gitattributes` decides first, as GitHub's linguist reads it: a
//! language Cortex parses, or one it does not, which leaves the file without a language. Then the
//! file's whole name (`Gemfile`), then its extension, then the interpreter its `#!` line names.
//! A `.h` header is C unless its first bytes hold something only C++ has.

use cortex_core::{Language, RepoPath};

use crate::languages::{self, Spec};

/// Texts that mark a `.h` header as C++: keywords and syntax C does not have.
const CPP_HEADER_MARKS: &[&[u8]] = &[
    b"namespace ",
    b"class ",
    b"template<",
    b"template <",
    b"std::",
    b"public:",
    b"private:",
    b"protected:",
];

/// What `.gitattributes` says a file's language is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageSetting {
    /// A language Cortex parses.
    Is(Language),
    /// A language Cortex does not parse: the file has none.
    Unsupported,
}

/// The language a `linguist-language` value names, compared without regard to case.
#[must_use]
pub fn linguist_language(name: &str) -> LanguageSetting {
    let name = name.to_ascii_lowercase();
    find(|spec| spec.linguist_names.contains(&name.as_str()))
        .map_or(LanguageSetting::Unsupported, LanguageSetting::Is)
}

/// The language of the file at `path`, given its first bytes when they were read and what its
/// attributes say.
#[must_use]
pub fn detect(
    path: &RepoPath,
    start: Option<&[u8]>,
    setting: Option<LanguageSetting>,
) -> Option<Language> {
    match setting {
        Some(LanguageSetting::Is(language)) => return Some(language),
        Some(LanguageSetting::Unsupported) => return None,
        None => {}
    }
    let name = path.file_name();
    if let Some(language) = find(|spec| spec.file_names.contains(&name)) {
        return Some(language);
    }
    if let Some(extension) = path.extension().map(str::to_ascii_lowercase) {
        if extension == "h" {
            return Some(header_language(start));
        }
        if let Some(language) = find(|spec| spec.extensions.contains(&extension.as_str())) {
            return Some(language);
        }
    }
    let interpreter = start.and_then(interpreter)?;
    find(|spec| spec.interpreters.contains(&interpreter.as_str()))
}

/// The first language whose spec passes `test`.
fn find(test: impl Fn(&Spec) -> bool) -> Option<Language> {
    languages::ALL
        .iter()
        .find(|spec| test(spec))
        .map(|spec| spec.language)
}

/// C++ when the first bytes of a header hold a C++ mark, otherwise C.
fn header_language(start: Option<&[u8]>) -> Language {
    let is_cpp = start.is_some_and(|start| {
        CPP_HEADER_MARKS
            .iter()
            .any(|mark| start.windows(mark.len()).any(|window| window == *mark))
    });
    if is_cpp { Language::Cpp } else { Language::C }
}

/// The interpreter a `#!` line at the very start names, without its folder or version number:
/// `python` for `#!/usr/bin/env python3.11`, `deno` for `#!/usr/bin/env -S deno run`.
fn interpreter(start: &[u8]) -> Option<String> {
    let line = start
        .strip_prefix(b"#!")?
        .split(|byte| *byte == b'\n')
        .next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut words = line.split_whitespace();
    let mut name = words.next()?.rsplit('/').next()?;
    if name == "env" {
        // Options such as -S and assignments such as FOO=1 come before the program.
        name = words.find(|word| !word.starts_with('-') && !word.contains('='))?;
    }
    let bare =
        name.trim_end_matches(|character: char| character.is_ascii_digit() || character == '.');
    Some(bare.to_owned())
}

#[cfg(test)]
mod tests {
    use cortex_core::PathError;

    use super::*;

    /// The language detected for `path` with no first bytes and no attributes.
    fn by_name(path: &str) -> Result<Option<Language>, PathError> {
        Ok(detect(&RepoPath::new(path)?, None, None))
    }

    /// Extensions, in any case, and whole file names pick the language; unknown ones give none.
    #[test]
    fn names_and_extensions() -> Result<(), PathError> {
        let cases = [
            ("src/app.ts", Some(Language::TypeScript)),
            ("src/types.d.ts", Some(Language::TypeScript)),
            ("ui/App.tsx", Some(Language::Tsx)),
            ("lib/a.MJS", Some(Language::JavaScript)),
            ("tool.py", Some(Language::Python)),
            ("main.go", Some(Language::Go)),
            ("lib.rs", Some(Language::Rust)),
            ("A.java", Some(Language::Java)),
            ("a.c", Some(Language::C)),
            ("a.hpp", Some(Language::Cpp)),
            ("a.cs", Some(Language::CSharp)),
            ("Gemfile", Some(Language::Ruby)),
            ("index.php", Some(Language::Php)),
            ("build.gradle.kts", Some(Language::Kotlin)),
            ("a.swift", Some(Language::Swift)),
            ("build.sbt", Some(Language::Scala)),
            (".bashrc", Some(Language::Bash)),
            ("run.sh", Some(Language::Bash)),
            ("package.json", Some(Language::Json)),
            (".eslintrc", Some(Language::Json)),
            ("ci.yml", Some(Language::Yaml)),
            ("Pipfile", Some(Language::Toml)),
            ("schema.sql", Some(Language::Sql)),
            ("README.md", Some(Language::Markdown)),
            ("notes.txt", None),
            ("Makefile", None),
            ("bashrc.bak", None),
        ];
        for (path, expected) in cases {
            assert_eq!(by_name(path)?, expected, "{path}");
        }
        Ok(())
    }

    /// Every language in the registry, once, in the order the core type lists them.
    #[test]
    fn registry_covers_every_language() {
        let registered: Vec<Language> = languages::ALL.iter().map(|spec| spec.language).collect();
        assert_eq!(registered, Language::ALL);
    }

    /// A file without a known name or extension takes the language its `#!` line names, past
    /// `env`, its options and assignments, folders and version numbers.
    #[test]
    fn shebangs() -> Result<(), PathError> {
        let cases: [(&[u8], Option<Language>); 8] = [
            (
                b"#!/usr/bin/env python3.11\nprint(1)\n",
                Some(Language::Python),
            ),
            (b"#!/bin/bash -e\n", Some(Language::Bash)),
            (b"#! /bin/sh\n", Some(Language::Bash)),
            (
                b"#!/usr/bin/env -S deno run --allow-net\n",
                Some(Language::TypeScript),
            ),
            (
                b"#!/usr/bin/env NODE_ENV=prod node\n",
                Some(Language::JavaScript),
            ),
            (b"#!/usr/bin/ruby\r\n", Some(Language::Ruby)),
            (b"#!/usr/bin/perl\n", None),
            (b"echo no shebang\n", None),
        ];
        let path = RepoPath::new("bin/tool")?;
        for (start, expected) in cases {
            assert_eq!(
                detect(&path, Some(start), None),
                expected,
                "{}",
                String::from_utf8_lossy(start)
            );
        }
        // A known extension wins over the shebang.
        let script = RepoPath::new("run.sh")?;
        assert_eq!(
            detect(&script, Some(b"#!/usr/bin/env python\n"), None),
            Some(Language::Bash)
        );
        Ok(())
    }

    /// A `.h` header is C++ when it holds a C++ mark, and C otherwise or when unread.
    #[test]
    fn headers() -> Result<(), PathError> {
        let header = RepoPath::new("include/a.h")?;
        let cpp: &[u8] = b"#pragma once\nnamespace app {\nclass Widget;\n}\n";
        let c: &[u8] = b"#ifndef A_H\n#define A_H\nint add(int a, int b);\n#endif\n";
        assert_eq!(detect(&header, Some(cpp), None), Some(Language::Cpp));
        assert_eq!(detect(&header, Some(c), None), Some(Language::C));
        assert_eq!(detect(&header, None, None), Some(Language::C));
        Ok(())
    }

    /// What the attributes say wins: a language Cortex parses, or one it does not, which leaves
    /// the file without a language. Linguist names compare without regard to case.
    #[test]
    fn attributes_decide_first() -> Result<(), PathError> {
        let header = RepoPath::new("include/a.h")?;
        let python = RepoPath::new("tool.py")?;
        assert_eq!(linguist_language("C++"), LanguageSetting::Is(Language::Cpp));
        assert_eq!(
            linguist_language("Shell"),
            LanguageSetting::Is(Language::Bash)
        );
        assert_eq!(linguist_language("Groovy"), LanguageSetting::Unsupported);
        assert_eq!(
            detect(&header, None, Some(linguist_language("C++"))),
            Some(Language::Cpp)
        );
        assert_eq!(detect(&python, None, Some(linguist_language("Text"))), None);
        Ok(())
    }
}
