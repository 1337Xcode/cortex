//! What a file's path alone says about it: whether it may hold secrets, and whether a tool or a
//! third party wrote it.
//!
//! Each rule is one line in a table below. Adding a rule means adding a line.

use cortex_core::{FileFlags, RepoPath};

/// A test on a file or folder name.
#[derive(Debug, Clone, Copy)]
enum NamePattern {
    /// The whole name.
    Exact(&'static str),
    /// The start of the name.
    Prefix(&'static str),
    /// The end of the name.
    Suffix(&'static str),
    /// Anywhere in the name.
    Contains(&'static str),
}

impl NamePattern {
    /// Whether `name` matches.
    fn matches(self, name: &str) -> bool {
        match self {
            Self::Exact(text) => name == text,
            Self::Prefix(text) => name.starts_with(text),
            Self::Suffix(text) => name.ends_with(text),
            Self::Contains(text) => name.contains(text),
        }
    }
}

/// Names that may hold secrets, matched without regard to case against the file's name and every
/// folder above it. Such a file is recorded by path and never opened.
const SECRET_NAMES: &[NamePattern] = &[
    NamePattern::Prefix(".env"),
    NamePattern::Suffix(".pem"),
    NamePattern::Suffix(".key"),
    NamePattern::Prefix("id_rsa"),
    NamePattern::Contains("credentials"),
    NamePattern::Suffix(".p12"),
];

/// Folders whose contents a third party wrote, at any depth.
const VENDORED_FOLDERS: &[&str] = &["node_modules", "vendor"];

/// Folders build tools write their output to, at any depth.
const GENERATED_FOLDERS: &[&str] = &["dist", "build", "target", ".next", "out", "coverage"];

/// Names of files a tool writes: minified scripts, source maps and package manager lockfiles.
const GENERATED_NAMES: &[NamePattern] = &[
    NamePattern::Suffix(".min.js"),
    NamePattern::Suffix(".map"),
    NamePattern::Exact("package-lock.json"),
    NamePattern::Exact("npm-shrinkwrap.json"),
    NamePattern::Exact("yarn.lock"),
    NamePattern::Exact("pnpm-lock.yaml"),
    NamePattern::Exact("bun.lock"),
    NamePattern::Exact("bun.lockb"),
    NamePattern::Exact("Cargo.lock"),
    NamePattern::Exact("Gemfile.lock"),
    NamePattern::Exact("composer.lock"),
    NamePattern::Exact("poetry.lock"),
    NamePattern::Exact("Pipfile.lock"),
    NamePattern::Exact("uv.lock"),
    NamePattern::Exact("go.sum"),
    NamePattern::Exact("Podfile.lock"),
    NamePattern::Exact("Package.resolved"),
    NamePattern::Exact("pubspec.lock"),
    NamePattern::Exact("mix.lock"),
    NamePattern::Exact("flake.lock"),
    NamePattern::Exact("gradle.lockfile"),
    NamePattern::Exact("packages.lock.json"),
];

/// Whether the file at `path` may hold secrets, by its name or the name of a folder above it.
#[must_use]
pub fn is_secret(path: &RepoPath) -> bool {
    path.as_str().split('/').any(|part| {
        let part = part.to_lowercase();
        SECRET_NAMES.iter().any(|pattern| pattern.matches(&part))
    })
}

/// The generated and vendored flags `path` earns by its folders and name.
#[must_use]
pub fn flags_for(path: &RepoPath) -> FileFlags {
    let mut flags = FileFlags::empty();
    let mut folders = path.as_str().split('/');
    // The last part is the file's own name; only the parts before it are folders.
    folders.next_back();
    for folder in folders {
        if VENDORED_FOLDERS.contains(&folder) {
            flags = flags.with(FileFlags::VENDORED);
        }
        if GENERATED_FOLDERS.contains(&folder) {
            flags = flags.with(FileFlags::GENERATED);
        }
    }
    let name = path.file_name();
    if GENERATED_NAMES.iter().any(|pattern| pattern.matches(name)) {
        flags = flags.with(FileFlags::GENERATED);
    }
    flags
}

#[cfg(test)]
mod tests {
    use cortex_core::PathError;

    use super::*;

    /// Secret names match whatever their case, in the file's name or a folder's; names that only
    /// resemble them do not.
    #[test]
    fn secret_names() -> Result<(), PathError> {
        for name in [
            ".env",
            "config/.env.production",
            ".envrc",
            "certs/server.pem",
            "deploy/Server.KEY",
            "id_rsa",
            "keys/id_rsa.pub",
            "aws_credentials",
            "config/Credentials.json",
            "store.p12",
            "config/credentials/aws.json",
            ".env.d/app.conf",
            "keys/prod.key/readme.md",
        ] {
            assert!(is_secret(&RepoPath::new(name)?), "{name}");
        }
        for name in [
            "environment.ts",
            "src/keys.rs",
            "keyboard.key.ts",
            "docs/pem.md",
            "credential.rs",
            "dev.env/readme.md",
        ] {
            assert!(!is_secret(&RepoPath::new(name)?), "{name}");
        }
        Ok(())
    }

    /// Folders flag everything under them at any depth; a file named like a folder does not.
    #[test]
    fn folder_rules() -> Result<(), PathError> {
        let vendored = FileFlags::empty().with(FileFlags::VENDORED);
        let generated = FileFlags::empty().with(FileFlags::GENERATED);
        let cases = [
            ("node_modules/react/index.js", vendored),
            ("web/vendor/lib/a.go", vendored),
            ("packages/app/dist/main.js", generated),
            ("build/out.o", generated),
            ("target/debug/cortex.d", generated),
            (".next/server/page.js", generated),
            ("coverage/lcov.info", generated),
            (
                "node_modules/pkg/dist/index.js",
                vendored.with(FileFlags::GENERATED),
            ),
            ("src/build.rs", FileFlags::empty()),
            ("src/vendor", FileFlags::empty()),
            ("src/main.rs", FileFlags::empty()),
        ];
        for (name, expected) in cases {
            assert_eq!(flags_for(&RepoPath::new(name)?), expected, "{name}");
        }
        Ok(())
    }

    /// Minified scripts, source maps and lockfiles are generated wherever they are.
    #[test]
    fn generated_names() -> Result<(), PathError> {
        let generated = FileFlags::empty().with(FileFlags::GENERATED);
        for name in [
            "static/app.min.js",
            "static/app.js.map",
            "Cargo.lock",
            "web/package-lock.json",
            "go.sum",
            "yarn.lock",
        ] {
            assert_eq!(flags_for(&RepoPath::new(name)?), generated, "{name}");
        }
        for name in ["static/app.js", "src/map.rs", "Cargo.toml", "lock.go"] {
            assert_eq!(
                flags_for(&RepoPath::new(name)?),
                FileFlags::empty(),
                "{name}"
            );
        }
        Ok(())
    }
}
