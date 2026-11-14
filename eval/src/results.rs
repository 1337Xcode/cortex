//! A results folder: `runs.json`, each completed run's counts, written as each run ends;
//! `summary.json` and `summary.txt`, what the runs add up to, written once all have. A summary can
//! be rebuilt from the runs, so that a change to how runs add up reaches results already made
//! without running their sessions again. An arm's folder also holds `comparison.json` and
//! `comparison.txt` once it is compared with another arm's.

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::compare;
use crate::report::{self, Incomplete};
use crate::run::TaskRun;

/// The file each completed run's counts go to.
const RUNS_FILE: &str = "runs.json";

/// The file the summary goes to as JSON.
const SUMMARY_FILE: &str = "summary.json";

/// The file the summary goes to as text.
const SUMMARY_TEXT_FILE: &str = "summary.txt";

/// The file a comparison with the held-out arm goes to as JSON, in the treated arm's folder.
const COMPARISON_FILE: &str = "comparison.json";

/// The file the comparison goes to as text.
const COMPARISON_TEXT_FILE: &str = "comparison.txt";

/// What a rebuild keeps from the summary written when the evaluation ran: the runs that could not
/// complete, which `runs.json` does not hold.
#[derive(Debug, Deserialize)]
struct Written {
    /// The runs that could not complete.
    incomplete: Vec<Incomplete>,
}

/// Writes the completed `runs` to `folder`'s `runs.json`.
///
/// # Errors
///
/// The file cannot be written.
pub(crate) fn write_runs(folder: &Path, runs: &[TaskRun]) -> anyhow::Result<()> {
    write_json(&folder.join(RUNS_FILE), &runs)
}

/// Writes what `runs` add up to, with the runs that could not complete, `incomplete`, to
/// `folder`'s `summary.json` and `summary.txt`, and returns the summary's lines.
///
/// # Errors
///
/// A file cannot be written.
pub(crate) fn write_summary(
    folder: &Path,
    runs: &[TaskRun],
    incomplete: &[Incomplete],
) -> anyhow::Result<Vec<String>> {
    let summary = report::summary(runs, incomplete);
    write_json(&folder.join(SUMMARY_FILE), &summary)?;
    let text = report::lines(&summary);
    let path = folder.join(SUMMARY_TEXT_FILE);
    std::fs::write(&path, format!("{}\n", text.join("\n")))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(text)
}

/// Rebuilds `folder`'s summary from its `runs.json`, keeping the runs that could not complete
/// from the `summary.json` written when the evaluation ran, and returns the summary's lines.
///
/// # Errors
///
/// `runs.json` or `summary.json` cannot be read or is not what the runner writes, or the summary
/// cannot be written.
pub(crate) fn rebuild(folder: &Path) -> anyhow::Result<Vec<String>> {
    let runs: Vec<TaskRun> = read_json(&folder.join(RUNS_FILE))?;
    let written: Written = read_json(&folder.join(SUMMARY_FILE))?;
    write_summary(folder, &runs, &written.incomplete)
}

/// Compares the runs in `treated`'s `runs.json` with those in `held_out`'s, paired by task (see
/// [`compare`]), writes the comparison to `treated`'s `comparison.json` and `comparison.txt`, and
/// returns its lines.
///
/// # Errors
///
/// A `runs.json` cannot be read or is not what the runner writes, or the comparison cannot be
/// written.
pub(crate) fn write_comparison(treated: &Path, held_out: &Path) -> anyhow::Result<Vec<String>> {
    let on: Vec<TaskRun> = read_json(&treated.join(RUNS_FILE))?;
    let off: Vec<TaskRun> = read_json(&held_out.join(RUNS_FILE))?;
    let groups = compare::compare(&on, &off);
    write_json(&treated.join(COMPARISON_FILE), &groups)?;
    let text = compare::lines(&groups, compare::arm_names(&off));
    let path = treated.join(COMPARISON_TEXT_FILE);
    std::fs::write(&path, format!("{}\n", text.join("\n")))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(text)
}

/// The JSON file at `path` as a `T`.
///
/// # Errors
///
/// The file cannot be read or is not a `T`.
fn read_json<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Writes `value` to `path` as pretty JSON.
///
/// # Errors
///
/// The file cannot be written.
fn write_json(path: &Path, value: &impl serde::Serialize) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(value).context("writing JSON")?;
    std::fs::write(path, format!("{text}\n")).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Session;
    use crate::run::Given;

    /// A run of task `t` numbered `run` whose measured session used `used` of its one labelled
    /// work order.
    fn run(run: u32, used: u64) -> TaskRun {
        let session = Session {
            agent_version: Some("2.1.318".to_owned()),
            turns: 2,
            ..Session::default()
        };
        TaskRun {
            task: "t".to_owned(),
            kind: "k".to_owned(),
            run,
            holdout_share: 0.0,
            sessions: vec![session],
            is_success: true,
            given: Given {
                delivered: 1,
                labelled: 1,
                used,
            },
            shown: None,
            warnings: None,
            config: Vec::new(),
            facts: None,
            notes: 0,
            effort: None,
        }
    }

    /// The runs read back are the runs written. A rebuild over two runs of t, one of them using its
    /// work order, counts 2 completed runs, 2 labelled and 1 used, though the summary written first
    /// counted only the first run; it keeps from that summary the run that could not complete (t
    /// run 3), and writes the same lines to `summary.txt` as it returns.
    #[test]
    fn a_summary_is_rebuilt_from_its_runs() -> anyhow::Result<()> {
        let folder = tempfile::tempdir()?;
        let runs = [run(1, 1), run(2, 0)];
        let missed = [Incomplete {
            task: "t".to_owned(),
            run: 3,
        }];
        write_runs(folder.path(), &runs)?;
        let read: Vec<TaskRun> = read_json(&folder.path().join(RUNS_FILE))?;
        assert_eq!(read, runs);
        write_summary(folder.path(), &runs[..1], &missed)?;
        let lines = rebuild(folder.path())?;
        assert!(
            lines.contains(
                &"runs: 2 completed, 1 could not complete; sessions that ended in an error: 0"
                    .to_owned()
            )
        );
        let text = std::fs::read_to_string(folder.path().join(SUMMARY_TEXT_FILE))?;
        assert_eq!(text, format!("{}\n", lines.join("\n")));
        let summary: serde_json::Value = read_json(&folder.path().join(SUMMARY_FILE))?;
        assert_eq!(summary["runs"], 2);
        assert_eq!(
            (summary["labelled"].clone(), summary["used"].clone()),
            (2.into(), 1.into())
        );
        assert_eq!(summary["incomplete"][0]["run"], 3);
        Ok(())
    }

    /// Without the summary written when the evaluation ran there is no telling which runs could not
    /// complete, so a rebuild refuses rather than say none did, and writes nothing.
    #[test]
    fn a_rebuild_needs_the_first_summary() -> anyhow::Result<()> {
        let folder = tempfile::tempdir()?;
        write_runs(folder.path(), &[run(1, 1)])?;
        let refusal = rebuild(folder.path()).err().context("a refusal")?;
        assert!(format!("{refusal:#}").contains(SUMMARY_FILE), "{refusal:#}");
        assert!(!folder.path().join(SUMMARY_TEXT_FILE).exists());
        Ok(())
    }
}
