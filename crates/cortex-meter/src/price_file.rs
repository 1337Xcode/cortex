//! The user's price file, `~/.cortex/pricing.toml`: written from the table Cortex ships the first
//! time a price table is needed, and from then on read as the user left it.
//!
//! The shipped table is written to a file of its own beside the price file, made private to the
//! user, and then linked in under the price file's name. A link never replaces a file that is
//! there, so the user's prices are never overwritten, and a reader sees either no price file or a
//! whole one, never a table cut short by a crash or a second process writing at the same time.

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use cortex_core::create_new_private_file;

use crate::pricing::{PriceTable, PricingError, SHIPPED_PRICES};

/// Most bytes a price file may hold. The shipped table is a few kilobytes; a much larger file is
/// not a price table.
pub const MAX_PRICE_FILE_BYTES: u64 = 1024 * 1024;

/// The price table in the file at `path`, the shipped table written there first when no file is
/// there. A file that is there is never written: the user's prices win.
///
/// # Errors
///
/// [`PricingError::Io`] when the file cannot be written or read (its folder must exist, on a file
/// system that keeps links), [`PricingError::TooLarge`], [`PricingError::Syntax`] for a file that
/// is not UTF-8 text, and the errors of [`PriceTable::parse`].
pub fn load_or_write(path: &Path) -> Result<PriceTable, PricingError> {
    if !path.exists() {
        publish_shipped(path)?;
    }
    read_table(path)
}

/// The price table in the file at `path`, or none when no file is there; nothing is ever written.
/// For a caller that only shows prices, such as the map, which must not create the user's file.
///
/// # Errors
///
/// As [`load_or_write`], but for writing.
pub fn load(path: &Path) -> Result<Option<PriceTable>, PricingError> {
    if !path.exists() {
        return Ok(None);
    }
    read_table(path).map(Some)
}

/// The price table in the file at `path`, read whole up to [`MAX_PRICE_FILE_BYTES`].
fn read_table(path: &Path) -> Result<PriceTable, PricingError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_PRICE_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(io("reading", path))?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_PRICE_FILE_BYTES) {
        return Err(PricingError::TooLarge(path.to_path_buf()));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| PricingError::Syntax("the file is not UTF-8 text".to_owned()))?;
    PriceTable::parse(&text)
}

/// Puts the shipped table at `path`, unless a file got there first: written whole beside it, then
/// linked in under its name. The file written beside it is removed either way.
fn publish_shipped(path: &Path) -> Result<(), PricingError> {
    let mut beside = path.as_os_str().to_owned();
    beside.push(format!(".{}.new", std::process::id()));
    let beside = PathBuf::from(beside);
    let written = create_new_private_file(&beside).and_then(|mut file| {
        file.write_all(SHIPPED_PRICES.as_bytes())?;
        file.sync_all()
    });
    let linked = written.and_then(|()| match std::fs::hard_link(&beside, path) {
        // Another process put its copy, or the user their own, there in the meantime.
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        linked => linked,
    });
    let removed = std::fs::remove_file(&beside);
    linked.map_err(io("writing", path))?;
    removed.map_err(io("removing", &beside))
}

/// Turns an I/O error met while `action` the file at `path` into a pricing error.
fn io(action: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> PricingError {
    let path = path.to_path_buf();
    move |source| PricingError::Io {
        action,
        path,
        source,
    }
}
