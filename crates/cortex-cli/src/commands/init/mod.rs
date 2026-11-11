//! `cortex init`: sets Cortex up in a repository, the top of the git repository holding the folder
//! it starts from (see [`root`]), for the agents `--agents` names or else those found on this
//! machine, once a Claude Code among them is a version Cortex's hooks work with. It asks its
//! questions once (see [`consent`]), registers Cortex's hooks with each agent (see [`hooks`]): for
//! Claude Code in the repository's `.claude/settings.local.json` (`--scope project`, the default, a
//! file Claude Code does not commit) or in the user's `~/.claude/settings.json` (`--scope user`),
//! writes the files of Cortex's own an agent loads (a plugin, hook scripts; see [`own`]) and
//! Cortex's MCP server into an agent's JSON settings (see [`mcp`]), says what the user is to do
//! themselves (see [`agent_file`]), keeps the answers, and has git leave out what Cortex writes
//! (see [`ignores`]). Whatever the
//! scope, and with no agent found, the repository is set up.
//!
//! Every file init may change is found, read and checked before anything is asked, and nothing is
//! written until every question is answered. Then the `.gitignore` is written first, so that a run
//! stopped partway never leaves Cortex's files for git to commit, and each file is backed up first,
//! with a manifest, in a folder of the run's own (the repository's files in its
//! `.cortex/backups/`, the manifest also in the user's `~/.cortex/backups/`, outside the
//! repository; the user's files in `~/.cortex/backups/`) and written atomically (see
//! [`crate::backup`]). Nothing of a repository's is written outside it (see [`place`]). Running
//! init again changes nothing.
//!
//! Then it finds the repository's daemon or starts one, waits for its first index (see
//! [`daemon`]), and runs `cortex doctor`'s checks, whose failures it reports without failing: what
//! init writes is written by then.
//!
//! `--global` sets up no repository: every agent found gets Cortex's hooks in the user's own
//! settings, the questions are kept in the user's `~/.cortex/config.toml`, and Cortex then sets up
//! on its own each repository an agent works in (see [`global`] and [`adopt`]).

mod adopt;
mod agent_file;
mod consent;
mod daemon;
mod git;
mod global;
mod hooks;
mod ignores;
mod mcp;
mod own;
mod place;
mod root;

use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, bail};
use cortex_core::{
    LayerKind, MAX_SETTINGS_BYTES, UserFolder, create_private_folder, read_text_file, unix_millis,
};
use cortex_hook::Versions;

use crate::agents::{self, Setup};
use crate::backup::{Backup, Kind};
use crate::commands::doctor::{Finding, checked, claude_code_version};
use crate::output;

pub(crate) use self::adopt::{Adopted, adopt};
use self::agent_file::AgentFile;
use self::consent::{Answers, ConfigFile, Kept};
pub(crate) use self::global::switch_off;
use self::ignores::Ignored;
pub(crate) use self::ignores::without_lines;
pub(crate) use self::place::own_folder;
use self::root::Root;

/// The arguments of `cortex init`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The folder to start from; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
    /// Where Claude Code reads the hooks from.
    #[arg(long, value_enum, default_value_t = Scope::Project)]
    scope: Scope,
    /// Take the default answer to each question not answered yet, instead of asking.
    #[arg(long)]
    yes: bool,
    /// The agents to set up, by their names on Cortex's command line (`claude,codex`); every agent
    /// found on this machine when left out.
    #[arg(long, value_delimiter = ',')]
    agents: Vec<String>,
    /// Set up no repository: write the agents' hooks to the user's own settings, and have Cortex
    /// set up each repository an agent works in on its own.
    #[arg(long, conflicts_with_all = ["root", "scope"])]
    global: bool,
}

/// Where Claude Code reads the hooks Cortex writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Scope {
    /// This repository only: `.claude/settings.local.json`.
    Project,
    /// Every project of the user: `~/.claude/settings.json`.
    User,
}

/// Whose a file init changes is, which decides where it is backed up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// The repository's, in its folder `folder` (made when missing) or at its root.
    Repository {
        /// The repository's folder the file is in, such as `.claude`, or `None` for its root.
        folder: Option<&'static str>,
    },
    /// The user's.
    User,
}

/// A file init may change, as it was found: where it is, and what it held.
struct Found {
    /// The file, links followed.
    file: PathBuf,
    /// Its path, for messages.
    origin: String,
    /// What it held, or `None` when there was none.
    text: Option<String>,
}

impl Found {
    /// The file at `file`, read once it is known not to be read-only.
    ///
    /// # Errors
    ///
    /// The file is read-only, or cannot be read as a small text file.
    fn read(file: PathBuf) -> anyhow::Result<Self> {
        place::writable(&file)?;
        Self::read_only(file)
    }

    /// The file at `file`, which init reads and never writes.
    ///
    /// # Errors
    ///
    /// The file cannot be read as a small text file.
    fn read_only(file: PathBuf) -> anyhow::Result<Self> {
        let text = read_text_file(&file, MAX_SETTINGS_BYTES)?;
        Ok(Self {
            origin: file.display().to_string(),
            file,
            text,
        })
    }

    /// The file as a configuration, `kind` being whose it is.
    fn config(&self, kind: LayerKind) -> ConfigFile<'_> {
        ConfigFile {
            kind,
            text: self.text.as_deref(),
            origin: &self.origin,
        }
    }

    /// The change that writes `after` to the file, a configuration of Cortex's that is `owner`'s,
    /// saying `said` and the path.
    fn changed(self, after: String, owner: Owner, said: &str) -> Change {
        Change {
            said: format!("{said} {}", self.origin),
            file: self.file,
            kind: Kind::CortexConfig,
            before: self.text,
            after,
            owner,
            is_executable: false,
        }
    }
}

/// Every file init may change, found, read and checked before anything is asked.
struct Files {
    /// The user's own configuration, `~/.cortex/config.toml`, read for an answer kept there and
    /// never written.
    users_config: Found,
    /// The repository's configuration, `.cortex/config.toml`.
    repository_config: Found,
    /// In a git repository, what git ignores and the `.gitignore` lines are added to.
    ignores: Option<(Ignored, Found)>,
    /// The files written for the agents.
    settings: Vec<AgentFile>,
    /// The agents set up: what init writes for each planned; in order.
    set_up: Vec<&'static Setup>,
    /// What init says of each agent it found on this machine whose hooks cannot be planned: it
    /// is left out, and the others are set up all the same.
    not_set_up: Vec<String>,
    /// What the user is to do for each agent set up, each said with the agent's name once
    /// everything is written.
    told: Vec<String>,
}

impl Files {
    /// The files init may change for `root` at `scope`, with the hooks of each of `setups`, which
    /// the command line named when `are_named`. A found agent whose hooks cannot be planned (its
    /// settings are not the JSON it reads, or this program's path cannot be written for its
    /// shell) is left out and said; a named one stops init.
    ///
    /// # Errors
    ///
    /// A file is read-only, cannot be read, would be written outside the repository or through a
    /// link git does not read, or git cannot say what it ignores; or a named agent's hooks cannot
    /// be planned.
    fn found(
        scope: Scope,
        root: &Root,
        setups: &[&'static Setup],
        are_named: bool,
    ) -> anyhow::Result<Self> {
        let user = UserFolder::of_current_user().context("finding the home folder")?;
        let repository_config = place::file_in(&root.folder, Some(".cortex"), "config.toml")?;
        let ignores = match root.without_git {
            Some(_) => None,
            None => {
                let ignored = ignores::read(root.worktree(), &root.git_bound)?;
                Some((ignored, Found::read(ignore_file(root)?)?))
            }
        };
        let Planned {
            settings,
            set_up,
            not_set_up,
            told,
        } = planned_hooks(scope, Some(root.worktree()), setups, are_named)?;
        Ok(Self {
            users_config: Found::read_only(user.config_path())?,
            repository_config: Found::read(repository_config)?,
            ignores,
            settings,
            set_up,
            not_set_up,
            told,
        })
    }

    /// The answers the files keep. Outside git nothing is committed, so nothing is shared.
    ///
    /// # Errors
    ///
    /// A configuration is not valid TOML or gives a key an unusable value.
    fn kept(&self) -> anyhow::Result<Kept> {
        let sharing = self
            .ignores
            .as_ref()
            .map_or(Some(false), |(ignored, _)| ignored.sharing());
        consent::kept(
            self.users_config.config(LayerKind::User),
            self.repository_config.config(LayerKind::Repository),
            sharing,
        )
    }

    /// The changes the `answers`, of which `kept` were kept, call for, the `.gitignore`'s first;
    /// the settings' current hooks said on stdout when they need none.
    ///
    /// # Errors
    ///
    /// A configuration with its answer added does not load, or stdout cannot be written.
    fn changes(self, kept: Kept, answers: Answers) -> anyhow::Result<Vec<Change>> {
        let mut changes = Vec::new();
        if let Some((ignored, gitignore)) = self.ignores {
            let made: Vec<&'static str> = self
                .settings
                .iter()
                .filter_map(AgentFile::made_in_repository)
                .collect();
            let lines = ignores::needed(&ignored, answers.sharing, &made);
            changes.extend(ignore_change(gitignore, &lines));
        }
        changes.extend(settings_changes(self.settings)?);
        let repository = consent::with_answers(
            self.repository_config.config(LayerKind::Repository),
            kept,
            answers,
        )?;
        if let Some(after) = repository {
            let owner = Owner::Repository {
                folder: Some(".cortex"),
            };
            changes.push(
                self.repository_config
                    .changed(after, owner, "answer kept in"),
            );
        }
        Ok(changes)
    }
}

/// A file init changes.
struct Change {
    /// The file, links followed.
    file: PathBuf,
    /// What the file is.
    kind: Kind,
    /// What it held when it was read, or `None` when there was none.
    before: Option<String>,
    /// What it gets.
    after: String,
    /// Whose it is.
    owner: Owner,
    /// What init says once it is written.
    said: String,
    /// Whether the agent runs it as a program, so that it is made executable where files carry
    /// that bit.
    is_executable: bool,
}

/// Sets Cortex up in the repository `args` start from, as the module says, and says what it did.
///
/// # Errors
///
/// A folder that cannot be found or would be the home folder, a Claude Code too old, a question
/// left unanswered, a file that cannot be read, is read-only or would be written outside the
/// repository, or a backup or a write that fails.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    if args.global {
        return global::run(args.yes, &args.agents);
    }
    let root = found_root(args)?;
    let (setups, are_named) = selected(&args.agents, &root.folder)?;
    if setups.is_empty() {
        output::line(&format!(
            "note: no agent Cortex sets up was found on this machine, so no hooks are written; \
             once one is installed, run `cortex init` again, or name one with `--agents` ({})",
            known_agents()
        ))?;
    }
    let has_claude = setups.iter().any(|setup| setup.agent.name() == "claude");
    if has_claude {
        let adapter = cortex_hook::adapter("claude").context("finding the Claude Code adapter")?;
        let versions = adapter
            .versions()
            .context("the Claude Code adapter names no versions")?;
        check_claude_code(&root.folder, versions)?;
    }
    let files = Files::found(args.scope, &root, &setups, are_named)?;
    for line in &files.not_set_up {
        output::line(line)?;
    }
    let set_up = files.set_up.clone();
    let told = files.told.clone();
    let kept = files.kept()?;
    let stdin = io::stdin();
    let mut terminal = stdin.is_terminal().then(|| stdin.lock());
    let answers = consent::answers(kept, args.yes, terminal.as_mut())?;
    let changes = files.changes(kept, answers)?;
    apply((&root.folder, root.linked.as_deref()), changes)?;
    daemon::started_and_indexed(&root.folder)?;
    say_told(&told)?;
    let is_claude_set_up = set_up.iter().any(|setup| setup.agent.name() == "claude");
    let failed = if is_claude_set_up {
        checked(root.folder)?
    } else {
        0
    };
    if failed > 0 {
        output::line(&format!(
            "init is done; {failed} of doctor's checks failed, each with its fix above"
        ))?;
    }
    Ok(())
}

/// The agents init sets up: those `names` name, or, when it names none, every agent found on this
/// machine (outside the repository at `root`), Claude Code as any other: init never writes hooks
/// for an agent that is not there. Named agents come each once, in the order first named, and
/// with `true`: the user asked for them.
///
/// # Errors
///
/// A name that is no agent Cortex sets up, or no home folder.
fn selected(names: &[String], root: &Path) -> anyhow::Result<(Vec<&'static Setup>, bool)> {
    if !names.is_empty() {
        let mut named: Vec<&'static Setup> = Vec::new();
        for name in names {
            let setup = agents::setup(name.trim()).with_context(|| {
                format!(
                    "`{name}` is not an agent Cortex sets up; it sets up {}",
                    known_agents()
                )
            })?;
            // A second plan of the same file would find it changed by the first.
            if !named.iter().any(|known| std::ptr::eq(*known, setup)) {
                named.push(setup);
            }
        }
        return Ok((named, true));
    }
    let home = std::env::home_dir().context("finding the home folder")?;
    let found = agents::SETUPS
        .iter()
        .copied()
        .filter(|setup| agents::is_found(setup, &home, root))
        .collect();
    Ok((found, false))
}

/// The names of every agent init sets up, as `--agents` takes them, in its order.
fn known_agents() -> String {
    agents::SETUPS
        .iter()
        .map(|setup| setup.agent.name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The repository init sets up from where `args` start it, said on stdout with why it is not
/// git's top when it is not.
///
/// # Errors
///
/// The folder or the home folder cannot be found, the folder would be the home folder or hold it,
/// or stdout cannot be written.
fn found_root(args: &Args) -> anyhow::Result<Root> {
    let start = match &args.root {
        Some(root) => root.clone(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    let home = std::env::home_dir().context("finding the home folder")?;
    let root = root::find(&start, &home)?;
    output::line(&format!("repository: {}", root.folder.display()))?;
    if let Some(linked) = &root.linked {
        output::line(&format!(
            "note: {} is a linked working tree of this repository; its agents share the \
             repository's daemon and what it learns, and its ignore lines go to the exclude file \
             git keeps for every working tree of the repository, never to a branch's .gitignore",
            linked.display()
        ))?;
    }
    if let Some(why) = &root.without_git {
        output::line(&format!(
            "note: {why}; Cortex works here without git history"
        ))?;
    }
    Ok(root)
}

/// What [`planned_hooks`] found: the files written for the agents, the agents set up, what init
/// says of each found agent left out, and what the user is to do.
struct Planned {
    /// The files written for the agents.
    settings: Vec<AgentFile>,
    /// The agents set up: what init writes for each planned; in order.
    set_up: Vec<&'static Setup>,
    /// What init says of each agent found on this machine whose hooks cannot be planned.
    not_set_up: Vec<String>,
    /// What the user is to do for each agent set up, each with its name.
    told: Vec<String>,
}

/// Says `told`, what the user is to do, once everything is written, each of several lines (the
/// lines to add to a file) a line at a time.
///
/// # Errors
///
/// Stdout cannot be written.
fn say_told(told: &[String]) -> anyhow::Result<()> {
    for text in told {
        output::lines(text)?;
    }
    Ok(())
}

/// What init writes for each of `setups` planned at `scope` for the working tree `worktree` (none
/// for the user's own files alone, as `--global` writes), which the command line named when
/// `are_named`. A found agent whose files cannot be planned is left out and said; a named one stops
/// init.
///
/// # Errors
///
/// A named agent's files cannot be planned.
fn planned_hooks(
    scope: Scope,
    worktree: Option<&Path>,
    setups: &[&'static Setup],
    are_named: bool,
) -> anyhow::Result<Planned> {
    let mut planned = Planned {
        settings: Vec::new(),
        set_up: Vec::new(),
        not_set_up: Vec::new(),
        told: Vec::new(),
    };
    for &setup in setups {
        match agent_file::planned(scope, worktree, setup) {
            Ok(plan) => {
                planned.settings.extend(plan.files);
                // What to add comes before what to turn on once it is there.
                let title = setup.agent.title();
                let steps = plan.told.iter().map(String::as_str);
                planned.told.extend(
                    steps
                        .chain(setup.told.iter().copied())
                        .map(|told| format!("{title}: to do yourself: {told}")),
                );
                planned.set_up.push(setup);
            }
            Err(error) if !are_named => planned.not_set_up.push(format!(
                "note: {} was found but is not set up: {error:#}; `cortex init --agents {}` sets \
                 it up once that is fixed",
                setup.agent.title(),
                setup.agent.name()
            )),
            Err(error) => return Err(error),
        }
    }
    Ok(planned)
}

/// The changes that write each of `settings`, those that hold what init writes already said on
/// stdout instead.
///
/// # Errors
///
/// Stdout cannot be written.
fn settings_changes(settings: Vec<AgentFile>) -> anyhow::Result<Vec<Change>> {
    let mut changes = Vec::new();
    for settings in settings {
        if settings.is_current() {
            output::line(&settings.current())?;
        } else {
            changes.push(settings.into_change());
        }
    }
    Ok(changes)
}

/// Checks that Claude Code, asked its version in `folder`, is one the `versions` Cortex knows
/// allow, and says what it found.
///
/// # Errors
///
/// Claude Code is older than the oldest of `versions`, or stdout cannot be written.
fn check_claude_code(folder: &Path, versions: Versions) -> anyhow::Result<()> {
    match claude_code_version(folder, versions) {
        Some(Finding::Fail { what, fix }) => {
            bail!("{what}; {fix}, then run `cortex init` again")
        }
        Some(Finding::Pass(what)) => output::line(&what)?,
        None => {}
    }
    Ok(())
}

/// The file init adds its ignore lines to for `root`: the `.gitignore` at its top, or, in a linked
/// working tree, whose `.gitignore` belongs to its branch, or for a repository Cortex sets up on
/// its own, whose `.gitignore` belongs to its team, the exclude file git names for it.
///
/// # Errors
///
/// The file is a link, or cannot be found; git cannot name the exclude file.
fn ignore_file(root: &Root) -> anyhow::Result<PathBuf> {
    match (&root.linked, root.is_adopted) {
        (None, false) => place::unlinked_file_in(&root.folder, ".gitignore"),
        (linked, _) => {
            let top = linked.as_deref().unwrap_or(&root.folder);
            let file = ignores::exclude_file(top, &root.git_bound)?;
            let (folder, name) = file
                .parent()
                .zip(file.file_name().and_then(|name| name.to_str()))
                .with_context(|| format!("{} names no file", file.display()))?;
            place::unlinked_file_in(folder, name)
        }
    }
}

/// The change that adds `lines` to the `.gitignore` `found`, or none when there are none to add.
fn ignore_change(found: Found, lines: &[&str]) -> Option<Change> {
    if lines.is_empty() {
        return None;
    }
    Some(Change {
        said: format!(
            "lines added to {}: {}",
            found.file.display(),
            lines.join(", ")
        ),
        after: ignores::with_lines(found.text.as_deref(), lines),
        file: found.file,
        kind: Kind::Gitignore {
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
        },
        before: found.text,
        owner: Owner::Repository { folder: None },
        is_executable: false,
    })
}

/// Writes each of `changes` for the repository whose root is `root`, started in the linked
/// working tree `linked` when it was, each backed up first: the repository's files in a folder of
/// this run's own in its `.cortex/backups/`, with the manifest also in the user's
/// `~/.cortex/backups/`, and the user's files there, each folder made when a first file for it
/// changes, an agent's folder of the repository's in the working tree init started in. For
/// `--global`, which changes the user's files alone, `root` is the home folder, which the
/// manifest names. Then says what it wrote and where the backups are, so that a closed stdout
/// never stops it partway.
///
/// # Errors
///
/// A folder or a backup cannot be made, a file changed since it was read, a write fails, or stdout
/// cannot be written.
fn apply((root, linked): (&Path, Option<&Path>), changes: Vec<Change>) -> anyhow::Result<()> {
    let worktree = linked.unwrap_or(root);
    let now_ms = unix_millis(SystemTime::now());
    let mut repository: Option<Backup> = None;
    let mut user: Option<Backup> = None;
    let mut said = Vec::new();
    for change in changes {
        let backup = match change.owner {
            Owner::Repository { folder } => {
                let backup = made(&mut repository, || {
                    let backups =
                        place::own_folder(&place::own_folder(root, ".cortex")?, "backups")?;
                    let backup =
                        Backup::new(&backups, now_ms, root)?.with_users_copy(&user_backups()?);
                    match linked {
                        Some(linked) => backup.with_worktree(linked),
                        None => Ok(backup),
                    }
                })?;
                if let Some(folder) = folder {
                    // The folder goes in the working tree its file is in: an agent's hooks in the
                    // linked working tree init started in, Cortex's configuration in the
                    // repository's main tree. A linked working tree never gets a `.cortex`, which
                    // would make it a repository of its own.
                    let tree = if change.file.starts_with(worktree) {
                        worktree
                    } else {
                        root
                    };
                    if change.kind != Kind::CortexConfig {
                        recorded_missing(backup, &place::joined(tree, folder))?;
                    }
                    place::own_folders(tree, folder)?;
                }
                backup
            }
            Owner::User => {
                // The user's backups are made first, so that `~/.cortex` is the user's alone
                // before the user's configuration goes into it.
                let backup = made(&mut user, || Backup::new(&user_backups()?, now_ms, root))?;
                if let Some(parent) = change.file.parent() {
                    recorded_missing(backup, parent)?;
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                backup
            }
        };
        let before = change.before.as_deref().map(str::as_bytes);
        backup.replace(&change.file, change.kind, before, change.after.as_bytes())?;
        // A file written over keeps its permissions: on Unix, Cline's switch for a hook is its
        // executable bit, which the user may have turned off.
        if change.is_executable && change.before.is_none() {
            made_executable(&change.file)?;
        }
        said.push(change.said);
    }
    for line in said {
        output::line(&line)?;
    }
    for backup in [repository, user].iter().flatten() {
        output::line(&format!(
            "backup and manifest in {}",
            backup.folder().display()
        ))?;
    }
    Ok(())
}

/// Makes the file at `path`, which init just wrote, executable by the user alone, as an agent that
/// runs it as a program needs.
///
/// # Errors
///
/// The file's permissions cannot be set.
#[cfg(unix)]
fn made_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("making {} executable", path.display()))
}

/// Nothing: Windows runs a script by its name, and its files carry no such bit.
///
/// # Errors
///
/// None.
#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the Unix version of the same function can fail"
)]
fn made_executable(_: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// Records in `backup` each folder on the way to `folder`, `folder` included, that does not exist
/// yet, the outermost first, before init makes them: uninstall removes exactly those again, once
/// nothing is left in them, and never a folder that was there before.
///
/// # Errors
///
/// The manifest cannot be written.
fn recorded_missing(backup: &mut Backup, folder: &Path) -> anyhow::Result<()> {
    let mut missing: Vec<&Path> = folder
        .ancestors()
        .take_while(|folder| fs::symlink_metadata(folder).is_err())
        .collect();
    missing.reverse();
    for folder in missing {
        backup.made_folder(folder)?;
    }
    Ok(())
}

/// The backup in `slot`, the one `make` gives when there is none yet.
///
/// # Errors
///
/// `make`'s: a folder cannot be made, or a path is not UTF-8.
fn made(
    slot: &mut Option<Backup>,
    make: impl FnOnce() -> anyhow::Result<Backup>,
) -> anyhow::Result<&mut Backup> {
    match slot {
        Some(backup) => Ok(backup),
        None => Ok(slot.insert(make()?)),
    }
}

/// The user's backup folder, `~/.cortex/backups/`, made the user's own when missing.
///
/// # Errors
///
/// No home folder, or a folder that cannot be made or leads elsewhere.
fn user_backups() -> anyhow::Result<PathBuf> {
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    create_private_folder(user.path())
        .with_context(|| format!("creating {}", user.path().display()))?;
    place::own_folder(user.path(), "backups")
}
