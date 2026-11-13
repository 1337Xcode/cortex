//! The map's build and checks, run through npm in `ui/`.
//!
//! `cargo xtask ui` installs exactly the packages `ui/package-lock.json` lists, running none of
//! their install scripts, and builds `ui/dist`, which the daemon's build embeds: run it before a
//! release build, or the binary has no map.
//! `cargo xtask verify` runs the map's type check, linters and unit tests, builds it and checks the
//! bundle's size when `ui/node_modules` is there, and says plainly that it skipped them when it is
//! not. `cargo xtask verify --full` adds the browser checks, which measure the map at 100,000 files
//! (see `map_budgets`), and the map's acceptance flow against a real daemon (`ui/e2e/real/`).

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

/// The most the built map may weigh: every file of `ui/dist` gzipped at level 9, summed. The map's
/// bundle budget is 1.5 MB.
const BUNDLE_LIMIT_BYTES: u64 = 1_500_000;

/// The environment variable that tells the browser measurement where to write its numbers.
const SCALE_FILE_VARIABLE: &str = "CORTEX_MAP_SCALE_FILE";

/// The environment variable that names the `cortex` binary the daemon checks run.
const BINARY_VARIABLE: &str = "CORTEX_BIN";

/// npm's launcher: a command script on Windows, which `Command` runs with its arguments escaped.
const NPM: &str = if cfg!(windows) { "npm.cmd" } else { "npm" };

/// Writes each file under the folder given as the first argument with its size gzipped at level 9,
/// as one JSON list of `[path, bytes]` pairs. Run by Node, which the map's checks need anyway.
const GZIP_SIZES_JS: &str = r"
const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');
const root = process.argv[1];
const sizes = [];
const walk = (dir) => {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full);
    else sizes.push([path.relative(root, full), zlib.gzipSync(fs.readFileSync(full), { level: 9 }).length]);
  }
};
walk(root);
process.stdout.write(JSON.stringify(sizes));
";

/// `cargo xtask ui`: installs the map's packages and builds `ui/dist`.
///
/// # Errors
///
/// npm cannot be started, or the install or the build fails.
pub(crate) fn install_and_build(root: &Path) -> anyhow::Result<()> {
    // No package's install script runs: the one the lockfile has (fsevents, optional and macOS
    // only) is not needed to build or check the map.
    run_npm(
        root,
        "ui: install",
        &["--prefix", "ui", "ci", "--ignore-scripts"],
        None,
    )?;
    run_npm(root, "ui: build", &["--prefix", "ui", "run", "build"], None)
}

/// Whether the map's packages are installed, so its checks can run.
pub(crate) fn is_installed(root: &Path) -> bool {
    root.join("ui").join("node_modules").is_dir()
}

/// The map's checks in `cargo xtask verify`: type check, linters and unit tests, the build, and
/// the bundle's size. Skipped, with a line that says so, when the packages are not installed.
///
/// # Errors
///
/// A check fails, the build fails, or the bundle is over its limit.
pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    if !is_installed(root) {
        println!(
            "xtask: ui: SKIPPED the map's checks, its build and its bundle size: ui/node_modules \
             is missing (`cargo xtask ui` installs it)"
        );
        return Ok(());
    }
    run_npm(
        root,
        "ui: checks",
        &["--prefix", "ui", "run", "check"],
        None,
    )?;
    run_npm(root, "ui: build", &["--prefix", "ui", "run", "build"], None)?;
    check_bundle(root)
}

/// The browser checks, with the measurement at 100,000 files written to `scale_file`.
///
/// # Errors
///
/// The packages are not installed (the budgets cannot be measured without them), or a browser
/// check fails.
pub(crate) fn browser_checks(root: &Path, scale_file: &Path) -> anyhow::Result<()> {
    if !is_installed(root) {
        bail!("the map's browser checks need ui/node_modules: run `cargo xtask ui` first");
    }
    run_npm(
        root,
        "ui: browser checks",
        &["--prefix", "ui", "run", "e2e"],
        Some((SCALE_FILE_VARIABLE, scale_file)),
    )
}

/// The map's acceptance flow against a real daemon, the `cortex` at `binary`: a scripted
/// repository and agent session, then each of the map's questions answered from a browser.
///
/// # Errors
///
/// The packages are not installed, or a check fails.
pub(crate) fn daemon_checks(root: &Path, binary: &Path) -> anyhow::Result<()> {
    if !is_installed(root) {
        bail!("the map's daemon checks need ui/node_modules: run `cargo xtask ui` first");
    }
    run_npm(
        root,
        "ui: checks against a real daemon",
        &["--prefix", "ui", "run", "e2e:real"],
        Some((BINARY_VARIABLE, binary)),
    )
}

/// Runs npm with `args` in `root`, its output passed through, with one environment variable set
/// when `env` names one.
fn run_npm(
    root: &Path,
    step: &str,
    args: &[&str],
    env: Option<(&str, &Path)>,
) -> anyhow::Result<()> {
    println!("xtask: {step}");
    let mut command = Command::new(NPM);
    command.args(args).current_dir(root);
    if let Some((name, value)) = env {
        command.env(name, value);
    }
    let status = command
        .status()
        .with_context(|| format!("starting npm for {step} (Node 24 or later is needed)"))?;
    if !status.success() {
        bail!("{step} failed ({status})");
    }
    Ok(())
}

/// Checks the built map's gzipped size against its limit and prints it.
fn check_bundle(root: &Path) -> anyhow::Result<()> {
    let dist = root.join("ui").join("dist");
    let output = Command::new("node")
        .args(["-e", GZIP_SIZES_JS])
        .arg(&dist)
        .output()
        .context("starting Node to measure the map's bundle")?;
    if !output.status.success() {
        bail!(
            "measuring {} failed: {}",
            dist.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let sizes: Vec<(String, u64)> =
        serde_json::from_slice(&output.stdout).context("reading the bundle's sizes")?;
    let (line, is_within) = bundle_verdict(&sizes);
    println!("xtask: ui: {line}");
    if !is_within {
        bail!("the map's bundle is over its limit");
    }
    Ok(())
}

/// The line that reports the bundle's gzipped `sizes`, and whether they fit the limit.
fn bundle_verdict(sizes: &[(String, u64)]) -> (String, bool) {
    let total: u64 = sizes.iter().map(|(_, bytes)| bytes).sum();
    let is_within = total <= BUNDLE_LIMIT_BYTES;
    let verdict = if is_within { "within" } else { "OVER" };
    let line = format!(
        "bundle: {total} bytes gzipped at level 9 over {} files, {verdict} the {BUNDLE_LIMIT_BYTES}-byte limit",
        sizes.len()
    );
    (line, is_within)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sizes from literal file lists: exactly 1,500,000 bytes fits, one byte more does not, and
    /// the line states the total, the file count and the verdict.
    #[test]
    fn the_bundle_fits_up_to_its_limit() {
        let at_limit = [
            ("index.html".to_owned(), 400),
            ("assets/index.js".to_owned(), 1_499_000),
            ("assets/index.css".to_owned(), 600),
        ];
        assert_eq!(
            bundle_verdict(&at_limit),
            (
                "bundle: 1500000 bytes gzipped at level 9 over 3 files, within the 1500000-byte limit"
                    .to_owned(),
                true
            )
        );
        let over = [
            ("index.html".to_owned(), 401),
            ("assets/index.js".to_owned(), 1_499_600),
        ];
        assert_eq!(
            bundle_verdict(&over),
            (
                "bundle: 1500001 bytes gzipped at level 9 over 2 files, OVER the 1500000-byte limit"
                    .to_owned(),
                false
            )
        );
    }

    /// An empty build weighs nothing and fits; the check that it exists is the build's own.
    #[test]
    fn an_empty_list_fits() {
        assert_eq!(
            bundle_verdict(&[]),
            (
                "bundle: 0 bytes gzipped at level 9 over 0 files, within the 1500000-byte limit"
                    .to_owned(),
                true
            )
        );
    }
}
