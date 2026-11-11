//! What Cortex keeps, in the user's own folder, of setting repositories up on its own: the switch
//! that turns it on (`~/.cortex/adopt`, the `adopt.enabled` key), the folders it refused by its
//! rules (`~/.cortex/run/refused/<id>`, the newest [`MAX_REFUSALS`], each its folder and why, so
//! that hooks stop offering them), the set-ups that failed (`~/.cortex/logs/adopt.log`, never
//! remembered as refusals: the next hook offers the folder again, at most once in ten seconds),
//! and the lines `cortex status` and `cortex doctor` say of them.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context as _;
use cortex_core::{
    RepoId, UserFolder, append_private_file, create_private_folder, one_line, open_private_file,
    rotate_log, unix_millis,
};

/// How many refusals are remembered, the oldest forgotten first: a hook offers a forgotten folder
/// once more, and is refused once more.
pub(crate) const MAX_REFUSALS: usize = 256;

/// How many of the newest refusals `cortex status` and `cortex doctor` name.
const SHOWN_REFUSALS: usize = 3;

/// The size past which the log of failed set-ups starts a new file.
const LOG_ROTATE_AT_BYTES: u64 = 1024 * 1024;

/// How many files that log keeps: the current one and two older ones.
const LOG_KEPT_FILES: usize = 3;

/// Most characters of a line of that log: a folder and an error, however long.
const MAX_LOG_LINE_CHARS: usize = 2_000;

/// One folder refused, as remembered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused {
    /// The folder, as written when it was refused.
    pub(crate) folder: String,
    /// Why.
    pub(crate) why: String,
}

/// Turns the switch whose file is `path` on (writes it, its folder made private when missing) or
/// off (removes it), and says whether it was on.
///
/// # Errors
///
/// The file or its folder cannot be written or removed.
pub(crate) fn switch(path: &Path, is_on: bool) -> anyhow::Result<bool> {
    let was_on = path.is_file();
    if is_on {
        if let Some(folder) = path.parent() {
            create_private_folder(folder)
                .with_context(|| format!("creating {}", folder.display()))?;
        }
        open_private_file(path).with_context(|| format!("writing {}", path.display()))?;
    } else if was_on {
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(was_on)
}

/// Remembers, in the user's folder `user`, that `folder` was refused by a rule, `why`, then
/// forgets the oldest refusals past [`MAX_REFUSALS`]. The file is written whole beside its place
/// and moved there, so that a reader finds the whole of it or nothing.
///
/// # Errors
///
/// The folder of refusals or the file cannot be written.
pub(crate) fn remember(user: &UserFolder, folder: &Path, why: &str) -> anyhow::Result<()> {
    let refused = user.refused_folder();
    create_private_folder(&refused).with_context(|| format!("creating {}", refused.display()))?;
    let path = user.refusal_path(&RepoId::of(folder));
    let written = path.with_extension("new");
    let mut file =
        open_private_file(&written).with_context(|| format!("writing {}", written.display()))?;
    file.set_len(0)
        .and_then(|()| writeln!(file, "{}\n{}", folder.display(), one_line(why, 1_000)))
        .with_context(|| format!("writing {}", written.display()))?;
    drop(file);
    fs::rename(&written, &path).with_context(|| format!("writing {}", path.display()))?;
    forget_oldest(&refused)
}

/// Forgets the refusal of `folder` remembered in the user's folder `user`, and says whether there
/// was one.
///
/// # Errors
///
/// It is there and cannot be removed.
pub(crate) fn forget(user: &UserFolder, folder: &Path) -> anyhow::Result<bool> {
    let path = user.refusal_path(&RepoId::of(folder));
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

/// Forgets every refusal remembered in the user's folder `user`.
///
/// # Errors
///
/// The folder is there and cannot be removed.
pub(crate) fn forget_all(user: &UserFolder) -> anyhow::Result<()> {
    let refused = user.refused_folder();
    match fs::remove_dir_all(&refused) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", refused.display())),
    }
}

/// Writes to the log of failed set-ups, in the user's folder `user`, that setting `folder` up
/// failed with `error`, at `now`.
///
/// # Errors
///
/// The log cannot be written.
pub(crate) fn log_failure(
    user: &UserFolder,
    folder: &Path,
    error: &anyhow::Error,
    now: SystemTime,
) -> anyhow::Result<()> {
    let logs = user.logs_folder();
    create_private_folder(&logs).with_context(|| format!("creating {}", logs.display()))?;
    let path = logs.join("adopt.log");
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= LOG_ROTATE_AT_BYTES) {
        // A log a reader holds open cannot move aside on Windows: the line then goes on its end.
        drop(rotate_log(&path, LOG_KEPT_FILES));
    }
    let line = format!(
        "{} {}: Cortex could not set it up: {error:#}",
        unix_millis(now),
        folder.display()
    );
    let mut file =
        append_private_file(&path).with_context(|| format!("writing {}", path.display()))?;
    writeln!(file, "{}", one_line(&line, MAX_LOG_LINE_CHARS))
        .with_context(|| format!("writing {}", path.display()))
}

/// The refusal of `folder` remembered in the user's folder `user`, if there is one.
///
/// # Errors
///
/// It is there and cannot be read.
pub(crate) fn refusal_of(user: &UserFolder, folder: &Path) -> anyhow::Result<Option<Refused>> {
    let path = user.refusal_path(&RepoId::of(folder));
    match fs::read_to_string(&path) {
        Ok(text) => Ok(Some(read_refusal(&text))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// A refusal's file `text`: its folder's line, then why.
fn read_refusal(text: &str) -> Refused {
    let mut lines = text.lines();
    Refused {
        folder: lines.next().unwrap_or_default().to_owned(),
        why: lines.next().unwrap_or_default().to_owned(),
    }
}

/// How many refusals the user's folder `user` remembers, and the newest [`SHOWN_REFUSALS`] of
/// them, newest first, by when each was written, then by name.
///
/// # Errors
///
/// The folder of refusals or a refusal cannot be read.
pub(crate) fn latest(user: &UserFolder) -> anyhow::Result<(usize, Vec<Refused>)> {
    let mut files = written(&user.refused_folder())?;
    files.sort();
    files.reverse();
    let count = files.len();
    let mut shown = Vec::new();
    for (_, path) in files.into_iter().take(SHOWN_REFUSALS) {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        shown.push(read_refusal(&text));
    }
    Ok((count, shown))
}

/// What `cortex status` and `cortex doctor` say of adoption for the user whose folder is `user`:
/// whether it is on, how many folders it refused, and the newest of them with why.
///
/// # Errors
///
/// The refusals cannot be read.
pub(crate) fn lines(user: &UserFolder) -> anyhow::Result<Vec<String>> {
    let is_on = user.adopt_path().is_file();
    let (count, shown) = latest(user)?;
    let mut lines = vec![format!(
        "adoption: {}; {} refused by its rules{}",
        if is_on { "on" } else { "off" },
        if count == 1 {
            "1 folder".to_owned()
        } else {
            format!("{count} folders")
        },
        if shown.is_empty() {
            String::new()
        } else {
            format!(", the newest {} below", shown.len())
        }
    )];
    lines.extend(
        shown
            .iter()
            .map(|refused| format!("adoption: refused {}: {}", refused.folder, refused.why)),
    );
    Ok(lines)
}

/// Removes the files of `refused` written longest ago past [`MAX_REFUSALS`].
///
/// # Errors
///
/// The folder or a file cannot be read or removed.
fn forget_oldest(refused: &Path) -> anyhow::Result<()> {
    let mut files = written(refused)?;
    files.sort();
    let past = files.len().saturating_sub(MAX_REFUSALS);
    for (_, path) in files.into_iter().take(past) {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// Each refusal in `refused` with when it was written; none when the folder is not there. A file
/// being written (`.new`) is not one yet.
///
/// # Errors
///
/// The folder or a file's metadata cannot be read.
fn written(refused: &Path) -> anyhow::Result<Vec<(SystemTime, PathBuf)>> {
    let entries = match fs::read_dir(refused) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", refused.display())),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", refused.display()))?;
        let path = entry.path();
        if path.extension().is_some() {
            continue;
        }
        let at = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .with_context(|| format!("reading {}", path.display()))?;
        files.push((at, path));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Past the bound, the refusals written longest ago go first; at the bound, none goes. The
    /// newest are shown first.
    #[test]
    fn the_oldest_refusals_are_forgotten() -> anyhow::Result<()> {
        let home = tempfile::tempdir()?;
        let user = UserFolder::in_home(home.path());
        let folder = user.refused_folder();
        fs::create_dir_all(&folder)?;
        let start = SystemTime::now() - Duration::from_secs(10_000);
        for at in 0..MAX_REFUSALS + 2 {
            let path = folder.join(format!("{at:04}"));
            fs::write(&path, format!("/f{at}\nwhy {at}\n"))?;
            let file = fs::File::options().write(true).open(&path)?;
            file.set_modified(start + Duration::from_secs(u64::try_from(at)?))?;
        }
        forget_oldest(&folder)?;
        assert!(!folder.join("0000").exists());
        assert!(!folder.join("0001").exists());
        assert!(folder.join("0002").exists());
        assert_eq!(fs::read_dir(&folder)?.count(), MAX_REFUSALS);
        let (count, shown) = latest(&user)?;
        assert_eq!(count, MAX_REFUSALS);
        assert_eq!(
            shown.first(),
            Some(&Refused {
                folder: format!("/f{}", MAX_REFUSALS + 1),
                why: format!("why {}", MAX_REFUSALS + 1),
            })
        );
        Ok(())
    }

    /// A refusal is remembered as its folder and why, forgotten alone or with every other; with
    /// none, the lines say adoption's state and no folder.
    #[test]
    fn a_refusal_is_remembered_and_forgotten() -> anyhow::Result<()> {
        let home = tempfile::tempdir()?;
        let user = UserFolder::in_home(home.path());
        assert_eq!(
            lines(&user)?,
            ["adoption: off; 0 folders refused by its rules"]
        );
        let folder = home.path().join("repo");
        remember(&user, &folder, "it is outside the home folder")?;
        switch(&user.adopt_path(), true)?;
        assert_eq!(
            lines(&user)?,
            [
                "adoption: on; 1 folder refused by its rules, the newest 1 below".to_owned(),
                format!(
                    "adoption: refused {}: it is outside the home folder",
                    folder.display()
                ),
            ]
        );
        assert!(forget(&user, &folder)?);
        assert!(!forget(&user, &folder)?);
        remember(&user, &folder, "again")?;
        forget_all(&user)?;
        assert_eq!(latest(&user)?.0, 0);
        assert!(switch(&user.adopt_path(), false)?);
        assert!(!switch(&user.adopt_path(), false)?);
        Ok(())
    }
}
