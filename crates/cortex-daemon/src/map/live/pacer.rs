//! When the live view sends a frame: once a change is noted, as soon as the last frame is at least
//! the gap old, so however fast changes come the page gets at most one frame per gap, and a change
//! that came during the gap is sent at its end rather than lost.

use std::time::{Duration, Instant};

/// The pace of one live connection's frames.
#[derive(Debug, Clone)]
pub(super) struct Pacer {
    /// The least time between two frames.
    gap: Duration,
    /// When the last frame was sent, if one was.
    last_sent: Option<Instant>,
    /// Whether a change came since then.
    is_pending: bool,
}

impl Pacer {
    /// A pacer sending at most one frame per `gap`, with nothing sent yet and nothing pending.
    pub(super) fn new(gap: Duration) -> Self {
        Self {
            gap,
            last_sent: None,
            is_pending: false,
        }
    }

    /// Notes that something changed that a frame should show.
    pub(super) fn note_change(&mut self) {
        self.is_pending = true;
    }

    /// Whether a frame is due at `now`: a change is pending and the last frame is at least the gap
    /// old. A frame that is due is taken: the pacer counts it sent at `now`.
    pub(super) fn take_due(&mut self, now: Instant) -> bool {
        let is_gap_over = self
            .last_sent
            .is_none_or(|sent| now.saturating_duration_since(sent) >= self.gap);
        if !self.is_pending || !is_gap_over {
            return false;
        }
        self.is_pending = false;
        self.last_sent = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six changes in one second, one every 150 ms from 0 to 750 ms, looked at every 50 ms with a
    /// gap of 250 ms, give four frames: at 0 (the change at 0), 250 (the one at 150), 500 (300
    /// and 450 together) and 750 (600 and 750 together). No change, no frame.
    #[test]
    fn six_changes_in_a_second_give_four_frames() {
        let start = Instant::now();
        let mut pacer = Pacer::new(Duration::from_millis(250));
        let changes_ms = [0_u64, 150, 300, 450, 600, 750];
        let mut sent_ms = Vec::new();
        for at_ms in (0..1000).step_by(50) {
            if changes_ms.contains(&at_ms) {
                pacer.note_change();
            }
            if pacer.take_due(start + Duration::from_millis(at_ms)) {
                sent_ms.push(at_ms);
            }
        }
        assert_eq!(sent_ms, [0, 250, 500, 750]);
        assert!(!pacer.take_due(start + Duration::from_secs(5)));
    }
}
