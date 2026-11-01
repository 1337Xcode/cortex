//! Fuzzes what the hook makes of the bytes an agent writes to its stdin: the Claude Code adapter
//! normalizes them as each event it registers. The working folder is always in a repository, so
//! that no path in the input reaches the file system. Every input must give an event, no event or
//! an error: never a panic, and never a hang.

#![no_main]

use std::path::{Path, PathBuf};

use cortex_core::Ulid;
use cortex_hook::{Located, Stamp, adapter};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|payload: &[u8]| {
    let Some(claude) = adapter("claude") else {
        return;
    };
    let locate = |_: &Path| {
        Some(Located {
            cwd: PathBuf::from("/repo/src"),
            root: PathBuf::from("/repo"),
            agent_root: PathBuf::from("/repo"),
        })
    };
    let stamp = Stamp {
        event_id: Ulid::from_seed(1, b"fuzz"),
        ts_ms: 1_700_000_000_000,
    };
    for entry in claude.hooks() {
        drop(claude.normalize(entry.event_name, payload, &locate, stamp));
    }
});
