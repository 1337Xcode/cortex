//! The checks `cargo xtask verify` runs, in order, each one reported on its own line.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use crate::{architecture, invariants};

/// Top-level folders whose sources may carry invariant tags.
const TAGGED_SOURCE_DIRS: &[&str] = &["crates", "ui", "xtask"];

/// File extensions scanned for invariant tags.
const TAGGED_SOURCE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx"];

/// Folders skipped while scanning: build output, dependencies and test data.
const SKIPPED_DIRS: &[&str] = &["target", "node_modules", "dist", ".git", "fixtures"];

/// Runs every check, stopping at the first failure.
///
/// # Errors
///
/// Names the failed check; its own output has already been printed.
pub(crate) fn verify(full: bool) -> anyhow::Result<()> {
    let root = workspace_root()?;
    run_cargo(&root, "format", &["fmt", "--all", "--", "--check"])?;
    let crate_names = check_architecture(&root)?;
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

/// Checks the dependency direction between workspace crates and returns their names.
fn check_architecture(root: &Path) -> anyhow::Result<BTreeSet<String>> {
    println!("xtask: architecture");
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
    let metadata: architecture::Metadata =
        serde_json::from_slice(&output.stdout).context("parsing cargo metadata output")?;
    let violations = architecture::check(&metadata.packages);
    if !violations.is_empty() {
        bail!("dependency direction:\n  {}", violations.join("\n  "));
    }
    Ok(metadata
        .packages
        .into_iter()
        .map(|package| package.name)
        .collect())
}

/// Checks that every invariant tag names a known invariant and that each owning crate has one.
fn check_invariant_tags(root: &Path, crate_names: &BTreeSet<String>) -> anyhow::Result<()> {
    println!("xtask: invariant tags");
    let mut tags = Vec::new();
    for dir in TAGGED_SOURCE_DIRS {
        let path = root.join(dir);
        if path.is_dir() {
            for file in source_files(&path)? {
                let text = fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?;
                tags.extend(invariants::scan(&relative_path(root, &file), &text));
            }
        }
    }
    let problems = invariants::check(&tags, crate_names);
    if !problems.is_empty() {
        bail!("invariant tags:\n  {}", problems.join("\n  "));
    }
    Ok(())
}

/// Every scanned source file under `dir`, sorted, skipping build output and test data.
fn source_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries =
            fs::read_dir(&current).with_context(|| format!("listing {}", current.display()))?;
        for entry in entries {
            let path = entry
                .with_context(|| format!("listing {}", current.display()))?
                .path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if path.is_dir() {
                if !SKIPPED_DIRS.contains(&name) {
                    pending.push(path);
                }
            } else if has_tagged_extension(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
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
