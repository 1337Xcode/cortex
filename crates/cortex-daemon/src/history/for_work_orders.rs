//! The history worker as a work order reads it: the texts it holds and the blames it is asked
//! for, through the narrow interface retrieval owns ([`WhyHistory`]). Neither waits for git.

use cortex_core::{ContentHash, LineRange, RepoPath};
use cortex_retrieve::{CommitText, WhyHistory};

use super::jobs::Jobs;
use super::queue::BlameJob;

impl WhyHistory for Jobs {
    /// The text cache's subject and reason line of `oid`.
    fn commit_text(&self, oid: &str) -> Option<CommitText> {
        self.text(oid).map(|text| CommitText {
            subject: text.subject,
            reason: text.reason,
        })
    }

    /// Asks the worker to read the commit `oid` again.
    fn ask_for_text(&self, oid: &str) {
        self.ask_text(oid);
    }

    /// Asks the worker for the blame, dropped when the worker does not run.
    fn ask_for_blame(&self, path: &RepoPath, file_hash: ContentHash, lines: LineRange) {
        self.ask_blame(BlameJob {
            path: path.clone(),
            file_hash,
            first_line: lines.start_line(),
            last_line: lines.end_line(),
        });
    }
}
