//! The table of the map's built files that the daemon's build script embeds: each file under the
//! UI's build folder, its path inside that folder with `/` separators, its content type, and the
//! Rust text that includes its bytes in the binary.
//!
//! The build script compiles this file as a module of its own; the daemon compiles it only for
//! its tests. Nothing here runs in the daemon.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

/// How deep the build folder's sub-folders may go: a built page nests two or three levels, so a
/// deeper tree is not one the UI's build made.
const MAX_DEPTH: usize = 16;

/// The content type of each file extension the map serves; a file of any other extension is
/// served as bytes to download, never as something a browser runs.
const CONTENT_TYPES: [(&str, &str); 15] = [
    ("css", "text/css; charset=utf-8"),
    ("gif", "image/gif"),
    ("html", "text/html; charset=utf-8"),
    ("ico", "image/x-icon"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("js", "text/javascript; charset=utf-8"),
    ("json", "application/json"),
    ("map", "application/json"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("png", "image/png"),
    ("svg", "image/svg+xml"),
    ("txt", "text/plain; charset=utf-8"),
    ("wasm", "application/wasm"),
    ("woff2", "font/woff2"),
];

/// The content type of a file of no extension the map knows.
const BYTES: &str = "application/octet-stream";

/// One file to embed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its path inside the build folder, with `/` separators: `assets/index-1a2b.js`.
    pub path: String,
    /// The content type it is served with.
    pub content_type: &'static str,
    /// Where it is on disk, for the build to read.
    pub file: PathBuf,
}

/// The content type for the file at `path`, by its extension in any case.
pub fn content_type(path: &str) -> &'static str {
    let Some((_, extension)) = path.rsplit_once('.') else {
        return BYTES;
    };
    let extension = extension.to_ascii_lowercase();
    CONTENT_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map_or(BYTES, |(_, content_type)| content_type)
}

/// Every file under the build folder `dist`, sorted by path; none when there is no `dist`.
///
/// # Errors
///
/// A `dist` that is a link or a file rather than a folder (a link could point anywhere), and
/// whatever [`list`] refuses inside it.
pub fn read(dist: &Path) -> io::Result<Vec<Entry>> {
    match std::fs::symlink_metadata(dist) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
        Ok(found) if found.file_type().is_dir() => list(dist),
        Ok(_) => Err(io::Error::other(format!(
            "{} is a link or a file, not a folder",
            dist.display()
        ))),
    }
}

/// Every file under the folder `dist`, sorted by path.
///
/// # Errors
///
/// A folder that cannot be read, a name that is not UTF-8 text, a link or anything else neither a
/// file nor a folder (the build makes none, and a link could point anywhere), or a tree deeper
/// than [`MAX_DEPTH`].
fn list(dist: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut folders = vec![(dist.to_path_buf(), String::new(), 0)];
    while let Some((folder, prefix, depth)) = folders.pop() {
        if depth > MAX_DEPTH {
            return Err(io::Error::other(format!(
                "{} is nested deeper than {MAX_DEPTH} folders",
                folder.display()
            )));
        }
        for child in std::fs::read_dir(&folder)? {
            let child = child?;
            let name = child.file_name().into_string().map_err(|_| {
                io::Error::other(format!("a name in {} is not UTF-8", folder.display()))
            })?;
            let path = format!("{prefix}{name}");
            let kind = child.file_type()?;
            if kind.is_dir() {
                folders.push((child.path(), format!("{path}/"), depth + 1));
            } else if kind.is_file() {
                let content_type = content_type(&path);
                entries.push(Entry {
                    path,
                    content_type,
                    file: child.path(),
                });
            } else {
                return Err(io::Error::other(format!(
                    "{} is a link, or neither a file nor a folder",
                    child.path().display()
                )));
            }
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

/// The Rust text of the table: a static slice of the daemon's `Asset`, one per entry in the order
/// given, each including its file's bytes.
///
/// # Errors
///
/// A file path that is not UTF-8 text, which `include_bytes!` cannot name.
pub fn render(entries: &[Entry]) -> io::Result<String> {
    let mut text = String::from(
        "/// The map's built files, sorted by path, written by the build script.\n\
         pub(crate) static ASSETS: &[Asset] = &[\n",
    );
    for entry in entries {
        let file = entry
            .file
            .to_str()
            .ok_or_else(|| io::Error::other(format!("{} is not UTF-8", entry.file.display())))?;
        // Debug formatting writes a string literal with its quotes and backslashes escaped.
        let _ = writeln!(
            text,
            "    Asset {{ path: {:?}, content_type: {:?}, bytes: include_bytes!({:?}) }},",
            entry.path, entry.content_type, file
        );
    }
    text.push_str("];\n");
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A build folder of four files in two levels, made in an order other than the sorted one, is
    /// listed sorted by path with `/` separators, each with the type its extension gives; the
    /// table names each in that order with its bytes included.
    #[test]
    fn files_are_listed_sorted_with_their_types() -> TestResult {
        let dist = tempfile::tempdir()?;
        std::fs::create_dir(dist.path().join("assets"))?;
        std::fs::write(dist.path().join("index.html"), "<p>map</p>")?;
        std::fs::write(dist.path().join("assets").join("z.JS"), "1")?;
        std::fs::write(dist.path().join("assets").join("a.css"), "p{}")?;
        std::fs::write(dist.path().join("LICENSE"), "text")?;
        let entries = read(dist.path())?;
        let listed: Vec<(&str, &str)> = entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.content_type))
            .collect();
        assert_eq!(
            listed,
            [
                ("LICENSE", "application/octet-stream"),
                ("assets/a.css", "text/css; charset=utf-8"),
                ("assets/z.JS", "text/javascript; charset=utf-8"),
                ("index.html", "text/html; charset=utf-8"),
            ]
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.file.clone())
                .collect::<Vec<_>>(),
            [
                dist.path().join("LICENSE"),
                dist.path().join("assets").join("a.css"),
                dist.path().join("assets").join("z.JS"),
                dist.path().join("index.html"),
            ]
        );
        let text = render(&entries)?;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 7);
        for (line, path) in
            lines[2..6]
                .iter()
                .zip(["LICENSE", "assets/a.css", "assets/z.JS", "index.html"])
        {
            assert!(
                line.starts_with(&format!("    Asset {{ path: \"{path}\", ")),
                "{line}"
            );
            assert!(line.contains("include_bytes!(\""), "{line}");
        }
        assert_eq!(lines[6], "];");
        Ok(())
    }

    /// An empty build folder, like none, gives an empty table.
    #[test]
    fn no_files_give_an_empty_table() -> TestResult {
        let dist = tempfile::tempdir()?;
        assert_eq!(read(&dist.path().join("missing"))?, []);
        let entries = read(dist.path())?;
        assert_eq!(entries, []);
        assert_eq!(
            render(&entries)?,
            "/// The map's built files, sorted by path, written by the build script.\n\
             pub(crate) static ASSETS: &[Asset] = &[\n];\n"
        );
        Ok(())
    }

    /// The message of `read`'s refusal of `dist`, failing the test when it is read.
    fn refusal(dist: &Path) -> Result<String, Box<dyn std::error::Error>> {
        match read(dist) {
            Ok(entries) => Err(format!("{} entries read", entries.len()).into()),
            Err(error) => Ok(error.to_string()),
        }
    }

    /// A tree 16 folders deep is read; one a folder deeper is refused.
    #[test]
    fn a_tree_too_deep_is_refused() -> TestResult {
        let dist = tempfile::tempdir()?;
        let deepest = (0..MAX_DEPTH).fold(dist.path().to_path_buf(), |path, _| path.join("a"));
        std::fs::create_dir_all(&deepest)?;
        std::fs::write(deepest.join("f.js"), "1")?;
        assert_eq!(read(dist.path())?.len(), 1);
        std::fs::create_dir(deepest.join("a"))?;
        assert!(refusal(dist.path())?.contains("nested deeper than 16 folders"));
        Ok(())
    }

    /// A file whose name is not UTF-8 is refused: on Windows a name holding half of a UTF-16
    /// pair, which the file system takes; on Linux a name holding a byte UTF-8 never starts
    /// with. (macOS refuses such names itself.)
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn a_name_that_is_not_utf_8_is_refused() -> TestResult {
        let dist = tempfile::tempdir()?;
        #[cfg(windows)]
        let name = {
            use std::os::windows::ffi::OsStringExt;
            std::ffi::OsString::from_wide(&[0xd800, u16::from(b'x')])
        };
        #[cfg(target_os = "linux")]
        let name = {
            use std::os::unix::ffi::OsStrExt;
            std::ffi::OsStr::from_bytes(b"\xffx").to_os_string()
        };
        std::fs::write(dist.path().join(name), "1")?;
        assert!(refusal(dist.path())?.contains("is not UTF-8"));
        Ok(())
    }

    /// Makes `link` a link to the folder `target`: a symbolic link on Unix; on Windows a junction,
    /// which needs no privilege, made by the system's own `mklink` and read by Rust as a link.
    fn link_folder(target: &Path, link: &Path) -> TestResult {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link)?;
        #[cfg(windows)]
        {
            let system = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
            // Backslashes only: the command shell reads a `/` in its own path as an option.
            let shell = PathBuf::from(system).join("System32").join("cmd.exe");
            let made = std::process::Command::new(shell)
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()?;
            if !made.status.success() {
                return Err("mklink made no junction".into());
            }
        }
        Ok(())
    }

    /// A link inside the build folder is refused, and so is a build folder that is itself a link,
    /// though each leads to a folder holding a file.
    #[test]
    fn links_are_refused_inside_and_as_the_folder() -> TestResult {
        let scratch = tempfile::tempdir()?;
        let elsewhere = scratch.path().join("elsewhere");
        std::fs::create_dir(&elsewhere)?;
        std::fs::write(elsewhere.join("secret.txt"), "1")?;
        let dist = scratch.path().join("dist");
        std::fs::create_dir(&dist)?;
        std::fs::write(dist.join("index.html"), "<p>map</p>")?;
        assert_eq!(read(&dist)?.len(), 1);
        link_folder(&elsewhere, &dist.join("linked"))?;
        assert!(refusal(&dist)?.contains("is a link"));
        let linked_dist = scratch.path().join("linked-dist");
        link_folder(&elsewhere, &linked_dist)?;
        assert!(refusal(&linked_dist)?.contains("is a link or a file, not a folder"));
        Ok(())
    }
}
