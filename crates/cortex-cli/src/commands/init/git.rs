//! Git as `cortex init` runs it: the program the `PATH` names outside a folder bounding every
//! repository around the one init works in (a repository could have placed a `git` inside itself),
//! run in a folder with no repository named by the environment, for at most 5 s and 64 KiB of
//! answer.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use cortex_core::{Captured, GIT};

/// How long git has to answer.
const GIT_WITHIN: Duration = Duration::from_secs(5);

/// The most of git's answer read: a path, or a few paths, however long.
const MAX_ANSWER_BYTES: u64 = 65_536;

/// What `git <args>` answered, run in `folder` by a git outside `bound`; or why it did not answer:
/// git is not on the `PATH`, or it did not end in time.
pub(super) fn run(args: &[&str], folder: &Path, bound: &Path) -> Result<Captured, String> {
    let git =
        cortex_core::find_program(GIT, bound).ok_or_else(|| "git is not on the PATH".to_owned())?;
    let mut command = Command::new(git);
    command
        .args(args)
        .current_dir(folder)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    cortex_core::capture(&mut command, GIT_WITHIN, MAX_ANSWER_BYTES)
        .map_err(|error| format!("git did not answer ({error})"))
}
