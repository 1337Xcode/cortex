//! `cortex-eval`: Cortex's evaluation suite. `cortex-eval check` reads every task and testbed and
//! says what is wrong with them; `cortex-eval run` runs tasks with a real agent in fresh copies of
//! their testbeds and writes what the runs took to a results folder, as counts only;
//! `cortex-eval report` rebuilds a results folder's summary from its runs.
//!
//! The suite's files are this crate's own: `testbeds.toml`, `tasks/<id>.toml` and the checks in
//! `checks/`.

mod agent;
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
use crate::run::{Settings, TaskRun, run_task, taken_folders};
use crate::task::Task;
use crate::testbed::Testbed;

/// The suite's own folder.
const SUITE: &str = env!("CARGO_MANIFEST_DIR");

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
    Check,
    /// Run tasks with Claude Code and write what the runs took.
    Run(Box<RunArgs>),
    /// Rebuild a results folder's summary from its runs, keeping the runs that could not complete
    /// from the summary written when the evaluation ran.
    Report {
        /// The results folder, holding `runs.json` and `summary.json`.
        folder: PathBuf,
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
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Check => {
            let (testbeds, tasks) = suite(Path::new(SUITE))?;
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
    }
}

/// The suite in `folder`: its testbeds, and its tasks by id, each naming a listed testbed.
///
/// # Errors
///
/// A file that cannot be read or is not what it should be, or a task naming no listed testbed.
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
    let from: BTreeMap<&str, &Path> = args
        .from
        .iter()
        .map(|pair| {
            pair.split_once('=')
                .map(|(name, folder)| (name, Path::new(folder)))
                .with_context(|| format!("--from {pair} is not name=folder"))
        })
        .collect::<anyhow::Result<_>>()?;
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .context("finding the home folder")?;
    let settings = Settings {
        cortex: std::fs::canonicalize(&args.cortex).context("finding the cortex binary")?,
        model: args.model.clone(),
        claude_config: args
            .claude_config
            .clone()
            .unwrap_or_else(|| home.join(".claude")),
        base: args.base.clone(),
        checks: Path::new(SUITE).join("checks"),
        holdout_share: match args.arm {
            Arm::Treated => 0.0,
            Arm::HeldOut => 1.0,
        },
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
