//! The map's budgets in `cargo xtask verify --full`: the browser measurement (`ui/e2e/scale.spec.ts`)
//! writes what it measured on a generated repository of 100,000 files, and this checks it. The first
//! load (from navigation to two drawn frames, in fresh browsers) must be within 2 s every time, and
//! the frame rate over a scripted pan and zoom at least 45 frames per second at its 5th percentile.
//! Like the hooks' latency budgets they hold on macOS and Linux and are printed, not checked, on
//! Windows.

use std::path::Path;

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::budgets::{Platforms, is_checked_here};

/// The slowest first load within budget, in milliseconds: the map's 2 s budget.
const FIRST_LOAD_LIMIT_MS: f64 = 2_000.0;

/// The lowest 5th-percentile frame rate within budget: the map's 45 frames per second.
const FRAME_RATE_FLOOR_FPS: f64 = 45.0;

/// Where the map's budgets hold.
const PLATFORMS: Platforms = Platforms::Reference;

/// The fewest fresh browsers a first-load verdict rests on: the budget holds for every first load,
/// and one fast run says little about the next.
const MIN_FRESH_LOADS: usize = 5;

/// Names in a WebGL renderer string that mark a CPU rasterizer. The budgets are for drawing on a
/// GPU; a machine with none measures the CPU, so its numbers cannot pass or fail them.
const SOFTWARE_RENDERERS: &[&str] = &["swiftshader", "llvmpipe", "softpipe", "basic render"];

/// Refuses a record a budget cannot rest on: too few fresh browsers, or a software renderer.
///
/// # Errors
///
/// Fewer than `MIN_FRESH_LOADS` first loads, or a renderer named in `SOFTWARE_RENDERERS`.
fn check_measurable(record: &ScaleRecord) -> anyhow::Result<()> {
    if record.first_load_ms.len() < MIN_FRESH_LOADS {
        bail!(
            "the measurement holds {} first loads; the budget needs {MIN_FRESH_LOADS} fresh browsers",
            record.first_load_ms.len()
        );
    }
    let renderer = record.renderer.to_lowercase();
    if SOFTWARE_RENDERERS
        .iter()
        .any(|name| renderer.contains(name))
    {
        bail!(
            "the map was drawn by a software renderer ({}); its budgets hold on a GPU",
            record.renderer
        );
    }
    Ok(())
}

/// What the browser measurement wrote.
#[derive(Debug, Deserialize)]
struct ScaleRecord {
    /// The machine, as the measurement describes it.
    platform: String,
    /// The GPU the page drew with, as WebGL names it.
    renderer: String,
    /// The first load of each fresh browser, in milliseconds.
    first_load_ms: Vec<f64>,
    /// The 5th-percentile frame rate of each scripted pan and zoom.
    pan_zoom_p5_fps: Vec<f64>,
}

/// Reads `path`, prints each budget's line and fails when one is breached where it is checked.
///
/// # Errors
///
/// The file cannot be read or does not hold a measurement, or a budget is breached.
pub(crate) fn check(path: &Path) -> anyhow::Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let record: ScaleRecord =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let (lines, breached) = verdicts(&record, is_checked_here(PLATFORMS))
        .with_context(|| format!("reading {}", path.display()))?;
    for line in lines {
        println!("xtask: budget: {line}");
    }
    if !breached.is_empty() {
        bail!("over budget: {}", breached.join("; "));
    }
    Ok(())
}

/// The printed lines for `record` and the budgets it breaches; with `is_checked` false, none is
/// breached and each line says the budget is not checked here.
///
/// # Errors
///
/// The record holds no first load or no frame rate, or, where the budgets are checked, it cannot
/// carry a verdict (`check_measurable`).
fn verdicts(record: &ScaleRecord, is_checked: bool) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    if is_checked {
        check_measurable(record)?;
    }
    let slowest = record
        .first_load_ms
        .iter()
        .copied()
        .max_by(f64::total_cmp)
        .context("the measurement holds no first load")?;
    let lowest = record
        .pan_zoom_p5_fps
        .iter()
        .copied()
        .min_by(f64::total_cmp)
        .context("the measurement holds no frame rate")?;
    let measured = format!("on {}, drawn by {}", record.platform, record.renderer);
    let mut lines = vec![format!("map at 100,000 files, {measured}")];
    let mut breached = Vec::new();
    let load_what = format!(
        "map first load, slowest of {} fresh browsers",
        record.first_load_ms.len()
    );
    let load_verdict = verdict(is_checked, slowest <= FIRST_LOAD_LIMIT_MS, "2000 ms budget");
    lines.push(format!("{load_what}: {slowest:.0} ms, {load_verdict}"));
    if is_checked && slowest > FIRST_LOAD_LIMIT_MS {
        breached.push(load_what);
    }
    let rate_what = format!(
        "map frame rate, 5th percentile, lowest of {} pans and zooms",
        record.pan_zoom_p5_fps.len()
    );
    let rate_verdict = verdict(is_checked, lowest >= FRAME_RATE_FLOOR_FPS, "45 fps budget");
    lines.push(format!("{rate_what}: {lowest:.1} fps, {rate_verdict}"));
    if is_checked && lowest < FRAME_RATE_FLOOR_FPS {
        breached.push(rate_what);
    }
    Ok((lines, breached))
}

/// How a measurement stands against `budget`.
fn verdict(is_checked: bool, is_within: bool, budget: &str) -> String {
    if !is_checked {
        format!("the {budget} holds on macOS and Linux and is not checked here")
    } else if is_within {
        format!("within the {budget}")
    } else {
        format!("OVER the {budget}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record as the measurement writes it, with five first loads and two frame rates.
    fn record(first_load_ms: &str, pan_zoom_p5_fps: &str) -> anyhow::Result<ScaleRecord> {
        Ok(serde_json::from_str(&format!(
            r#"{{"platform":"linux 6.6 x64","renderer":"Mesa Intel Xe","first_load_ms":{first_load_ms},"pan_zoom_p5_fps":{pan_zoom_p5_fps}}}"#
        ))?)
    }

    /// The slowest load and the lowest frame rate are what is checked: 2000 ms and 45.0 fps are
    /// within budget, 2001 ms and 44.9 fps are over it.
    #[test]
    fn the_slowest_load_and_lowest_rate_are_checked() -> anyhow::Result<()> {
        let within = record("[930, 2000, 890, 1200, 999]", "[59.9, 45.0]")?;
        let (lines, breached) = verdicts(&within, true)?;
        assert_eq!(
            lines,
            [
                "map at 100,000 files, on linux 6.6 x64, drawn by Mesa Intel Xe",
                "map first load, slowest of 5 fresh browsers: 2000 ms, within the 2000 ms budget",
                "map frame rate, 5th percentile, lowest of 2 pans and zooms: 45.0 fps, within the 45 fps budget",
            ]
        );
        assert!(breached.is_empty());
        let over = record("[930, 2001, 890, 1000, 1100]", "[44.9, 60.0]")?;
        let (lines, breached) = verdicts(&over, true)?;
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some("map first load, slowest of 5 fresh browsers: 2001 ms, OVER the 2000 ms budget")
        );
        assert_eq!(
            lines.get(2).map(String::as_str),
            Some(
                "map frame rate, 5th percentile, lowest of 2 pans and zooms: 44.9 fps, OVER the 45 fps budget"
            )
        );
        assert_eq!(
            breached,
            [
                "map first load, slowest of 5 fresh browsers",
                "map frame rate, 5th percentile, lowest of 2 pans and zooms"
            ]
        );
        Ok(())
    }

    /// Where the budgets are not checked, a breach is printed and nothing fails.
    #[test]
    fn unchecked_budgets_are_printed_only() -> anyhow::Result<()> {
        let (lines, breached) = verdicts(&record("[4000, 3900]", "[10.0]")?, false)?;
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some(
                "map first load, slowest of 2 fresh browsers: 4000 ms, the 2000 ms budget holds on macOS and Linux and is not checked here"
            )
        );
        assert!(breached.is_empty());
        Ok(())
    }

    /// Where the budgets are checked, four fresh browsers or a software renderer (SwiftShader,
    /// llvmpipe) give no verdict, even with numbers inside the budgets; where they are not
    /// checked, the same records are printed.
    #[test]
    fn a_record_that_cannot_carry_a_verdict_is_refused_where_checked() -> anyhow::Result<()> {
        let four = record("[900, 910, 920, 930]", "[60.0]")?;
        assert!(verdicts(&four, true).is_err());
        assert!(verdicts(&four, false).is_ok());
        for renderer in [
            "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
            "llvmpipe (LLVM 19.1.7, 256 bits)",
        ] {
            let software: ScaleRecord = serde_json::from_str(&format!(
                r#"{{"platform":"linux 6.6 x64","renderer":"{renderer}","first_load_ms":[900,910,920,930,940],"pan_zoom_p5_fps":[60.0]}}"#
            ))?;
            assert!(verdicts(&software, true).is_err(), "{renderer}");
            assert!(verdicts(&software, false).is_ok(), "{renderer}");
        }
        Ok(())
    }

    /// A measurement with no first load or no frame rate proves nothing and is refused, as is a
    /// file missing a field.
    #[test]
    fn an_empty_measurement_is_refused() -> anyhow::Result<()> {
        assert!(verdicts(&record("[]", "[50.0]")?, false).is_err());
        assert!(verdicts(&record("[900]", "[]")?, false).is_err());
        assert!(serde_json::from_str::<ScaleRecord>(r#"{"platform":"x","renderer":"y"}"#).is_err());
        Ok(())
    }
}
