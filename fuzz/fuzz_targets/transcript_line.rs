//! Fuzzes what the usage meter makes of a line of an agent's transcript. The transcript is a file
//! the agent writes and Cortex only reads, so its shape is not Cortex's to guarantee: a line may
//! be truncated mid-write, hold any bytes a model emitted, or come from a newer agent whose format
//! has moved on.
//!
//! The first byte chooses the agent whose format reads the line; the rest is the line. A line is
//! carried through more than once with the same carried state, as a real transcript does, so that
//! what one line leaves behind for the next is exercised too. Every line must give a response or
//! none: never a panic, and never a hang.

#![no_main]

use cortex_core::Agent;
use cortex_meter::{Earlier, format_of};
use libfuzzer_sys::fuzz_target;

/// The agents whose transcripts are read, in a fixed order so an input keeps its meaning.
const AGENTS: &[Agent] = &[Agent::Claude, Agent::Codex];

fuzz_target!(|input: &[u8]| {
    let Some((chooser, line)) = input.split_first() else {
        return;
    };
    let Some(agent) = AGENTS.get(usize::from(*chooser) % AGENTS.len()) else {
        return;
    };
    let Some(format) = format_of(*agent) else {
        return;
    };
    let mut earlier = Earlier::default();
    // Twice with the same carried state: the second reading sees whatever the first left behind,
    // which is how a transcript's later lines meet its earlier ones.
    drop(format.response(line, &mut earlier));
    drop(format.response(line, &mut earlier));
});
