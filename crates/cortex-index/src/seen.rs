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

/// Whether `path` holds rules about which files the walk finds.
pub(crate) fn is_rules_file(path: &RepoPath) -> bool {
    RULES_FILES.contains(&path.file_name()) || path.as_str().starts_with(".git/info/")
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
    /// The repository-wide rules.
    repository: RepositoryRules,
    /// The rules of each folder looked at, by path.
    folders: BTreeMap<RepoPath, Rc<FolderRules>>,
}

impl PathRules {
    /// The rules of the repository `walk` describes.
    pub(crate) fn new(walk: WalkRules) -> Result<Self, DiscoverError> {
        let repository = walk.repository()?;
        let mut roots = vec![walk.root.clone()];
        if let Ok(canonical) = walk.root.canonicalize() {
            roots.push(canonical);
        }
        Ok(Self {
            walk,
            roots,
            repository,
            folders: BTreeMap::new(),
        })
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
        // The walk enters folders only, never a link to one.
        let mut above = path.parent();
        while let Some(at) = above.filter(|at| *at != RepoPath::root()) {
            let native = at.to_native(&self.walk.root);
            if !fs::symlink_metadata(&native).is_ok_and(|metadata| metadata.is_dir()) {
                return PathState::Absent;
            }
            above = at.parent();
        }
        let native = path.to_native(&self.walk.root);
        let metadata = match fs::symlink_metadata(&native) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return PathState::Absent,
            Err(_) => return PathState::Unreadable,
        };
        if !metadata.is_file() {
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
        let is_listed = |name: &str| native.join(name).is_file();
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
