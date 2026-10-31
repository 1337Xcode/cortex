//! The walk's rules applied one path at a time: whether the walk would find a path, and the file
//! it would find there, described as the walk describes it. The rules of each folder are read once
//! and kept until a file of rules changes. The watcher sorts its events by them, and an index of
//! chosen paths finds its files by them.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cortex_core::RepoPath;

use crate::discover::{
    self, DiscoverError, DiscoverOptions, DiscoveredFile, MAX_FOLDER_DEPTH, PRUNED_NAMES,
};
use crate::rules::{FolderRules, Reading, RepositoryRules};

/// The names of the files that hold rules about which files the walk finds.
const RULES_FILES: [&str; 3] = [".gitignore", ".cortexignore", ".gitattributes"];

/// Where the repository's own rules are kept, outside any folder the walk enters.
const REPOSITORY_RULES: &str = ".git/info/";

/// Whether `path` holds rules about which files the walk finds.
fn is_rules_file(path: &RepoPath) -> bool {
    RULES_FILES.contains(&path.file_name()) || path.as_str().starts_with(REPOSITORY_RULES)
}

/// Whether the file at `path` is reached from `root`, whose canonical form is `canonical_root`, as
/// the walk reaches files: through folders, never links, and under the spelling it has on disk. A
/// folder on the path replaced by a link since the walk could lead outside the repository, and on a
/// file system that ignores case, the old spelling of a file renamed in case alone still opens.
pub(crate) fn is_reached_directly(root: &Path, canonical_root: &Path, path: &RepoPath) -> bool {
    path.to_native(root)
        .canonicalize()
        .is_ok_and(|canonical| canonical == path.to_native(canonical_root))
}

/// What the walk would find at a path.
pub(crate) enum PathState {
    /// A file, as the walk would describe it.
    File(DiscoveredFile),
    /// A file whose time or first bytes cannot be read now: the walk keeps such a file as stored.
    Unreadable,
    /// Nothing the walk would find: no file, a path the walk does not look at, or something other
    /// than a regular file (a link is not followed).
    Absent,
}

/// What the walk's rules are read from.
pub(crate) struct WalkRules {
    /// The repository's root.
    root: PathBuf,
    /// `index.extra_ignores`.
    extra_ignores: Vec<String>,
    /// The user's global gitignore, if any.
    global_gitignore: Option<PathBuf>,
    /// The largest rules file read.
    max_file_bytes: u64,
}

impl WalkRules {
    /// The rules of the repository at `root` that `options` add to.
    pub(crate) fn of(root: &Path, options: &DiscoverOptions<'_>) -> Self {
        Self {
            root: root.to_owned(),
            extra_ignores: options.extra_ignores.to_vec(),
            global_gitignore: options.global_gitignore.map(Path::to_owned),
            max_file_bytes: options.max_file_bytes,
        }
    }

    /// The repository-wide rules, read now.
    pub(crate) fn repository(&self) -> Result<RepositoryRules, DiscoverError> {
        let mut problems = Vec::new();
        let mut reading = Reading {
            max_bytes: self.max_file_bytes,
            problems: &mut problems,
        };
        RepositoryRules::load(
            &self.root,
            &self.extra_ignores,
            self.global_gitignore.as_deref(),
            &mut reading,
        )
    }
}

/// The walk's rules for one path at a time, each folder's read once.
pub(crate) struct PathRules {
    /// Where the rules are read from.
    walk: WalkRules,
    /// The repository's root as written and, where it differs, as the file system spells it.
    roots: Vec<PathBuf>,
    /// The root as the file system spells it, when it can be read.
    canonical_root: Option<PathBuf>,
    /// The repository-wide rules.
    repository: RepositoryRules,
    /// The rules of each folder looked at, by path.
    folders: BTreeMap<RepoPath, Rc<FolderRules>>,
}

impl PathRules {
    /// The rules of the repository `walk` describes.
    pub(crate) fn new(walk: WalkRules) -> Result<Self, DiscoverError> {
        let repository = walk.repository()?;
        let canonical_root = walk.root.canonicalize().ok();
        let mut roots = vec![walk.root.clone()];
        roots.extend(canonical_root.clone());
        Ok(Self {
            walk,
            roots,
            canonical_root,
            repository,
            folders: BTreeMap::new(),
        })
    }

    /// Whether a change at `path` can change which files the walk finds: `path` holds rules, and
    /// the walk reads them, as the repository's own or from a folder it enters. Rules in a folder
    /// the walk leaves out govern nothing it finds, as installed packages' own ignore files.
    pub(crate) fn changes_rules(&mut self, path: &RepoPath) -> bool {
        if !is_rules_file(path) {
            return false;
        }
        if path.as_str().starts_with(REPOSITORY_RULES) {
            return true;
        }
        path.parent()
            .filter(|folder| *folder != RepoPath::root())
            .is_none_or(|folder| self.is_seen(&folder))
    }

    /// The paths of the files the walk would find in `folder` and below, at most `limit`: `None`
    /// when there are more. Folders are entered as the walk enters them, never through a link, and
    /// what the walk passes over (links, special files, names it cannot store) is passed over.
    pub(crate) fn files_below(&mut self, folder: &RepoPath, limit: usize) -> Option<Vec<RepoPath>> {
        let mut found = Vec::new();
        let mut folders = vec![folder.clone()];
        while let Some(at) = folders.pop() {
            if !self.is_seen(&at) {
                continue;
            }
            // A folder the walk could not list keeps its stored files, as in a full index.
            let Ok(entries) = fs::read_dir(at.to_native(&self.walk.root)) else {
                continue;
            };
            for entry in entries.flatten() {
                let (Ok(kind), Some(name)) = (
                    entry.file_type(),
                    entry.file_name().to_str().map(str::to_owned),
                ) else {
                    continue;
                };
                let Ok(path) = at.join(&name) else {
                    continue;
                };
                if kind.is_dir() {
                    folders.push(path);
                } else if kind.is_file() && self.is_seen(&path) {
                    found.push(path);
                    if found.len() > limit {
                        return None;
                    }
                }
            }
        }
        Some(found)
    }

    /// Reads the rules again, after a file of rules changed.
    pub(crate) fn reload(&mut self) {
        if let Ok(repository) = self.walk.repository() {
            self.repository = repository;
        }
        self.folders.clear();
    }

    /// The repository path of the file system path `native`, if it is inside the repository.
    pub(crate) fn relative(&self, native: &Path) -> Option<RepoPath> {
        let inside = self
            .roots
            .iter()
            .find_map(|root| native.strip_prefix(root).ok())?;
        let parts: Vec<&str> = inside
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect::<Option<_>>()?;
        RepoPath::new(&parts.join("/")).ok()
    }

    /// Whether the walk would look at the path `path`: no folder above it is pruned, ignored or
    /// too deep, and the path itself is not ignored.
    pub(crate) fn is_seen(&mut self, path: &RepoPath) -> bool {
        let is_dir = path.to_native(&self.walk.root).is_dir();
        self.folder_of(path, is_dir).is_some()
    }

    /// What the walk would find at `path`.
    pub(crate) fn state_of(&mut self, path: &RepoPath) -> PathState {
        let Some(folder) = self.folder_of(path, false) else {
            return PathState::Absent;
        };
        let native = path.to_native(&self.walk.root);
        let metadata = match fs::symlink_metadata(&native) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return PathState::Absent,
            Err(_) => return PathState::Unreadable,
        };
        let is_reached = self
            .canonical_root
            .as_deref()
            .is_some_and(|canonical| is_reached_directly(&self.walk.root, canonical, path));
        if !metadata.is_file() || !is_reached {
            return PathState::Absent;
        }
        let verdict = self.repository.verdict(&folder, path);
        match discover::classify(&native, &metadata, path, verdict, self.walk.max_file_bytes) {
            Some(file) => PathState::File(file),
            None => PathState::Unreadable,
        }
    }

    /// The rules of the folder holding `path` when the walk would look at `path`, a folder when
    /// `is_dir`: no folder above it is pruned, ignored or too deep, and the path itself is not
    /// ignored.
    fn folder_of(&mut self, path: &RepoPath, is_dir: bool) -> Option<Rc<FolderRules>> {
        let parts: Vec<&str> = path.as_str().split('/').collect();
        if parts.len() > MAX_FOLDER_DEPTH + 1 {
            return None;
        }
        let mut folder = self.rules_of(&RepoPath::root(), None);
        let mut at = RepoPath::root();
        for (position, part) in parts.iter().enumerate() {
            if PRUNED_NAMES.contains(part) {
                return None;
            }
            let next = at.join(part).ok()?;
            let native = next.to_native(&self.walk.root);
            let is_last = position + 1 == parts.len();
            if self
                .repository
                .is_ignored(&folder, &native, !is_last || is_dir)
            {
                return None;
            }
            if !is_last {
                folder = self.rules_of(&next, Some(folder));
            }
            at = next;
        }
        Some(folder)
    }

    /// The rules of the folder at `path`, inside the folder `parent` describes, read once.
    fn rules_of(&mut self, path: &RepoPath, parent: Option<Rc<FolderRules>>) -> Rc<FolderRules> {
        if let Some(rules) = self.folders.get(path) {
            return Rc::clone(rules);
        }
        let native = path.to_native(&self.walk.root);
        let mut problems = Vec::new();
        let mut reading = Reading {
            max_bytes: self.walk.max_file_bytes,
            problems: &mut problems,
        };
        // As the walk does, rules files are taken from the folder's listing by exact name: on a
        // file system that ignores case, asking for `.gitignore` would find a `.GitIgnore` too.
        let listed: Vec<std::ffi::OsString> = fs::read_dir(&native)
            .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
            .unwrap_or_default();
        let is_listed = |name: &str| listed.iter().any(|entry| entry == name);
        let rules = Rc::new(FolderRules::load(
            &native,
            path.clone(),
            parent,
            &is_listed,
            &mut reading,
        ));
        self.folders.insert(path.clone(), Rc::clone(&rules));
        rules
    }
}
