//! A credential's option given its value as the next word: `--token abc`, `--api-key abc`,
//! `--password abc`, `--client-secret abc`. The option is a word starting with `-` whose name, the
//! dashes taken off, is a credential's (see `names`) and is two characters or more. An option
//! written with its value (`--token=abc`) is the `keys` detector's.
//!
//! A one-letter option is never taken for a credential's, `-p` above all. It means a password
//! for some programs (`mysql -p`), but for every runner whose command is kept it means something
//! else: a plugin to pytest, a package to cargo, uv and npx, parallelism to `go test`, a project
//! to tsc and gradle, a property to dotnet (`-p:Name=value`, which `keys` reads). Its value is
//! still read by every other detector.

use super::detectors::Finding;
use super::names;

/// The least characters of an option's name for it to be taken for a credential's.
const MIN_NAME_CHARS: usize = 2;

/// Whether `word` is a credential's option whose value is the next word.
pub(super) fn find(word: &str) -> Vec<Finding> {
    if !word.starts_with('-') || word.contains(['=', ':']) {
        return Vec::new();
    }
    let name = word.trim_start_matches('-');
    if name.len() >= MIN_NAME_CHARS && names::is_credential(name) {
        vec![Finding::ValueNext]
    } else {
        Vec::new()
    }
}
