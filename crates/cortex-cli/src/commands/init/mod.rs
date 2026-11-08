//! `cortex init`: sets Cortex up in a repository, the top of the git repository holding the folder
//! it starts from (see [`root`]), for the agents `--agents` names or else those found on this
//! machine, once a Claude Code among them is a version Cortex's hooks work with. It asks its
//! questions once (see [`consent`]), registers Cortex's hooks with each agent (see [`hooks`]): for
//! Claude Code in the repository's `.claude/settings.local.json` (`--scope project`, the default, a
//! file Claude Code does not commit) or in the user's `~/.claude/settings.json` (`--scope user`),
//! keeps the answers, and has git leave out what Cortex writes (see [`ignores`]). Whatever the
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

mod consent;
mod daemon;
mod git;
mod hooks;
mod ignores;
mod place;
mod root;

use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, bail};
use cortex_core::{
    MAX_SETTINGS_BYTES, UserFolder, create_private_folder, read_text_file, unix_millis,
};
use cortex_hook::Versions;

use crate::agents::{self, Setup};
use crate::backup::{Backup, Kind};
use crate::commands::doctor::{Finding, checked, claude_code_version};
use crate::output;

use self::consent::{Answers, ConfigFile, Kept};
use self::ignores::Ignored;
pub(crate) use self::ignores::without_lines;
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
        let text = read_text_file(&file, MAX_SETTINGS_BYTES)?;
        Ok(Self {
            origin: file.display().to_string(),
            file,
            text,
        })
    }

    /// The file as a configuration, the user's own when `is_users`.
    fn config(&self, is_users: bool) -> ConfigFile<'_> {
        ConfigFile {
            text: self.text.as_deref(),
            origin: &self.origin,
            is_users,
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
        }
    }
}

/// Every file init may change, found, read and checked before anything is asked.
struct Files {
    /// The user's own configuration, `~/.cortex/config.toml`.
    users_config: Found,
    /// The repository's configuration, `.cortex/config.toml`.
    repository_config: Found,
    /// In a git repository, what git ignores and the `.gitignore` lines are added to.
    ignores: Option<(Ignored, Found)>,
    /// The settings each agent's hooks go to.
    settings: Vec<hooks::Settings>,
    /// The agents set up: their hooks planned, or with no hooks file Cortex writes; in order.
    set_up: Vec<&'static Setup>,
    /// What init says of each agent it found on this machine whose hooks cannot be planned: it
    /// is left out, and the others are set up all the same.
    not_set_up: Vec<String>,
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
                let ignored = ignores::read(&root.folder, &root.git_bound)?;
                let file = place::unlinked_file_in(&root.folder, ".gitignore")?;
                Some((ignored, Found::read(file)?))
            }
        };
        let (mut settings, mut set_up, mut not_set_up) = (Vec::new(), Vec::new(), Vec::new());
        for &setup in setups {
            match hooks::planned(scope, &root.folder, setup) {
                Ok(planned) => {
                    settings.extend(planned);
                    set_up.push(setup);
                }
                Err(error) if !are_named => not_set_up.push(format!(
                    "note: {} was found but is not set up: {error:#}; `cortex init --agents {}` \
                     sets it up once that is fixed",
                    setup.agent.title(),
                    setup.agent.name()
                )),
                Err(error) => return Err(error),
            }
        }
        Ok(Self {
            users_config: Found::read(place::file_beside(user.path(), "config.toml")?)?,
            repository_config: Found::read(repository_config)?,
            ignores,
            settings,
            set_up,
            not_set_up,
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
            self.users_config.config(true),
            self.repository_config.config(false),
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
                .filter_map(hooks::Settings::made_in_repository)
                .collect();
            let lines = ignores::needed(&ignored, answers.sharing, &made);
            changes.extend(ignore_change(gitignore, &lines));
        }
        for settings in self.settings {
            if settings.is_current() {
                output::line(&settings.current())?;
            } else {
                changes.push(settings.into_change());
            }
        }
        let repository =
            consent::repository_config(self.repository_config.config(false), kept, answers)?;
        if let Some(after) = repository {
            let owner = Owner::Repository {
                folder: Some(".cortex"),
            };
            changes.push(
                self.repository_config
                    .changed(after, owner, "answer kept in"),
            );
        }
        if let Some(after) = consent::users_config(self.users_config.config(true), kept, answers)? {
            changes.push(
                self.users_config
                    .changed(after, Owner::User, "answer kept in"),
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
}

/// Sets Cortex up in the repository `args` start from, as the module says, and says what it did.
///
/// # Errors
///
/// A folder that cannot be found or would be the home folder, a Claude Code too old, a question
/// left unanswered, a file that cannot be read, is read-only or would be written outside the
/// repository, or a backup or a write that fails.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
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
        check_claude_code(&root, versions)?;
    }
    let files = Files::found(args.scope, &root, &setups, are_named)?;
    for line in &files.not_set_up {
        output::line(line)?;
    }
    let set_up = files.set_up.clone();
    let kept = files.kept()?;
    let stdin = io::stdin();
    let mut terminal = stdin.is_terminal().then(|| stdin.lock());
    let answers = consent::answers(kept, args.yes, terminal.as_mut())?;
    let changes = files.changes(kept, answers)?;
    apply(&root.folder, changes)?;
    daemon::started_and_indexed(&root.folder)?;
    for setup in &set_up {
        for step in setup.told {
            output::line(&format!("{}: to do yourself: {step}", setup.agent.title()))?;
        }
    }
    let is_claude_set_up = set_up.iter().any(|setup| setup.agent.name() == "claude");
    let failed = if is_claude_set_up {
        checked(root.folder)?
    } else {
        0
    };
    if failed > 0 {
        output::line(&format!(
            "note: init is done, and {failed} of doctor's checks failed; each says what to do, \
             and a latency check over its budget on a busy machine may pass when `cortex doctor` \
             runs again"
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
    if let Some(why) = &root.without_git {
        output::line(&format!(
            "note: {why}; Cortex sets up this folder alone, without git history"
        ))?;
    }
    Ok(root)
}

/// Checks that Claude Code, asked its version in `root`'s folder, is one the `versions` Cortex
/// knows allow, and says what it found.
///
/// # Errors
///
/// Claude Code is older than the oldest of `versions`, or stdout cannot be written.
fn check_claude_code(root: &Root, versions: Versions) -> anyhow::Result<()> {
    match claude_code_version(&root.folder, versions) {
        Finding::Fail { what, fix } => bail!("{what}; {fix}, then run `cortex init` again"),
        Finding::Pass(what) => output::line(&what)?,
        Finding::Note(what) => output::line(&format!("note: {what}"))?,
    }
    Ok(())
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
    })
}

/// Writes each of `changes` for the repository whose canonical root is `root`, each backed up
/// first: the repository's files in a folder of this run's own in its `.cortex/backups/`, with
/// the manifest also in the user's `~/.cortex/backups/`, and the user's files there, each folder
/// made when a first file for it changes. Then says what it wrote and where the backups are, so
/// that a closed stdout never stops it partway.
///
/// # Errors
///
/// A folder or a backup cannot be made, a file changed since it was read, a write fails, or stdout
/// cannot be written.
fn apply(root: &Path, changes: Vec<Change>) -> anyhow::Result<()> {
    let now_ms = unix_millis(SystemTime::now());
    let mut repository: Option<Backup> = None;
    let mut user: Option<Backup> = None;
    let mut said = Vec::new();
    for change in changes {
        let backup = match change.owner {
            Owner::Repository { folder } => {
                if let Some(folder) = folder {
                    place::own_folder(root, folder)?;
                }
                made(&mut repository, || {
                    let backups =
                        place::own_folder(&place::own_folder(root, ".cortex")?, "backups")?;
                    Ok(Backup::new(&backups, now_ms, root)?.with_users_copy(&user_backups()?))
                })?
            }
            Owner::User => {
                // The user's backups are made first, so that `~/.cortex` is the user's alone
                // before the user's configuration goes into it.
                let backup = made(&mut user, || Backup::new(&user_backups()?, now_ms, root))?;
                if let Some(parent) = change.file.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                backup
            }
        };
        let before = change.before.as_deref().map(str::as_bytes);
        backup.replace(&change.file, change.kind, before, change.after.as_bytes())?;
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
