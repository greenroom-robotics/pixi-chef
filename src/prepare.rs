use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use rattler_lock::{
    CondaPackageData, LockFile, LockedPackage, ParseCondaLockError, PypiPackageData, UrlOrPath,
};

use crate::include::IncludeGlob;
use crate::path::{RepoPath, RepoPathError};

const METADATA_FILES: [&str; 8] = [
    "pixi.toml",
    "pixi.lock",
    "package.xml",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "CMakeLists.txt",
    "README.md",
];

const LOCK_FILE: &str = "pixi.lock";

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("output directory {0} exists and is not empty")]
    OutNotEmpty(PathBuf),
    #[error("source package directory {0} does not exist")]
    MissingSourceDir(PathBuf),
    #[error("manifest {0} does not exist")]
    MissingManifest(PathBuf),
    #[error("reading lock file {path}: {source}")]
    Lock {
        path: PathBuf,
        source: Box<ParseCondaLockError>,
    },
    #[error(
        "--into {into}: source package paths in the lock do not resolve to the same directories as from the manifest's directory"
    )]
    IntoMismatch { into: PathBuf },
    #[error("recipe path {0} would be written from two different files")]
    Collision(PathBuf),
    #[error("--include {0} matched no file in any source package")]
    UnmatchedInclude(String),
    #[error("walking {path}: {source}")]
    Walk {
        path: PathBuf,
        source: ignore::Error,
    },
    #[error(transparent)]
    Path(#[from] RepoPathError),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

impl PrepareError {
    fn io(path: &Path) -> impl FnOnce(io::Error) -> Self {
        let path = path.to_path_buf();
        move |source| Self::Io { path, source }
    }
}

/// A directory inside the repo root to place the workspace manifest and lock at.
#[must_use]
#[derive(Debug, Clone)]
pub struct IntoDir(RepoPath);

impl IntoDir {
    pub fn new(path: &Path, root: &Path) -> Result<Self, RepoPathError> {
        RepoPath::new(path, root).map(Self)
    }
}

#[derive(Debug, Clone, Default)]
pub struct PrepareOptions {
    pub into: Option<IntoDir>,
    pub include: Vec<IncludeGlob>,
}

/// A directory that does not exist yet or exists and holds nothing.
struct EmptyDir(PathBuf);

impl EmptyDir {
    fn new(path: &Path) -> Result<Self, PrepareError> {
        match fs::read_dir(path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(PrepareError::OutNotEmpty(path.to_path_buf()));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(PrepareError::io(path)(e)),
        }
        Ok(Self(path.to_path_buf()))
    }
}

#[derive(Default)]
struct Files(BTreeMap<RepoPath, RepoPath>);

impl Files {
    fn add(&mut self, recipe_path: RepoPath, source: RepoPath) -> Result<(), PrepareError> {
        match self.0.entry(recipe_path) {
            Entry::Vacant(slot) => {
                slot.insert(source);
            }
            Entry::Occupied(slot) if *slot.get() != source => {
                return Err(PrepareError::Collision(slot.key().as_path().to_path_buf()));
            }
            Entry::Occupied(_) => {}
        }
        Ok(())
    }
}

fn manifest_file_name(manifest: &RepoPath) -> &OsStr {
    manifest.as_path().file_name().unwrap_or_default()
}

/// Maps each recipe path to the repo path it is copied from.
struct Recipe {
    files: BTreeMap<RepoPath, RepoPath>,
}

impl Recipe {
    fn gather(
        root: &Path,
        manifest: &RepoPath,
        options: &PrepareOptions,
    ) -> Result<Self, PrepareError> {
        if !root.join(manifest.as_path()).is_file() {
            return Err(PrepareError::MissingManifest(
                manifest.as_path().to_path_buf(),
            ));
        }
        let manifest_dir = manifest.parent();
        let source_lock = manifest_dir.join(Path::new(LOCK_FILE), root)?;
        let lock_path = root.join(source_lock.as_path());
        let lock = LockFile::from_path(&lock_path).map_err(|source| PrepareError::Lock {
            path: lock_path.clone(),
            source: Box::new(source),
        })?;
        let source_dirs = source_dirs(&lock, &manifest_dir, root)?;

        let workspace_dir = match &options.into {
            Some(IntoDir(into)) => {
                let from_into = source_dirs_from(&lock, into, root);
                if from_into.ok().as_ref() != Some(&source_dirs) {
                    return Err(PrepareError::IntoMismatch {
                        into: into.as_path().to_path_buf(),
                    });
                }
                into.clone()
            }
            None => manifest_dir,
        };

        let mut files = Files::default();
        files.add(
            workspace_dir.join(Path::new(manifest_file_name(manifest)), root)?,
            manifest.clone(),
        )?;
        files.add(workspace_dir.join(Path::new(LOCK_FILE), root)?, source_lock)?;

        let mut matched = BTreeSet::new();
        for dir in source_dirs {
            if !root.join(dir.as_path()).is_dir() {
                return Err(PrepareError::MissingSourceDir(dir.as_path().to_path_buf()));
            }
            let found = metadata_files(&root.join(dir.as_path()), &options.include)?;
            matched.extend(found.matched);
            for relative in found.files {
                let file = dir.join(&relative, root)?;
                files.add(file.clone(), file)?;
            }
        }
        if let Some((_, glob)) = options
            .include
            .iter()
            .enumerate()
            .find(|(index, _)| !matched.contains(index))
        {
            return Err(PrepareError::UnmatchedInclude(glob.pattern().to_owned()));
        }
        Ok(Self { files: files.0 })
    }

    fn write(&self, root: &Path, out: &EmptyDir) -> Result<(), PrepareError> {
        for (file, source) in &self.files {
            let from = root.join(source.as_path());
            let to = out.0.join(file.as_path());
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(PrepareError::io(parent))?;
            }
            fs::copy(&from, &to).map_err(PrepareError::io(&from))?;
        }
        Ok(())
    }
}

struct Found {
    files: Vec<PathBuf>,
    matched: BTreeSet<usize>,
}

fn metadata_files(dir: &Path, include: &[IncludeGlob]) -> Result<Found, PrepareError> {
    let mut files: Vec<PathBuf> = METADATA_FILES
        .iter()
        .filter(|name| dir.join(name).is_file())
        .map(PathBuf::from)
        .collect();
    let mut matched = BTreeSet::new();
    if !include.is_empty() {
        let walk = WalkBuilder::new(dir)
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        for entry in walk {
            let entry = entry.map_err(|source| PrepareError::Walk {
                path: dir.to_path_buf(),
                source,
            })?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(dir) else {
                continue;
            };
            for (index, glob) in include.iter().enumerate() {
                if glob.matches(relative) {
                    matched.insert(index);
                    files.push(relative.to_path_buf());
                }
            }
        }
    }
    Ok(Found { files, matched })
}

fn source_dirs(
    lock: &LockFile,
    lock_dir: &RepoPath,
    root: &Path,
) -> Result<BTreeSet<RepoPath>, PrepareError> {
    Ok(source_dirs_from(lock, lock_dir, root)?)
}

fn source_dirs_from(
    lock: &LockFile,
    lock_dir: &RepoPath,
    root: &Path,
) -> Result<BTreeSet<RepoPath>, RepoPathError> {
    let mut dirs = BTreeSet::new();
    for package in lock.packages() {
        if let Some(UrlOrPath::Path(path)) = source_location(package) {
            dirs.insert(lock_dir.join(Path::new(path.as_str()), root)?);
        }
    }
    Ok(dirs)
}

fn source_location(package: &LockedPackage) -> Option<&UrlOrPath> {
    match package {
        LockedPackage::Conda(CondaPackageData::Source(source)) => Some(&source.location),
        LockedPackage::Pypi(PypiPackageData::Source(source)) => Some(&source.location),
        LockedPackage::Conda(CondaPackageData::Binary(_))
        | LockedPackage::Pypi(PypiPackageData::Distribution(_)) => None,
    }
}

pub fn prepare(
    root: &Path,
    manifest: &Path,
    out: &Path,
    options: &PrepareOptions,
) -> Result<(), PrepareError> {
    let out = EmptyDir::new(out)?;
    let manifest = RepoPath::new(manifest, root)?;
    let recipe = Recipe::gather(root, &manifest, options)?;
    recipe.write(root, &out)
}
