//! How long the daemon takes to handle each hook's requests, each from the first byte of the
//! request's frame to its answer being ready to write: the latest [`KEPT`] times per hook, and
//! their percentiles for the daemon's status. The hook's own process, its connection and the
//! handshake are not in these times.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use cortex_core::HookKind;
use cortex_core::ipc::HookLatency;

/// How many handling times are kept per hook. The percentiles are over these, the latest, so that
/// they show how the daemon does now rather than since it started.
const KEPT: usize = 1_000;

/// The latest handling times of each hook.
#[derive(Debug, Default, Clone)]
pub(crate) struct Latencies {
    /// Per hook, its latest handling times in microseconds, oldest first.
    times_us: BTreeMap<HookKind, VecDeque<u64>>,
}

impl Latencies {
    /// Keeps that a request of `hook` took `took` to handle, forgetting the oldest of that hook's
    /// times past [`KEPT`].
    pub(crate) fn note(&mut self, hook: HookKind, took: Duration) {
        let times = self.times_us.entry(hook).or_default();
        if times.len() == KEPT {
            times.pop_front();
        }
        times.push_back(u64::try_from(took.as_micros()).unwrap_or(u64::MAX));
    }

    /// The median, 95th and 99th percentiles of each hook's kept times, in hook order.
    pub(crate) fn report(&self) -> Vec<HookLatency> {
        self.times_us
            .iter()
            .map(|(hook, times)| {
                let mut sorted: Vec<u64> = times.iter().copied().collect();
                sorted.sort_unstable();
                HookLatency {
                    hook: hook.name().to_owned(),
                    requests: u64::try_from(sorted.len()).unwrap_or(u64::MAX),
                    p50_us: percentile(&sorted, 50),
                    p95_us: percentile(&sorted, 95),
                    p99_us: percentile(&sorted, 99),
                }
            })
            .collect()
    }
}

/// The `percent`th percentile of `sorted` by nearest rank: the shortest of the times that at least
/// `percent` in a hundred of them are no longer than. An empty list has none and reads as 0.
fn percentile(sorted: &[u64], percent: usize) -> u64 {
    let rank = (sorted.len() * percent).div_ceil(100).max(1);
    sorted.get(rank - 1).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report of one hook, of `latencies`.
    fn only(latencies: &Latencies) -> Result<HookLatency, String> {
        match latencies.report().as_slice() {
            [report] => Ok(report.clone()),
            other => Err(format!("expected one hook, got {other:?}")),
        }
    }

    /// One hundred times of 1 to 100 µs, noted longest first: by nearest rank the median is the
    /// 50th shortest, 50 µs, the 95th percentile the 95th, 95 µs, and the 99th the 99th, 99 µs.
    #[test]
    fn percentiles_are_by_nearest_rank() -> Result<(), String> {
        let mut latencies = Latencies::default();
        for micros in (1..=100).rev() {
            latencies.note(HookKind::PostTool, Duration::from_micros(micros));
        }
        let report = only(&latencies)?;
        assert_eq!(
            (report.requests, report.p50_us, report.p95_us, report.p99_us),
            (100, 50, 95, 99)
        );
        Ok(())
    }

    /// Three times of 10, 20 and 30 µs: the median's rank is 3 × 50 / 100 = 1.5, rounded up to 2,
    /// so 20 µs; the 95th and 99th percentiles' ranks, 2.85 and 2.97, round up to 3, so 30 µs. One
    /// time is every percentile.
    #[test]
    fn few_times_round_the_rank_up() -> Result<(), String> {
        let mut latencies = Latencies::default();
        for micros in [30, 10, 20] {
            latencies.note(HookKind::Prompt, Duration::from_micros(micros));
        }
        let report = only(&latencies)?;
        assert_eq!((report.p50_us, report.p95_us, report.p99_us), (20, 30, 30));
        let mut one = Latencies::default();
        one.note(HookKind::Stop, Duration::from_micros(7));
        let report = only(&one)?;
        assert_eq!(
            (report.requests, report.p50_us, report.p95_us, report.p99_us),
            (1, 7, 7, 7)
        );
        Ok(())
    }

    /// Only the latest 1,000 times count: 500 requests of a second each, then 1,000 of 1 to
    /// 1,000 µs, give the percentiles of the 1,000 alone (500, 950 and 990 µs). Were the 500 slow
    /// ones kept, the median would be the 750th of 1,500, 750 µs.
    #[test]
    fn only_the_latest_times_count() -> Result<(), String> {
        let mut latencies = Latencies::default();
        for _ in 0..500 {
            latencies.note(HookKind::PreTool, Duration::from_secs(1));
        }
        for micros in 1..=1_000 {
            latencies.note(HookKind::PreTool, Duration::from_micros(micros));
        }
        let report = only(&latencies)?;
        assert_eq!(
            (report.requests, report.p50_us, report.p95_us, report.p99_us),
            (1_000, 500, 950, 990)
        );
        Ok(())
    }

    /// Each hook keeps its own times, and the report lists the hooks that sent a request in hook
    /// order, whatever order they came in: the session start before the tool's result.
    #[test]
    fn each_hook_is_reported_on_its_own() {
        let mut latencies = Latencies::default();
        latencies.note(HookKind::PostTool, Duration::from_micros(40));
        latencies.note(HookKind::SessionStart, Duration::from_micros(900));
        latencies.note(HookKind::PostTool, Duration::from_micros(60));
        let reported: Vec<(String, u64, u64)> = latencies
            .report()
            .into_iter()
            .map(|report| (report.hook, report.requests, report.p99_us))
            .collect();
        let expected = [("session_start", 1, 900), ("post_tool", 2, 60)]
            .map(|(hook, requests, p99_us)| (hook.to_owned(), requests, p99_us));
        assert_eq!(reported, expected);
    }
}
