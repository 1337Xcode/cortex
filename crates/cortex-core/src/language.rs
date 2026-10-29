//! The languages Cortex knows, with one stable name each.
//!
//! Tier A languages get symbols and references; Tier B formats get file-level indexing plus their
//! top-level keys or headings. Every other file is indexed at file level only and has no
//! [`Language`]. The names are stored in the index, so they never change.

/// How deeply a language is indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Symbols, references and imports.
    A,
    /// File level plus top-level keys or headings.
    B,
}

/// A language or format Cortex parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    /// TypeScript, `.ts`.
    TypeScript,
    /// TypeScript with JSX, `.tsx`.
    Tsx,
    /// JavaScript, including JSX.
    JavaScript,
    /// Python.
    Python,
    /// Go.
    Go,
    /// Rust.
    Rust,
    /// Java.
    Java,
    /// C.
    C,
    /// C++.
    Cpp,
    /// C#.
    CSharp,
    /// Ruby.
    Ruby,
    /// PHP.
    Php,
    /// Kotlin.
    Kotlin,
    /// Swift.
    Swift,
    /// Scala.
    Scala,
    /// Bash and other POSIX shell scripts.
    Bash,
    /// JSON.
    Json,
    /// YAML.
    Yaml,
    /// TOML.
    Toml,
    /// SQL.
    Sql,
    /// Markdown.
    Markdown,
}

impl Language {
    /// Every language, Tier A first, in a fixed order.
    pub const ALL: [Self; 21] = [
        Self::TypeScript,
        Self::Tsx,
        Self::JavaScript,
        Self::Python,
        Self::Go,
        Self::Rust,
        Self::Java,
        Self::C,
        Self::Cpp,
        Self::CSharp,
        Self::Ruby,
        Self::Php,
        Self::Kotlin,
        Self::Swift,
        Self::Scala,
        Self::Bash,
        Self::Json,
        Self::Yaml,
        Self::Toml,
        Self::Sql,
        Self::Markdown,
    ];

    /// The stored name, lower case: `typescript`, `cpp`, `csharp` and so on.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
            Self::Ruby => "ruby",
            Self::Php => "php",
            Self::Kotlin => "kotlin",
            Self::Swift => "swift",
            Self::Scala => "scala",
            Self::Bash => "bash",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Sql => "sql",
            Self::Markdown => "markdown",
        }
    }

    /// How deeply this language is indexed.
    #[must_use]
    pub fn tier(self) -> Tier {
        match self {
            Self::Json | Self::Yaml | Self::Toml | Self::Sql | Self::Markdown => Tier::B,
            _ => Tier::A,
        }
    }

    /// The language with this stored name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sixteen Tier A languages and five Tier B formats, each with a distinct name that reads
    /// back to it.
    #[test]
    fn names_are_distinct_and_read_back() {
        let tier_a = Language::ALL
            .iter()
            .filter(|language| language.tier() == Tier::A)
            .count();
        assert_eq!(tier_a, 16);
        assert_eq!(Language::ALL.len() - tier_a, 5);
        let mut names: Vec<&str> = Language::ALL
            .iter()
            .map(|language| language.name())
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 21);
        for language in Language::ALL {
            assert_eq!(Language::from_name(language.name()), Some(language));
        }
        assert_eq!(Language::from_name("cobol"), None);
    }
}
