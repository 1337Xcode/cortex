//! The checks `cargo xtask verify` runs, in order, each one reported on its own line.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use crate::architecture::Metadata;
use crate::{architecture, invariants, lints};

/// Top-level folders whose sources may carry invariant tags.
const TAGGED_SOURCE_DIRS: &[&str] = &["crates", "ui", "xtask"];

/// File extensions scanned for invariant tags.
const TAGGED_SOURCE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx"];

/// Runs every check, stopping at the first failure.
///
/// # Errors
///
/// Names the failed check; its own output has already been printed.
pub(crate) fn verify(full: bool) -> anyhow::Result<()> {
    let root = workspace_root()?;
    run_cargo(&root, "format", &["fmt", "--all", "--", "--check"])?;
    let metadata = workspace_metadata(&root)?;
    check_architecture(&metadata)?;
    check_lint_inheritance(&metadata)?;
    let crate_names = metadata
        .packages
        .iter()
        .map(|package| package.name.clone())
        .collect();
    check_invariant_tags(&root, &crate_names)?;
    run_cargo(
        &root,
        "clippy",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_cargo(&root, "tests", &["test", "--workspace", "--locked"])?;
    run_cargo(&root, "cargo deny", &["deny", "check"])?;
    if full {
        println!("xtask: budgets: no benchmarks are registered yet");
    }
    Ok(())
}

/// The repository root: the parent of this crate's folder.
fn workspace_root() -> anyhow::Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("finding the workspace root above xtask/")
}

/// The cargo that started xtask, so every step uses the same toolchain.
fn cargo_program() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

/// Runs one cargo command in the workspace root with its output passed through.
fn run_cargo(root: &Path, step: &str, args: &[&str]) -> anyhow::Result<()> {
    println!("xtask: {step}");
    let status = Command::new(cargo_program())
        .args(args)
        .current_dir(root)
        .status()
        .with_context(|| format!("starting cargo for {step}"))?;
    if !status.success() {
        bail!("{step} failed ({status})");
    }
    Ok(())
}

/// The workspace's packages as `cargo metadata --no-deps` reports them.
fn workspace_metadata(root: &Path) -> anyhow::Result<Metadata> {
    let output = Command::new(cargo_program())
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .current_dir(root)
        .output()
        .context("starting cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing cargo metadata output")
}

/// Checks the dependency direction between workspace crates.
fn check_architecture(metadata: &Metadata) -> anyhow::Result<()> {
    println!("xtask: architecture");
    let violations = architecture::check(&metadata.packages);
    if !violations.is_empty() {
        bail!("dependency direction:\n  {}", violations.join("\n  "));
    }
    Ok(())
}

/// Checks that every workspace member's manifest opts into the workspace lints.
fn check_lint_inheritance(metadata: &Metadata) -> anyhow::Result<()> {
    println!("xtask: workspace lints");
    let mut problems = Vec::new();
    for package in &metadata.packages {
        let manifest = fs::read_to_string(&package.manifest_path)
            .with_context(|| format!("reading {}", package.manifest_path.display()))?;
        if !lints::inherits_workspace_lints(&manifest) {
            problems.push(format!(
                "{}: {} does not set `[lints] workspace = true`",
                package.name,
                package.manifest_path.display()
            ));
        }
    }
    if !problems.is_empty() {
        bail!("workspace lints:\n  {}", problems.join("\n  "));
    }
    Ok(())
}

/// Checks that every invariant tag names a known invariant and that each owning crate has one.
fn check_invariant_tags(root: &Path, crate_names: &BTreeSet<String>) -> anyhow::Result<()> {
    println!("xtask: invariant tags");
    let mut tags = Vec::new();
    let mut test_paths = Vec::new();
    for dir in TAGGED_SOURCE_DIRS {
        let path = root.join(dir);
        if path.is_dir() {
            for file in source_files(root, &path)? {
                let text = fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?;
                let relative = relative_path(root, &file);
                tags.extend(invariants::scan(&relative, &text));
                test_paths.extend(invariants::test_module_paths(&relative, &text));
            }
        }
    }
    let problems = invariants::check(&tags, &test_paths, crate_names);
    if !problems.is_empty() {
        bail!("invariant tags:\n  {}", problems.join("\n  "));
    }
    Ok(())
}

/// Every scanned source file under `dir`, sorted.
///
/// Folders `is_skipped_dir` names are left out, and symbolic links are never followed, so a link
/// loop cannot make the walk run forever.
fn source_files(root: &Path, dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries =
            fs::read_dir(&current).with_context(|| format!("listing {}", current.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("listing {}", current.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("reading the type of {}", path.display()))?;
            if file_type.is_dir() {
                if !is_skipped_dir(&relative_path(root, &path)) {
                    pending.push(path);
                }
            } else if file_type.is_file() && has_tagged_extension(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Whether a folder, given relative to the workspace root with `/` separators, is left out of the
/// tag scan: version-control data and installed dependencies anywhere, the UI's build output, a
/// package's own `target` folder, and recorded test data under a `tests` folder.
fn is_skipped_dir(relative: &str) -> bool {
    let parts: Vec<&str> = relative.split('/').collect();
    matches!(
        parts.as_slice(),
        [.., ".git" | "node_modules"]
            | ["ui", "dist"]
            | ["xtask" | "ui", "target"]
            | ["crates", _, "target"]
            | [.., "tests", "fixtures"]
    )
}

/// Whether the file's extension is one that may carry invariant tags.
fn has_tagged_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| TAGGED_SOURCE_EXTENSIONS.contains(&extension))
}

/// `file` relative to `root`, with `/` separators on every platform.
fn relative_path(root: &Path, file: &Path) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{SystemTime, UNIX_EPOCH};

    /// A folder under the system temp folder, removed when the value is dropped.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        /// Creates an empty folder whose name is unique to this process and moment.
        fn new(label: &str) -> anyhow::Result<Self> {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let path =
                std::env::temp_dir().join(format!("xtask-{label}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&path)?;
            Ok(Self(path))
        }

        /// Writes `text` to `relative`, creating its parent folders.
        fn write(&self, relative: &str, text: &str) -> anyhow::Result<()> {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, text)?;
            Ok(())
        }
    }

    impl Drop for ScratchDir {
        /// Removes the folder; a failure only leaves a temp folder behind.
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The scanned files under `scratch`, as relative paths.
    fn scanned(scratch: &ScratchDir) -> anyhow::Result<Vec<String>> {
        Ok(source_files(&scratch.0, &scratch.0.join("crates"))?
            .iter()
            .map(|file| relative_path(&scratch.0, file))
            .collect())
    }

    /// Skip rules depend on where a folder sits, not only on its name.
    #[test]
    fn skip_rules_follow_position() {
        let cases = [
            ("crates/cortex-index/target", true),
            ("crates/cortex-retrieve/src/target", false),
            ("xtask/target", true),
            ("ui/node_modules", true),
            ("ui/src/vendor/node_modules", true),
            ("ui/dist", true),
            ("ui/src/dist", false),
            ("crates/cortex-index/tests/fixtures", true),
            ("crates/cortex-index/src/fixtures", false),
            ("crates/cortex-core/.git", true),
            ("crates/cortex-core/src", false),
        ];
        for (relative, skipped) in cases {
            assert_eq!(is_skipped_dir(relative), skipped, "{relative}");
        }
    }

    /// The walk finds tagged-extension files in sorted order and leaves out skipped folders.
    #[test]
    fn walk_finds_sources_and_skips_data() -> anyhow::Result<()> {
        let scratch = ScratchDir::new("walk")?;
        scratch.write("crates/a/src/lib.rs", "")?;
        scratch.write("crates/a/src/target/mod.rs", "")?;
        scratch.write("crates/a/src/notes.txt", "")?;
        scratch.write("crates/a/target/debug/build.rs", "")?;
        scratch.write("crates/a/tests/fixtures/sample.rs", "")?;
        scratch.write("crates/a/tests/walk.rs", "")?;
        assert_eq!(
            scanned(&scratch)?,
            vec![
                "crates/a/src/lib.rs",
                "crates/a/src/target/mod.rs",
                "crates/a/tests/walk.rs",
            ]
        );
        Ok(())
    }

    /// A symbolic link loop is not followed, so the walk ends and lists the real file once.
    // Creating symbolic links on Windows needs a privilege most accounts lack, so this case runs
    // on Unix only; the code under test is the same on every platform.
    #[cfg(unix)]
    #[test]
    fn walk_does_not_follow_symlinks() -> anyhow::Result<()> {
        let scratch = ScratchDir::new("links")?;
        scratch.write("crates/a/src/lib.rs", "")?;
        std::os::unix::fs::symlink(
            scratch.0.join("crates"),
            scratch.0.join("crates/a/src/loop"),
        )?;
        assert_eq!(scanned(&scratch)?, vec!["crates/a/src/lib.rs"]);
        Ok(())
    }

    /// The real workspace parses, keeps the layer rules and inherits the workspace lints.
    #[test]
    fn real_workspace_passes_the_manifest_checks() -> anyhow::Result<()> {
        let metadata = workspace_metadata(&workspace_root()?)?;
        let names: Vec<&str> = metadata
            .packages
            .iter()
            .map(|package| package.name.as_str())
            .collect();
        for expected in ["cortex-cli", "cortex-tests", "xtask"] {
            assert!(
                names.contains(&expected),
                "{expected} missing from {names:?}"
            );
        }
        check_architecture(&metadata)?;
        check_lint_inheritance(&metadata)?;
        Ok(())
    }
}
