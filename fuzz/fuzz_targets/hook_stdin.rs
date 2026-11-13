//! Fuzzes what the hook makes of the bytes an agent writes to its stdin: the Claude Code adapter
//! normalizes them as each event it registers. The working folder is always in a repository, so
//! that no path in the input reaches the file system. Every input must give an event, no event or
//! an error: never a panic, and never a hang.

#![no_main]

use std::path::{Path, PathBuf};

use cortex_core::Ulid;
use cortex_hook::{Located, Stamp, adapter};
use libfuzzer_sys::fuzz_target;

/// A working folder inside an ordinary repository.
fn in_a_repository(_: &Path) -> Option<Located> {
    Some(Located {
        cwd: PathBuf::from("/repo/src"),
        root: PathBuf::from("/repo"),
        agent_root: PathBuf::from("/repo"),
        main_root: None,
    })
}

/// A working folder inside a linked working tree, whose repository is set up elsewhere: the
/// paths an adapter has to hold apart are not the same as above.
fn in_a_linked_working_tree(_: &Path) -> Option<Located> {
    Some(Located {
        cwd: PathBuf::from("/trees/feature/src"),
        root: PathBuf::from("/trees/feature"),
        agent_root: PathBuf::from("/trees/feature"),
        main_root: Some(PathBuf::from("/repo")),
    })
}

fuzz_target!(|payload: &[u8]| {
    let Some(claude) = adapter("claude") else {
        return;
    };
    let stamp = Stamp {
        event_id: Ulid::from_seed(1, b"fuzz"),
        ts_ms: 1_700_000_000_000,
    };
    for entry in claude.hooks() {
        drop(claude.normalize(entry.event_name, payload, &in_a_repository, stamp));
        drop(claude.normalize(entry.event_name, payload, &in_a_linked_working_tree, stamp));
    }
});
