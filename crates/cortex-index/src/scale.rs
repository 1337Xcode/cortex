//! How large a repository is, as the scale modes count it, and the mode it is in.
//!
//! A source file is a stored file in a language Cortex reads that is not generated, vendored,
//! binary, over the size limit or one that may hold secrets; its lines are counted. A repository
//! is huge with more than 20,000 source files or a million lines; else tiny with fewer than 50 or
//! fewer than 5,000 lines; else small with at most 250; else medium. A mode set in the
//! configuration holds whatever the size. Hysteresis, keeping a mode near a boundary, is upkeep's.

use cortex_core::{FileFlags, ModeSetting, ScaleMode};
use cortex_store::{MetaKey, StoreError, Writer};

use crate::discover::DiscoveredFile;

/// More source files than this make a repository huge.
const HUGE_ABOVE_FILES: u64 = 20_000;

/// More lines of source than this make a repository huge.
const HUGE_ABOVE_LINES: u64 = 1_000_000;

/// Fewer source files than this make a repository that is not huge tiny.
const TINY_BELOW_FILES: u64 = 50;

/// Fewer lines of source than this make a repository that is not huge tiny.
const TINY_BELOW_LINES: u64 = 5_000;

/// At most this many source files make a repository that is neither huge nor tiny small.
const SMALL_UP_TO_FILES: u64 = 250;

/// The flags a source file has none of.
const NOT_SOURCE: FileFlags = FileFlags::GENERATED
    .with(FileFlags::VENDORED)
    .with(FileFlags::BINARY)
    .with(FileFlags::TOO_LARGE)
    .with(FileFlags::SECRET_PATTERN);

/// Whether an index of the walk's `files` takes inventory of the files never parsed, as huge mode
/// does: when `setting` pins huge or, left to the size, when the last index recorded huge or the
/// files are more than 20,000 source files. Lines are not known before files are read, so a
/// repository huge by its lines alone is found so by its first index, which reads every file.
///
/// # Errors
///
/// Store errors.
pub(crate) fn takes_inventory(
    writer: &Writer,
    setting: ModeSetting,
    files: &[DiscoveredFile],
) -> Result<bool, StoreError> {
    if let ModeSetting::Pinned(mode) = setting {
        return Ok(mode == ScaleMode::Huge);
    }
    let recorded = writer.queries().meta(MetaKey::ScaleMode)?;
    let was_huge = recorded.as_deref().and_then(ScaleMode::from_name) == Some(ScaleMode::Huge);
    Ok(was_huge || is_huge_by_files(files))
}

/// Whether the walk's `files` make a repository huge by their number alone, before any is read:
/// more than 20,000 source files.
fn is_huge_by_files(files: &[DiscoveredFile]) -> bool {
    let source = files
        .iter()
        .filter(|file| file.language.is_some() && file.flags.bits() & NOT_SOURCE.bits() == 0)
        .count();
    u64::try_from(source).unwrap_or(u64::MAX) > HUGE_ABOVE_FILES
}

/// A repository's size as the scale modes count it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepoSize {
    /// Its source files.
    pub source_files: u64,
    /// The lines they hold.
    pub lines: u64,
}

/// The mode a repository of `size` is in.
#[must_use]
pub fn mode_for(size: RepoSize) -> ScaleMode {
    if size.source_files > HUGE_ABOVE_FILES || size.lines > HUGE_ABOVE_LINES {
        ScaleMode::Huge
    } else if size.source_files < TINY_BELOW_FILES || size.lines < TINY_BELOW_LINES {
        ScaleMode::Tiny
    } else if size.source_files <= SMALL_UP_TO_FILES {
        ScaleMode::Small
    } else {
        ScaleMode::Medium
    }
}

/// Counts the stored repository, settles its mode by `setting`, and records the mode.
///
/// # Errors
///
/// Store errors.
pub(crate) fn settle_mode(
    writer: &mut Writer,
    setting: ModeSetting,
) -> Result<(ScaleMode, RepoSize), StoreError> {
    let (source_files, lines) = writer.queries().source_size(NOT_SOURCE)?;
    let size = RepoSize {
        source_files,
        lines,
    };
    let mode = match setting {
        ModeSetting::Auto => mode_for(size),
        ModeSetting::Pinned(mode) => mode,
    };
    let mut transaction = writer.transaction()?;
    transaction.set_scale_mode(mode)?;
    transaction.commit()?;
    Ok((mode, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each boundary from both sides, worked out from the thresholds: huge is tested first, so a
    /// repository of few files but over a million lines is huge; tiny before small, so 50 files
    /// with 4,999 lines are tiny.
    #[test]
    fn modes_follow_the_size_thresholds() {
        let cases = [
            (49, 10_000, ScaleMode::Tiny),
            (50, 4_999, ScaleMode::Tiny),
            (50, 5_000, ScaleMode::Small),
            (250, 5_000, ScaleMode::Small),
            (251, 5_000, ScaleMode::Medium),
            (20_000, 1_000_000, ScaleMode::Medium),
            (20_001, 1_000_000, ScaleMode::Huge),
            (20_000, 1_000_001, ScaleMode::Huge),
            (10, 1_000_001, ScaleMode::Huge),
            (0, 0, ScaleMode::Tiny),
        ];
        for (source_files, lines, expected) in cases {
            let size = RepoSize {
                source_files,
                lines,
            };
            assert_eq!(
                mode_for(size),
                expected,
                "{source_files} files, {lines} lines"
            );
        }
    }
}
