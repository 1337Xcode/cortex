//! The daemon's answer to `cortex why` and the `cortex_why` tool: the retrieval crate's answer,
//! read from a kept reader with the repository's settings, positions held as the coordination
//! settings say. Before the first index there is nothing a target could name, and the answer says
//! so instead of refusing the target.

use cortex_coord::Timing;
use cortex_retrieve::{RetrieveError, Why, WhyAt, why};

use super::{Service, mode_of};

impl Service {
    /// Why `target` is where it is, as the store and the files on disk say at `now_ms`.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub(crate) fn why(&self, target: &str, now_ms: i64) -> Result<Why, RetrieveError> {
        let timing = Timing::of(&self.coordination);
        self.with_reader(|queries| {
            if mode_of(queries)?.is_none() {
                return Ok(Why::Refused(
                    "the repository is not indexed yet: try again once it is".to_owned(),
                ));
            }
            let at = WhyAt {
                now_ms,
                timing: &timing,
                root: &self.root,
                max_file_bytes: self.max_file_bytes,
                link_decay: self.edits.link_decay,
            };
            Ok(why(queries, target, &at)?)
        })
    }
}
