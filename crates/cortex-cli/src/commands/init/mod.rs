//! `cortex init`: sets Cortex up in a repository, the top of the git repository holding the folder
//! it starts from (see [`root`]), once Claude Code is a version Cortex's hooks work with. It asks
//! its questions once (see [`consent`]), registers Cortex's hooks with Claude Code (see [`hooks`])
//! in the repository's `.claude/settings.local.json` (`--scope project`, the default, a file Claude
//! Code does not commit) or in the user's `~/.claude/settings.json` (`--scope user`), keeps the
//! answers, and has git leave out what Cortex writes (see [`ignores`]). Whatever the scope, the
//! repository is set up.
//!
//! Every file init may change is found, read and checked before anything is asked, and nothing is
//! written until every question is answered. Then the `.gitignore` is written first, so that a run
//! stopped partway never leaves Cortex's files for git to commit, and each file is backed up first,
//! with a manifest, in a folder of the run's own (the repository's files in its
//! `.cortex/backups/`, the user's in `~/.cortex/backups/`) and written atomically (see [`backup`]).
//! Nothing of a repository's is written outside it (see [`place`]). Running init again changes
//! nothing.
//!
//! Then it finds the repository's daemon or starts one, waits for its first index (see
//! [`daemon`]), and runs `cortex doctor`'s checks, whose failures it reports without failing: what
//! init writes is written by then.

mod backup;
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
use cortex_hook::{Adapter, Versions};

use crate::commands::doctor::{Finding, checked, claude_code_version};
use crate::output;

use self::backup::Backup;
use self::consent::{Answers, ConfigFile, Kept};
use self::ignores::Ignored;
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
}

/// Where Claude Code reads the hooks Cortex writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Scope {
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

    /// The change that writes `after` to the file, which is `owner`'s, saying `said` and the path.
    fn changed(self, after: String, owner: Owner, said: &str) -> Change {
        Change {
            said: format!("{said} {}", self.origin),
            file: self.file,
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
    /// Claude Code's settings the hooks go to.
    settings: hooks::Settings,
}

impl Files {
    /// The files init may change for `root` at `scope`, with the hooks of `adapter`.
    ///
    /// # Errors
    ///
    /// A file is read-only, cannot be read, would be written outside the repository or through a
    /// link git does not read, or git cannot say what it ignores.
    fn found(scope: Scope, root: &Root, adapter: &dyn Adapter) -> anyhow::Result<Self> {
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
        Ok(Self {
            users_config: Found::read(place::file_beside(user.path(), "config.toml")?)?,
            repository_config: Found::read(repository_config)?,
            ignores,
            settings: hooks::planned(scope, &root.folder, adapter)?,
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
            let is_made = self.settings.is_made_in_repository();
            let lines = ignores::needed(ignored, answers.sharing, is_made);
            changes.extend(ignore_change(gitignore, &lines));
        }
        if self.settings.is_current() {
            output::line(&format!(
                "Claude Code: the hooks in {} are current",
                self.settings.file.display()
            ))?;
        } else {
            changes.push(self.settings.into_change());
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
    let adapter = cortex_hook::adapter("claude").context("finding the Claude Code adapter")?;
    let root = checked_root(args, adapter.versions())?;
    let files = Files::found(args.scope, &root, adapter)?;
    let kept = files.kept()?;
    let stdin = io::stdin();
    let mut terminal = stdin.is_terminal().then(|| stdin.lock());
    let answers = consent::answers(kept, args.yes, terminal.as_mut())?;
    let changes = files.changes(kept, answers)?;
    apply(&root.folder, changes)?;
    daemon::started_and_indexed(&root.folder)?;
    let failed = checked(root.folder)?;
    if failed > 0 {
        output::line(&format!(
            "note: init is done, and {failed} of doctor's checks failed; each says what to do, \
             and a latency check over its budget on a busy machine may pass when `cortex doctor` \
             runs again"
        ))?;
    }
    Ok(())
}

/// The repository init sets up from where `args` start it, said on stdout with why it is not
/// git's top when it is not, once Claude Code, asked its version, is one the `versions` Cortex
/// knows allow.
///
/// # Errors
///
/// The folder or the home folder cannot be found, the folder would be the home folder or hold it,
/// Claude Code is older than the oldest of `versions`, or stdout cannot be written.
fn checked_root(args: &Args, versions: Versions) -> anyhow::Result<Root> {
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
    match claude_code_version(&root.folder, versions) {
        Finding::Fail { what, fix } => bail!("{what}; {fix}, then run `cortex init` again"),
        Finding::Pass(what) => output::line(&what)?,
        Finding::Note(what) => output::line(&format!("note: {what}"))?,
    }
    Ok(root)
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
        before: found.text,
        owner: Owner::Repository { folder: None },
    })
}

/// Writes each of `changes` for the repository whose canonical root is `root`, each backed up
/// first: the repository's files in a folder of this run's own in its `.cortex/backups/`, the
/// user's in `~/.cortex/backups/`, each folder made when a first file for it changes. Then says
/// what it wrote and where the backups are, so that a closed stdout never stops it partway.
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
                made(&mut repository, now_ms, || {
                    place::own_folder(&place::own_folder(root, ".cortex")?, "backups")
                })?
            }
            Owner::User => {
                // The user's backups are made first, so that `~/.cortex` is the user's alone
                // before the user's configuration goes into it.
                let backup = made(&mut user, now_ms, user_backups)?;
                if let Some(parent) = change.file.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                backup
            }
        };
        let before = change.before.as_deref().map(str::as_bytes);
        backup.replace(&change.file, before, change.after.as_bytes())?;
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

/// The backup in `slot`, made for a run at `now_ms` in the folder `folder` gives when there is
/// none yet.
///
/// # Errors
///
/// The folder cannot be made.
fn made(
    slot: &mut Option<Backup>,
    now_ms: i64,
    folder: impl FnOnce() -> anyhow::Result<PathBuf>,
) -> anyhow::Result<&mut Backup> {
    match slot {
        Some(backup) => Ok(backup),
        None => Ok(slot.insert(Backup::new(&folder()?, now_ms))),
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
