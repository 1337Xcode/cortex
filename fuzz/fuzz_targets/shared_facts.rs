//! Fuzzes the reading of a shared facts file. The file is committed by a teammate, so any clone of
//! a repository can hand Cortex one built to break its reader, and what it says may reach an agent
//! once a person confirms it: it must be refused whole or read whole, never half read.
//!
//! The input is the file's text, as far as it is UTF-8; the reader is only ever given text, since
//! the command reads the file as text and refuses one that is not. Every input must be read or
//! refused: never a panic and never a hang. What is read is written back and read again, and must
//! give the same facts, so the writer and the reader agree on every file the reader accepts.

#![no_main]

use cortex_core::{MAX_SHARED_FACTS, facts_file, read_facts_file};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Ok(text) = std::str::from_utf8(input) else {
        return;
    };
    let Ok(facts) = read_facts_file(text) else {
        return;
    };
    assert!(facts.len() <= MAX_SHARED_FACTS, "read more facts than a file may hold");
    let written = facts_file(&facts, MAX_SHARED_FACTS).expect("facts that were read can be written");
    let again = read_facts_file(&written).expect("a file this build wrote can be read back");
    // The writer sorts the facts and each fact's anchors, so the round trip is compared as sorted.
    let sorted = |mut facts: Vec<cortex_core::SharedFact>| {
        for fact in &mut facts {
            fact.at.sort();
        }
        facts.sort_by(|left, right| {
            (&left.at, &left.text, left.kind.name()).cmp(&(&right.at, &right.text, right.kind.name()))
        });
        facts
    };
    assert_eq!(sorted(again), sorted(facts), "the round trip changed the facts");
});
