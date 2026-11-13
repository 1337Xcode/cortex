//! Fuzzes what Cortex makes of a file in a repository it did not write: arbitrary bytes read as
//! source in each language it knows. A repository is untrusted input, and this is the widest way
//! in, since every file an agent touches is read this way.
//!
//! The first byte of the input chooses the language, so one corpus covers them all and libFuzzer
//! can steer towards the parser that is hardest to satisfy; the rest is the file's contents. Every
//! input must give an extraction or an error, never a panic and never a hang: the extractor is
//! given a short deadline, and a file that passes it is a failure the deadline is there to turn
//! into an error.

#![no_main]

use std::sync::Arc;
use std::time::Duration;

use cortex_core::{Language, RepoPath};
use cortex_index::{Extractor, Grammars};
use libfuzzer_sys::fuzz_target;

/// How long one file may take. Short, so that a hang shows as an error rather than as the fuzzer
/// stalling, and long enough that an ordinary file is read.
const TIMEOUT: Duration = Duration::from_millis(500);

/// The languages fed, in a fixed order so an input keeps its meaning between runs.
const LANGUAGES: &[Language] = &Language::ALL;

fuzz_target!(|input: &[u8]| {
    let Some((chooser, source)) = input.split_first() else {
        return;
    };
    let Some(language) = LANGUAGES.get(usize::from(*chooser) % LANGUAGES.len()) else {
        return;
    };
    // Loading the grammars costs about a second, so it is done once and kept.
    thread_local! {
        static EXTRACTOR: Option<std::cell::RefCell<Extractor>> = Grammars::load()
            .ok()
            .map(|grammars| std::cell::RefCell::new(Extractor::new(Arc::new(grammars), TIMEOUT)));
    }
    let Ok(path) = RepoPath::new("src/fuzzed") else {
        return;
    };
    EXTRACTOR.with(|held| {
        if let Some(extractor) = held {
            drop(extractor.borrow_mut().extract(&path, *language, source));
        }
    });
});
