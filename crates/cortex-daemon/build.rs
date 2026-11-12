//! Embeds the map's built interface: writes the table of every file under the workspace's
//! `ui/dist` into the build's output folder, where the daemon's `map::assets` includes it. With no
//! `ui/dist` the table is empty and the map says the binary was built without it.
//!
//! Cargo runs this again when the build folder changes, so a new build of the interface is
//! embedded by the next build of the daemon. While there is no build folder it watches the `ui`
//! folder, in which one would appear, or, with no `ui` folder either, this script alone: a path
//! that does not exist would make Cargo run the script, and build the daemon again, every time.

#![warn(clippy::pedantic)]
#![allow(
    clippy::print_stdout,
    reason = "a build script speaks to Cargo through its standard output"
)]

#[path = "src/map/table.rs"]
mod table;

use std::error::Error;
use std::path::{Path, PathBuf};

/// Writes the table and tells Cargo what to watch.
///
/// # Errors
///
/// Cargo did not say where the package or the output folder is, the build folder holds something
/// that cannot be embedded (see `table::list`), or the table cannot be written.
fn main() -> Result<(), Box<dyn Error>> {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("no package folder")?);
    let out = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("no output folder")?);
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .ok_or("the package is not two folders inside its workspace")?;
    let ui = workspace.join("ui");
    let dist = ui.join("dist");
    let entries = table::read(&dist)?;
    std::fs::write(out.join("map_assets.rs"), table::render(&entries)?)?;
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/map/table.rs");
    // `read` refused a `dist` that is a link or a file, so one that exists here is a folder.
    if dist.symlink_metadata().is_ok() {
        println!("cargo:rerun-if-changed={}", dist.display());
    } else if ui.symlink_metadata().is_ok() {
        println!("cargo:rerun-if-changed={}", ui.display());
    }
    Ok(())
}
