//! The claims holding now, as `cortex claims list` reads them: from a kept store reader, positions
//! held as the coordination settings say.

use cortex_coord::Timing;
use cortex_core::ipc::ClaimLine;
use cortex_retrieve::RetrieveError;

use super::Service;

impl Service {
    /// The claims holding at `now_ms`, by file, then by session.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub(crate) fn claims(&self, now_ms: i64) -> Result<Vec<ClaimLine>, RetrieveError> {
        let timing = Timing::of(&self.coordination);
        self.with_reader(|queries| Ok(crate::claims::listed(queries, now_ms, &timing)?))
    }
}
