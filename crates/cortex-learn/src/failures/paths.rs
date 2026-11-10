//! Which frames are the project's: the rules that drop the interpreter's, the toolchains' and the
//! installed packages' paths. A path here has `/` separators and is as the output printed it.

/// Whether `path` names a file with the extension `extension` (without its dot), in any case:
/// `Ops.GO` is a Go file as `ops.go` is.
pub(super) fn has_extension(path: &str, extension: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, found)| found.eq_ignore_ascii_case(extension))
}

/// Parts of a path that mark a frame as an installed package's or a runtime's own: Python and
/// Node packages, the interpreter's frozen modules, the toolchain's source root, and the folders
/// cargo keeps downloaded crates in.
const DROPPED_PATH_PARTS: &[&str] = &[
    "site-packages",
    "dist-packages",
    "node_modules",
    "<frozen",
    "/rustc/",
    ".cargo/registry/",
    ".cargo/git/",
];

/// The Rust standard library's folders. They name the standard library only at the start of a
/// path (`library/core/src/option.rs`); inside `/rustc/<hash>/` the `/rustc/` part drops them, and
/// anywhere else (a project's own `pkg/library/core/x.py`) they are the project's.
const RUST_LIBRARY_PREFIXES: &[&str] = &["library/core/", "library/std/", "library/alloc/"];

/// Whether a frame in `path` may be shown: not empty, not a placeholder such as `<string>`, not
/// Node's `node:` or `internal/` modules, and not one of the dropped paths.
pub(super) fn is_kept_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('<')
        && !path.starts_with("node:")
        && !path.starts_with("internal/")
        && !DROPPED_PATH_PARTS.iter().any(|part| path.contains(part))
        && !RUST_LIBRARY_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
        && !is_python_library(path)
}

/// Whether `path` is a file of a Python interpreter's own library (`json/__init__.py`,
/// `importlib/__init__.py`), by where the interpreter keeps it:
///
/// - a `/lib/python2.` or `/lib/python3.` folder, as on Linux, macOS, pyenv and Homebrew
///   (`/usr/lib/python3.12/importlib/__init__.py`);
/// - a `/Lib/` folder (the case as printed) whose parent folder is an interpreter's install, one
///   whose name starts with `python` in any case or holds `conda`
///   (`C:/Users/x/AppData/Local/Programs/Python/Python312/Lib/json/__init__.py`).
///
/// A project's own `Lib/` folder, or `lib/python` without a version, is the project's.
fn is_python_library(path: &str) -> bool {
    if path.contains("/lib/python2.") || path.contains("/lib/python3.") {
        return true;
    }
    path.match_indices("/Lib/").any(|(at, _)| {
        let parent = path.get(..at).and_then(|head| head.rsplit('/').next());
        parent.is_some_and(is_interpreter_folder)
    })
}

/// Whether a folder name is a Python install's: `Python312`, `python3`, `miniconda3`.
fn is_interpreter_folder(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("python") || lower.contains("conda")
}
