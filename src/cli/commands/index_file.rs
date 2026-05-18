//! Implementation of the `cortex index-file <path>` CLI subcommand.
//!
//! Parses a single source file, dispatches to the appropriate language extractor,
//! and prints the resulting `ExtractionResult` as formatted JSON to stdout.

use std::path::Path;

use anyhow::{bail, Context};

use crate::indexer::languages;
use crate::indexer::parser::{self, SupportedLanguage};

/// Run the `index-file` command.
///
/// Reads the file at `path`, parses it with tree-sitter, dispatches to the
/// appropriate language extractor, and prints the `ExtractionResult` as JSON.
///
/// The `repo_root` is used to compute a repo-root-relative file path for FQN
/// construction inside the extractors.
pub fn run(path: &Path, repo_root: &Path) -> Result<(), anyhow::Error> {
    // Verify the file exists.
    if !path.exists() {
        bail!("file not found: {}", path.display());
    }

    // Read file content.
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read file: {}", path.display()))?;

    // Compute repo-root-relative path for FQN construction.
    // Use the canonical paths to handle relative path resolution.
    let abs_path = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf());
    let abs_repo_root = std::fs::canonicalize(repo_root)
        .unwrap_or_else(|_| repo_root.to_path_buf());

    let relative_path = abs_path
        .strip_prefix(&abs_repo_root)
        .unwrap_or(&abs_path);

    // Normalize to forward slashes for consistent FQNs across platforms.
    let relative_str = relative_path
        .to_string_lossy()
        .replace('\\', "/");

    // Parse the file to get language and tree.
    // For regex-based languages, parser::parse() returns UnsupportedLanguage,
    // so we handle them separately.
    let result = match parser::parse(path, &source) {
        Ok((language, tree)) => {
            match language {
                SupportedLanguage::Python => languages::python::extract(&tree, &relative_str, &source),
                SupportedLanguage::TypeScript => {
                    languages::typescript::extract(&tree, &relative_str, &source)
                }
                SupportedLanguage::Tsx => languages::typescript::extract(&tree, &relative_str, &source),
                SupportedLanguage::JavaScript => {
                    languages::typescript::extract(&tree, &relative_str, &source)
                }
                SupportedLanguage::Go => languages::go::extract(&tree, &relative_str, &source),
                SupportedLanguage::Rust => languages::rust_lang::extract(&tree, &relative_str, &source),
                SupportedLanguage::Java => languages::java::extract(&tree, &relative_str, &source),
                SupportedLanguage::CSharp => languages::csharp::extract(&tree, &relative_str, &source),
                SupportedLanguage::Cpp => languages::cpp::extract(&tree, &relative_str, &source),
                SupportedLanguage::Ruby => languages::ruby::extract(&tree, &relative_str, &source),
                SupportedLanguage::C => languages::c_lang::extract(&tree, &relative_str, &source),
                // Regex-based languages (shouldn't reach here, but handle gracefully)
                _ => dispatch_regex(&relative_str, &source, path)?,
            }
        }
        Err(crate::error::ParseError::UnsupportedLanguage { .. }) => {
            // Try regex-based extraction
            dispatch_regex(&relative_str, &source, path)?
        }
        Err(e) => {
            return Err(e.into());
        }
    };

    // Print as formatted JSON to stdout.
    let json = serde_json::to_string_pretty(&result)
        .context("failed to serialize ExtractionResult to JSON")?;
    println!("{json}");

    Ok(())
}

/// Dispatch to regex-based extractors based on file extension.
fn dispatch_regex(
    file: &str,
    source: &str,
    path: &Path,
) -> Result<crate::store::types::ExtractionResult, anyhow::Error> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let result = match ext {
        #[allow(deprecated)]
        "scala" | "sc" => languages::scala::extract_regex(file, source),
        #[allow(deprecated)]
        "swift" => languages::swift::extract_swift(file, source),
        #[allow(deprecated)]
        "php" => languages::php::extract_php(file, source),
        "sql" => languages::sql::extract_sql(file, source),
        "kt" | "kts" => languages::kotlin::extract_regex(file, source),
        "dart" => languages::dart::extract_regex(file, source),
        "ex" | "exs" => languages::elixir::extract_regex(file, source),
        "hs" | "lhs" => languages::haskell::extract_regex(file, source),
        "lua" => languages::lua::extract_regex(file, source),
        "zig" => languages::zig::extract_regex(file, source),
        "sh" | "bash" | "zsh" => languages::bash::extract_regex(file, source),
        "pl" | "pm" => languages::perl::extract_regex(file, source),
        "r" | "R" => languages::r_lang::extract_regex(file, source),
        "m" => languages::objc::extract_regex(file, source),
        "ml" | "mli" => languages::ocaml::extract_regex(file, source),
        "jl" => languages::julia::extract_regex(file, source),
        "tf" | "hcl" => languages::terraform::extract_regex(file, source),
        "yml" | "yaml" => languages::yaml::extract_regex(file, source),
        _ => bail!("unsupported language for extension '.{}'", ext),
    };

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_index_python_fixture() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("example.py");
        fs::write(
            &file_path,
            r#"
def greet(name):
    print(f"Hello, {name}")

class Calculator:
    def add(self, a, b):
        return a + b

    def subtract(self, a, b):
        return a - b
"#,
        )
        .unwrap();

        // Use the temp dir as repo root so relative path is just "example.py"
        let result = run_extract(&file_path, tmp.path());
        assert!(result.is_ok());
        let extraction = result.unwrap();

        // Should have: greet function, Calculator class, add method, subtract method
        assert!(
            extraction.nodes.len() >= 4,
            "expected at least 4 nodes, got {}",
            extraction.nodes.len()
        );

        // Check that FQNs use the relative path
        let fqns: Vec<&str> = extraction.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(
            fqns.iter().any(|f| f.contains("example.py")),
            "FQNs should contain relative file path: {:?}",
            fqns
        );
    }

    #[test]
    fn test_unsupported_extension_error() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("data.xyz");
        fs::write(&file_path, "some content").unwrap();

        let result = run(file_path.as_path(), tmp.path());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unsupported") || err_msg.contains("xyz"),
            "error should mention unsupported extension: {err_msg}"
        );
    }

    #[test]
    fn test_file_not_found_error() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("nonexistent.py");

        let result = run(file_path.as_path(), tmp.path());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found"),
            "error should mention file not found: {err_msg}"
        );
    }

    #[test]
    fn test_invalid_syntax_partial_results() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("broken.py");
        // This file has invalid syntax but also a valid function definition
        fs::write(
            &file_path,
            r#"
def valid_function():
    pass

def broken_function(:::
    @@@!!!

class ValidClass:
    def method(self):
        pass
"#,
        )
        .unwrap();

        // Should still produce partial results (tree-sitter is error-tolerant)
        let result = run_extract(&file_path, tmp.path());
        assert!(result.is_ok());
        let extraction = result.unwrap();

        // Should have at least some nodes from the valid parts
        assert!(
            !extraction.nodes.is_empty(),
            "should produce partial results for invalid syntax"
        );
    }

    /// Helper that runs the extraction logic without printing to stdout.
    fn run_extract(
        path: &Path,
        repo_root: &Path,
    ) -> Result<crate::store::types::ExtractionResult, anyhow::Error> {
        use anyhow::{bail, Context as _};

        if !path.exists() {
            bail!("file not found: {}", path.display());
        }

        let source = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read file: {}", path.display()))?;

        let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let abs_repo_root =
            std::fs::canonicalize(repo_root).unwrap_or_else(|_| repo_root.to_path_buf());

        let relative_path = abs_path.strip_prefix(&abs_repo_root).unwrap_or(&abs_path);
        let relative_str = relative_path.to_string_lossy().replace('\\', "/");

        let result = match parser::parse(path, &source) {
            Ok((language, tree)) => {
                match language {
                    SupportedLanguage::Python => {
                        languages::python::extract(&tree, &relative_str, &source)
                    }
                    SupportedLanguage::TypeScript => {
                        languages::typescript::extract(&tree, &relative_str, &source)
                    }
                    SupportedLanguage::Tsx => {
                        languages::typescript::extract(&tree, &relative_str, &source)
                    }
                    SupportedLanguage::JavaScript => {
                        languages::typescript::extract(&tree, &relative_str, &source)
                    }
                    SupportedLanguage::Go => languages::go::extract(&tree, &relative_str, &source),
                    SupportedLanguage::Rust => {
                        languages::rust_lang::extract(&tree, &relative_str, &source)
                    }
                    SupportedLanguage::Java => languages::java::extract(&tree, &relative_str, &source),
                    SupportedLanguage::CSharp => {
                        languages::csharp::extract(&tree, &relative_str, &source)
                    }
                    SupportedLanguage::Cpp => languages::cpp::extract(&tree, &relative_str, &source),
                    SupportedLanguage::Ruby => languages::ruby::extract(&tree, &relative_str, &source),
                    SupportedLanguage::C => languages::c_lang::extract(&tree, &relative_str, &source),
                    _ => dispatch_regex(&relative_str, &source, path)?,
                }
            }
            Err(crate::error::ParseError::UnsupportedLanguage { .. }) => {
                dispatch_regex(&relative_str, &source, path)?
            }
            Err(e) => {
                return Err(e.into());
            }
        };

        Ok(result)
    }
}
