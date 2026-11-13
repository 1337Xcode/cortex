//! Where the memory of a graph build goes, for a repository large enough that it matters.
//!
//! Reads one repository's graph once, with dhat as the allocator, and writes `dhat-heap.json`
//! beside the working directory. What it does is what the daemon does when it rebuilds: open the
//! store, read every node and every link inside one transaction, build the graph, and in huge mode
//! prepare the view by files. Nothing else runs, so the profile is of that and not of a daemon's
//! start-up.
//!
//! ```text
//! cargo run --profile profiling --example graph_heap -- <repository>
//! ```
//!
//! The database's memory map is turned off, since a mapped file is not an allocation and only
//! makes the resident figure harder to read.

// This is a diagnostic run by hand from a terminal, and what it finds has to reach the person who
// ran it; the rule against printing is there to keep the product's output in one place, and an
// example is not the product.
#![allow(clippy::print_stdout, reason = "a diagnostic says what it found")]

use std::error::Error;
use std::path::PathBuf;

use cortex_retrieve::{GraphSettings, read_graph};
use cortex_store::Reader;

/// The profiler counts what the program asks for by being the allocator it asks. The unsafe this
/// needs is dhat's own; the attribute is not unsafe code.
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() -> Result<(), Box<dyn Error>> {
    let profiler = dhat::Profiler::new_heap();
    let repository = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("give the repository to read as the first argument")?;
    let folder = repository.join(".cortex");

    let reader = Reader::open(&folder, 0)?;
    let graph = reader.consistently(|queries| read_graph(queries, &GraphSettings::default()))?;
    println!(
        "{} nodes, {} links, {} bytes of structure",
        graph.len(),
        graph.link_count(),
        graph.size_bytes()
    );

    // The view by files is what the daemon prepares after a rebuild in huge mode, and it is the
    // step that took the daemon from 2.55 GB to 2.88 GB, so the profile covers it too.
    graph.prepare_by_files()?;
    println!("with the view by files: {} bytes", graph.size_bytes());

    drop(profiler);
    Ok(())
}
