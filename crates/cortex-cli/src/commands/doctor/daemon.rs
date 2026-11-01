//! The check on the repository's daemon: it answers, and runs this build. Doctor only asks; it
//! starts, stops and replaces nothing.

use std::time::{Duration, Instant};

use cortex_core::ipc::{CORTEX_VERSION, RequestBody, ResponseBody, Role};
use cortex_core::{RepoId, UserFolder};
use cortex_hook::{Answer, ClientError, ask};

use super::{Context, Finding};

/// How long the daemon has to answer: it answers status from memory, so one that takes longer is
/// stuck, not busy.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);

/// The repository's daemon answers, and runs this build.
pub(super) fn answers(context: &Context) -> Vec<Finding> {
    let Some(user) = UserFolder::of_current_user() else {
        return vec![Finding::Fail {
            what: "no home folder to find the daemon from".to_owned(),
            fix: "set HOME (USERPROFILE on Windows) to the user's home folder".to_owned(),
        }];
    };
    let files = user.run_files(&RepoId::of(&context.root));
    let start = format!(
        "start it with `cortex daemon --root {}`, or open Claude Code there: its first hook \
         starts one",
        context.root.display()
    );
    let log = format!(
        "see {}",
        user.logs_folder()
            .join(format!("daemon-{}.log", RepoId::of(&context.root)))
            .display()
    );
    let finding = match ask(
        &files,
        Role::Cli,
        RequestBody::Status,
        Instant::now() + ANSWER_WITHIN,
    ) {
        Ok(Answer {
            body: ResponseBody::Status(report),
            ..
        }) if report.cortex_version == CORTEX_VERSION => Finding::Pass(format!(
            "running (process {}, Cortex {})",
            report.pid, report.cortex_version
        )),
        Ok(Answer {
            body: ResponseBody::Status(report),
            ..
        }) => Finding::Fail {
            what: format!(
                "the daemon runs Cortex {}, not this {CORTEX_VERSION}",
                report.cortex_version
            ),
            fix: "run `cortex status`, which replaces an older daemon; a newer one comes from \
                  another installed cortex"
                .to_owned(),
        },
        Ok(_) => Finding::Fail {
            what: "the daemon answered without its status".to_owned(),
            fix: log,
        },
        Err(ClientError::NotRunning) => Finding::Fail {
            what: "not running".to_owned(),
            fix: start,
        },
        Err(ClientError::TimedOut) => Finding::Fail {
            what: format!("not answering within {} s", ANSWER_WITHIN.as_secs()),
            fix: log,
        },
        Err(ClientError::Mismatch(theirs)) => Finding::Fail {
            what: format!(
                "a daemon of Cortex {} that speaks another protocol",
                theirs.cortex_version
            ),
            fix: "run `cortex status`, which replaces an older daemon".to_owned(),
        },
        Err(other) => Finding::Fail {
            what: format!("unreachable ({other})"),
            fix: log,
        },
    };
    vec![finding]
}
