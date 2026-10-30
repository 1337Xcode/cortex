//! Helpers shared by the indexer's integration tests. Each test file uses part of them.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

use std::error::Error;
use std::fs::{self, File};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use cortex_index::Grammars;

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Every grammar, loaded once for all the tests of one test file: compiling the queries takes
/// longer than anything a test reads.
pub fn grammars() -> Result<Arc<Grammars>, Box<dyn Error>> {
    static GRAMMARS: OnceLock<Result<Arc<Grammars>, String>> = OnceLock::new();
    GRAMMARS
        .get_or_init(|| {
            Grammars::load()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(Into::into)
}

/// Writes `text` to `path` under `root`, creating folders as needed.
pub fn write(root: &Path, path: &str, text: &[u8]) -> TestResult {
    let native = root.join(path);
    if let Some(folder) = native.parent() {
        fs::create_dir_all(folder)?;
    }
    fs::write(native, text)?;
    Ok(())
}

/// Keeps the file at `path` from being opened by this process while the returned value lives: on
/// Unix by removing every permission, on Windows by holding it open with no sharing.
pub fn lock_out(path: &Path) -> std::io::Result<Option<File>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o000))?;
        Ok(None)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        File::options()
            .read(true)
            .share_mode(0)
            .open(path)
            .map(Some)
    }
}
