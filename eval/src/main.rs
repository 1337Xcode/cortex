//! `cortex-eval`: Cortex's evaluation suite. `cortex-eval check` reads every task and testbed and
//! says what is wrong with them; `cortex-eval run` runs tasks with a real agent in fresh copies of
//! their testbeds and writes what the runs took to a results folder, as counts only;
//! `cortex-eval report` rebuilds a results folder's summary from its runs; `cortex-eval compare`
//! sets a treated arm's runs beside a held-out arm's, paired by task.
//!
//! The suite's files are this crate's own: `testbeds.toml`, `tasks/<id>.toml`, the checks in
//! `checks/` and the regression diffs in `regressions/`.

mod agent;
mod compare;
mod process;
mod report;
mod results;
mod run;
mod shown;
mod snapshot;
mod task;
mod testbed;
mod texts;
mod warned;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};

use crate::report::Incomplete;
use crate::run::{Settings, TaskRun, remove_run_folder, run_folder, run_task, taken_folders};
use crate::task::Task;
use crate::testbed::Testbed;

/// The suite's own folder.
const SUITE: &str = env!("CARGO_MANIFEST_DIR");

/// The suite's folder of regression diffs, which tasks name by file.
const REGRESSIONS: &str = "regressions";

/// The command line.
#[derive(Debug, Parser)]
#[command(name = "cortex-eval", version)]
struct Cli {
    /// What to do.
    #[command(subcommand)]
    command: Command,
}

/// What `cortex-eval` does.
#[derive(Debug, Subcommand)]
enum Command {
    /// Read every task and testbed and check them.
    Check(CheckArgs),
    /// Run tasks with Claude Code and write what the runs took.
    Run(Box<RunArgs>),
    /// Rebuild a results folder's summary from its runs, keeping the runs that could not complete
    /// from the summary written when the evaluation ran.
    Report {
        /// The results folder, holding `runs.json` and `summary.json`.
        folder: PathBuf,
    },
    /// Compare an arm's runs with another arm's, paired by task: a treated arm's with a held-out
    /// arm's, or one noting the tasks' rules with one leaving them out; and write the comparison to
    /// the first arm's folder.
    Compare {
        /// The first arm's results folder, holding `runs.json`: the treated arm, or the one
        /// noting the tasks' rules.
        treated: PathBuf,
        /// The second arm's results folder, holding `runs.json`: the held-out arm, or the one
        /// leaving the rules out.
        held_out: PathBuf,
    },
}

/// Which arm of the evaluation a run is in.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Arm {
    /// Cortex installed and giving work orders.
    Treated,
    /// Cortex installed and holding every task out.
    HeldOut,
}

/// The arguments of `cortex-eval check`.
#[derive(Debug, clap::Args)]
struct CheckArgs {
    /// A testbed's folder, as `name=folder`, as `run --from` takes it: a task that needs history
    /// is refused when its testbed's folder is not a git repository holding the testbed's commit.
    #[arg(long = "from")]
    from: Vec<String>,
}

/// The arguments of `cortex-eval run`.
#[derive(Debug, clap::Args)]
struct RunArgs {
    /// The arm the runs are in.
    #[arg(long, value_enum)]
    arm: Arm,
    /// A task to run; every task when none is named.
    #[arg(long = "task")]
    tasks: Vec<String>,
    /// How many times each task runs.
    #[arg(long, default_value_t = 1)]
    runs: u32,
    /// The `cortex` binary to install.
    #[arg(long)]
    cortex: PathBuf,
    /// The model Claude Code runs.
    #[arg(long, default_value = "sonnet")]
    model: String,
    /// The effort level Claude Code runs at.
    #[arg(long, default_value = "medium", value_parser = ["low", "medium", "high", "xhigh", "max"])]
    effort: String,
    /// A testbed copied from a folder holding its commit, as `name=folder`, instead of cloned.
    #[arg(long = "from")]
    from: Vec<String>,
    /// The folder each run's copy is made in, outside any repository.
    #[arg(long)]
    base: PathBuf,
    /// The folder the results are written to.
    #[arg(long)]
    out: PathBuf,
    /// Claude Code's configuration folder, where its login is; `~/.claude` when left out.
    #[arg(long)]
    claude_config: Option<PathBuf>,
    /// A Cortex configuration key every run gets, as `key=value` with the value in TOML
    /// (`warnings.max_per_edit=0`); kept with each run's results.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,
    /// Leave out the project rules the tasks carry: no run notes them before its first session.
    #[arg(long)]
    without_facts: bool,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Check(args) => {
            let (testbeds, tasks) = suite(Path::new(SUITE))?;
            check_history_sources(tasks.values(), &testbeds, &parse_from(&args.from)?)?;
            say(&[format!(
                "{} testbeds and {} tasks, each task's testbed listed",
                testbeds.len(),
                tasks.len()
            )]);
            Ok(())
        }
        Command::Run(args) => run(&args),
        Command::Report { folder } => {
            say(&results::rebuild(&folder)?);
            Ok(())
        }
        Command::Compare { treated, held_out } => {
            say(&results::write_comparison(&treated, &held_out)?);
            Ok(())
        }
    }
}

/// The configuration lines `set` asks for, each `key=value` written `key = value`.
///
/// # Errors
///
/// An entry with no `=`, an empty key or value, or a key that is not dotted words.
fn config_lines(set: &[String]) -> anyhow::Result<Vec<String>> {
    set.iter()
        .map(|entry| {
            let (key, value) = entry
                .split_once('=')
                .with_context(|| format!("`{entry}` is not key=value"))?;
            let key = key.trim();
            let is_key = key.split('.').all(|word| {
                !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
            if !is_key || value.trim().is_empty() {
                bail!("`{entry}` is not a dotted key and a value");
            }
            Ok(format!("{key} = {}", value.trim()))
        })
        .collect()
}

/// The suite in `folder`: its testbeds, and its tasks by id, each naming a listed testbed.
///
/// # Errors
///
/// A file that cannot be read or is not what it should be, a task naming no listed testbed, or a
/// task whose regression is not a file in `regressions/`.
fn suite(folder: &Path) -> anyhow::Result<(Vec<Testbed>, BTreeMap<String, Task>)> {
    let beds_file = folder.join("testbeds.toml");
    let text = std::fs::read_to_string(&beds_file)
        .with_context(|| format!("reading {}", beds_file.display()))?;
    let testbeds = testbed::parse(&text)?;
    let mut tasks = BTreeMap::new();
    let tasks_folder = folder.join("tasks");
    for entry in std::fs::read_dir(&tasks_folder)
        .with_context(|| format!("reading {}", tasks_folder.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let task = Task::parse(&text, &path)?;
        task.check_regression(&folder.join(REGRESSIONS))?;
        if !testbeds.iter().any(|bed| bed.name == task.testbed) {
            bail!(
                "the task {} names no listed testbed: {}",
                task.id,
                task.testbed
            );
        }
        tasks.insert(task.id.clone(), task);
    }
    Ok((testbeds, tasks))
}

/// The testbed folders `pairs` give, each written `name=folder`, by testbed name.
///
/// # Errors
///
/// An entry that is not `name=folder`.
fn parse_from(pairs: &[String]) -> anyhow::Result<BTreeMap<&str, &Path>> {
    pairs
        .iter()
        .map(|pair| {
            pair.split_once('=')
                .map(|(name, folder)| (name, Path::new(folder)))
                .with_context(|| format!("--from {pair} is not name=folder"))
        })
        .collect()
}

/// Refuses a task of `tasks` that needs history when `from` gives its testbed a folder that is not
/// a git repository holding the testbed's commit: such a folder has files to copy and no history
/// to keep. A task whose testbed has no folder in `from` is cloned, and is not refused here.
///
/// # Errors
///
/// The first folder that cannot give its task's testbed its history, naming the task.
fn check_history_sources<'a>(
    tasks: impl IntoIterator<Item = &'a Task>,
    testbeds: &[Testbed],
    from: &BTreeMap<&str, &Path>,
) -> anyhow::Result<()> {
    for task in tasks.into_iter().filter(|task| task.history) {
        let folder = testbeds
            .iter()
            .find(|bed| bed.name == task.testbed)
            .and_then(|bed| from.get(bed.name.as_str()).map(|folder| (bed, *folder)));
        if let Some((bed, folder)) = folder {
            bed.check_history_source(folder)
                .with_context(|| format!("the task {} needs history", task.id))?;
        }
    }
    Ok(())
}

/// Runs the tasks `args` names, or every task, writing each run's counts as it ends and the
/// summary once all have.
///
/// # Errors
///
/// The suite cannot be read, a task named is not in it, or a result cannot be written. A run
/// that cannot complete is not an error: it is printed with why, and counted in the summary.
fn run(args: &RunArgs) -> anyhow::Result<()> {
    let (testbeds, tasks) = suite(Path::new(SUITE))?;
    let chosen: Vec<&Task> = if args.tasks.is_empty() {
        tasks.values().collect()
    } else {
        args.tasks
            .iter()
            .map(|id| tasks.get(id).with_context(|| format!("no task {id}")))
            .collect::<anyhow::Result<_>>()?
    };
    let from = parse_from(&args.from)?;
    check_history_sources(chosen.iter().copied(), &testbeds, &from)?;
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .context("finding the home folder")?;
    let settings = Settings {
        cortex: std::fs::canonicalize(&args.cortex).context("finding the cortex binary")?,
        model: args.model.clone(),
        effort: args.effort.clone(),
        claude_config: args
            .claude_config
            .clone()
            .unwrap_or_else(|| home.join(".claude")),
        base: args.base.clone(),
        checks: Path::new(SUITE).join("checks"),
        regressions: Path::new(SUITE).join(REGRESSIONS),
        holdout_share: match args.arm {
            Arm::Treated => 0.0,
            Arm::HeldOut => 1.0,
        },
        config: config_lines(&args.set)?,
        facts: !args.without_facts,
    };
    let ids: Vec<&str> = chosen.iter().map(|task| task.id.as_str()).collect();
    let taken = taken_folders(&args.base, &ids, args.runs);
    if let Some(first) = taken.first() {
        bail!(
            "{} run folders exist already under {}, {} the first: each evaluation needs a fresh --base",
            taken.len(),
            args.base.display(),
            first.display()
        );
    }
    std::fs::create_dir_all(&args.out).with_context(|| format!("making {}", args.out.display()))?;
    let mut runs: Vec<TaskRun> = Vec::new();
    let mut incomplete: Vec<Incomplete> = Vec::new();
    for task in chosen {
        let testbed = testbeds
            .iter()
            .find(|bed| bed.name == task.testbed)
            .with_context(|| format!("no testbed {}", task.testbed))?;
        for number in 1..=args.runs {
            let ran = match run_task(
                (task, testbed),
                from.get(testbed.name.as_str()).copied(),
                &settings,
                number,
            ) {
                Ok(ran) => ran,
                Err(error) => {
                    // The error names local paths: it is printed, and the results keep only which
                    // run it was.
                    say(&[format!(
                        "{} run {number}: could not complete: {error:#}",
                        task.id
                    )]);
                    incomplete.push(Incomplete {
                        task: task.id.clone(),
                        run: number,
                    });
                    continue;
                }
            };
            say(&[format!(
                "{} run {number}: {}, {} work orders delivered, {} labelled used",
                task.id,
                if ran.is_success { "passed" } else { "failed" },
                ran.given.delivered,
                ran.given.used
            )]);
            runs.push(ran);
            results::write_runs(&args.out, &runs)?;
            // A run that could not complete keeps its folder, to be looked into; this one is done.
            let folder = run_folder(&args.base, &task.id, number);
            if let Err(error) = remove_run_folder(&folder) {
                say(&[format!(
                    "{} run {number}: its folder is left: {error:#}",
                    task.id
                )]);
            }
        }
    }
    say(&results::write_summary(&args.out, &runs, &incomplete)?);
    Ok(())
}

/// Prints `lines` to stdout.
#[allow(clippy::print_stdout, reason = "the runner's report is its output")]
fn say(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task as a file writes it, with `extra` after its keys.
    fn task(extra: &str) -> anyhow::Result<Task> {
        let text = format!(
            "id = \"fix-tax\"\ntestbed = \"shop\"\nkind = \"fix\"\nprompt = \"Fix it.\"\n\
             check = [\"python\", \"c.py\"]\ntouches = [\"a.py\"]\ntools = [\"Read\"]\n{extra}"
        );
        Task::parse(&text, Path::new("fix-tax.toml"))
    }

    /// `check` refuses a task that needs history when its testbed's folder is not a git repository,
    /// naming the task and the folder, and does not refuse the same folder for a task that needs
    /// none, a task whose testbed has no folder, or a folder of another testbed.
    #[test]
    fn a_history_task_is_refused_a_folder_without_git() -> anyhow::Result<()> {
        let files = tempfile::tempdir()?;
        std::fs::write(files.path().join("a.py"), "a = 1\n")?;
        let beds = [Testbed {
            name: "shop".to_owned(),
            url: "https://example.invalid/shop".to_owned(),
            commit: "abc".to_owned(),
            description: String::new(),
        }];
        let (needs, needs_none) = (task("history = true\n")?, task("")?);
        let given = BTreeMap::from([("shop", files.path())]);
        let refused = check_history_sources([&needs], &beds, &given)
            .err()
            .map(|error| format!("{error:#}"))
            .unwrap_or_default();
        assert!(
            refused.contains("the task fix-tax needs history"),
            "{refused}"
        );
        assert!(refused.contains("has no .git"), "{refused}");
        assert!(
            refused.contains(&files.path().display().to_string()),
            "{refused}"
        );
        check_history_sources([&needs_none], &beds, &given)?;
        check_history_sources([&needs], &beds, &BTreeMap::new())?;
        check_history_sources([&needs], &beds, &BTreeMap::from([("other", files.path())]))?;
        Ok(())
    }

    /// `--from` entries are read as `name=folder`, and one that is not is refused.
    #[test]
    fn from_entries_are_read_as_name_and_folder() -> anyhow::Result<()> {
        let pairs = ["serena=beds/serena".to_owned()];
        let read = parse_from(&pairs)?;
        assert_eq!(read.get("serena"), Some(&Path::new("beds/serena")));
        assert!(parse_from(&["serena".to_owned()]).is_err());
        Ok(())
    }

    /// `--set` entries become dotted TOML lines; one with no `=`, an empty or broken key or an
    /// empty value is refused, naming the entry.
    #[test]
    fn set_entries_become_configuration_lines() -> anyhow::Result<()> {
        let lines = config_lines(&[
            "warnings.max_per_edit=0".to_owned(),
            "retrieval.gate = \"off\"".to_owned(),
        ])?;
        assert_eq!(
            lines,
            ["warnings.max_per_edit = 0", "retrieval.gate = \"off\""]
        );
        for entry in [
            "no_equals",
            "=1",
            "a..b=1",
            "a b=1",
            "warnings.max_per_edit= ",
        ] {
            let refused = config_lines(&[entry.to_owned()]);
            assert!(refused.is_err(), "{entry}");
        }
        Ok(())
    }
}
