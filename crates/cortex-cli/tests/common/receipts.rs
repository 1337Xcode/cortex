//! `cortex receipts` as the tests run it, and the tally of responses its JSON shows.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

use super::{TestResult, isolate};

/// Runs `cortex receipts --root <root>` with `extra` arguments and `home` as the home folder.
pub fn run(root: &Path, home: &Path, extra: &[&str]) -> TestResult<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["receipts", "--root"]).arg(root).args(extra);
    isolate(&mut command, home);
    Ok(command.output()?)
}

/// The JSON of `turns` responses that took `[input in all, uncached, read from the cache,
/// written to it, output]` tokens, every one priced, at `cost` picodollars.
pub fn tally(turns: u64, [input, uncached, reads, writes, output]: [u64; 5], cost: u64) -> Value {
    json!({
        "turns": turns,
        "cumulative_input": input,
        "uncached_input": uncached,
        "cache_reads": reads,
        "cache_writes": writes,
        "output": output,
        "estimate": {"picodollars": cost, "priced_turns": turns, "unpriced_turns": {}},
    })
}
