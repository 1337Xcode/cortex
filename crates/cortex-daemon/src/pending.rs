//! What the hook events the writer records leave it to do once they are: learn from the tasks they
//! ended, and read the transcripts their stops named. Both wait while an index or a mining holds
//! the writer, and are done between the batches it records, the events that wait meanwhile
//! recorded between one task or transcript and the next.

use cortex_core::Config;
use cortex_learn::Recorder;
use cortex_store::Writer;

use crate::host::{Recorded, record_waiting};
use crate::inbox::Inbox;
use crate::learning::Learning;
use crate::meter::Meter;

/// The tasks that wait to be learned from and the transcripts that wait to be read.
#[derive(Debug)]
pub(crate) struct Pending {
    /// The tasks, and what learns from them.
    learning: Learning,
    /// The transcripts, and what reads them.
    meter: Meter,
}

impl Pending {
    /// Nothing pending, the transcripts read with `meter`.
    pub(crate) fn new(meter: Meter) -> Self {
        Self {
            learning: Learning::default(),
            meter,
        }
    }

    /// Adds what a recorded batch left: the tasks it ended and the transcripts it named.
    pub(crate) fn wait(&mut self, recorded: Recorded) {
        self.learning.wait(recorded.ended);
        self.meter.wait(recorded.transcripts);
    }

    /// Whether nothing waits.
    pub(crate) fn is_idle(&self) -> bool {
        self.learning.is_idle() && self.meter.is_idle()
    }

    /// Learns from the tasks waiting, with the settings of `config`, then reads the transcripts
    /// waiting, those the events recorded meanwhile named among them, through `writer`; after each
    /// task and each transcript, records with `recorder` the hook events waiting in `inbox`. What
    /// those events leave waits for the next time. Whether a task was learned from.
    pub(crate) fn settle(
        &mut self,
        (writer, recorder): (&mut Writer, &mut Recorder),
        inbox: &Inbox,
        config: &Config,
    ) -> bool {
        let meter = &mut self.meter;
        let is_learned = self.learning.learn(writer, (inbox, config), |writer| {
            let recorded = record_waiting(writer, recorder, inbox);
            meter.wait(recorded.transcripts);
            recorded.ended
        });
        let learning = &mut self.learning;
        self.meter.read(writer, inbox, |writer| {
            let recorded = record_waiting(writer, recorder, inbox);
            learning.wait(recorded.ended);
            recorded.transcripts
        });
        is_learned
    }
}
