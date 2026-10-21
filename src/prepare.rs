use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use rattler_lock::{
    CondaPackageData, LockFile, LockedPackage, ParseCondaLockError, PypiPackageData, UrlOrPath,
};

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

const MANIFEST_FILE: &str = "pixi.toml";
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
    #[error("building include globs: {0}")]
    Globs(#[from] globset::Error),
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

#[derive(Debug, Clone, Default)]
pub struct PrepareOptions {
    pub into: Option<PathBuf>,
    pub include: Vec<Glob>,
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

/// Maps each recipe path to the repo path it is copied from.
struct Recipe {
    files: BTreeMap<RepoPath, RepoPath>,
}

impl Recipe {
    fn gather(
        root: &Path,
        manifest: &RepoPath,
        into: Option<&RepoPath>,
        include: &GlobSet,
    ) -> Result<Self, PrepareError> {
        if !root.join(manifest.as_path()).is_file() {
            return Err(PrepareError::MissingManifest(
                manifest.as_path().to_path_buf(),
            ));
        }
        let manifest_dir = manifest.parent();
        let lock_path = root.join(manifest_dir.as_path()).join(LOCK_FILE);
        let lock = LockFile::from_path(&lock_path).map_err(|source| PrepareError::Lock {
            path: lock_path.clone(),
            source: Box::new(source),
        })?;

        let workspace_dir = into.cloned().unwrap_or(manifest_dir);
        let mut files = BTreeMap::new();
        files.insert(
            workspace_dir.join(Path::new(MANIFEST_FILE), root)?,
            manifest.clone(),
        );
        let recipe_lock = workspace_dir.join(Path::new(LOCK_FILE), root)?;
        files.insert(
            recipe_lock,
            manifest.parent().join(Path::new(LOCK_FILE), root)?,
        );

        for dir in source_dirs(&lock, &workspace_dir, root)? {
            if !root.join(dir.as_path()).is_dir() {
                return Err(PrepareError::MissingSourceDir(dir.as_path().to_path_buf()));
            }
            for relative in metadata_files(&root.join(dir.as_path()), include)? {
                let file = dir.join(&relative, root)?;
                files.insert(file.clone(), file);
            }
        }
        Ok(Self { files })
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

fn metadata_files(dir: &Path, include: &GlobSet) -> Result<Vec<PathBuf>, PrepareError> {
    let mut found = Vec::new();
    for name in METADATA_FILES {
        if dir.join(name).is_file() {
            found.push(PathBuf::from(name));
        }
    }
    if !include.is_empty() {
        let mut all = Vec::new();
        list_files(dir, Path::new(""), &mut all)?;
        found.extend(all.into_iter().filter(|path| include.is_match(path)));
    }
    Ok(found)
}

fn list_files(base: &Path, relative: &Path, out: &mut Vec<PathBuf>) -> Result<(), PrepareError> {
    let dir = base.join(relative);
    for entry in fs::read_dir(&dir).map_err(PrepareError::io(&dir))? {
        let entry = entry.map_err(PrepareError::io(&dir))?;
        let path = relative.join(entry.file_name());
        if entry.path().is_dir() {
            list_files(base, &path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn source_dirs(
    lock: &LockFile,
    lock_dir: &RepoPath,
    root: &Path,
) -> Result<BTreeSet<RepoPath>, PrepareError> {
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
    let into = options
        .into
        .as_deref()
        .map(|dir| RepoPath::new(dir, root))
        .transpose()?;
    let mut include = GlobSetBuilder::new();
    for glob in &options.include {
        include.add(glob.clone());
    }
    let recipe = Recipe::gather(root, &manifest, into.as_ref(), &include.build()?)?;
    recipe.write(root, &out)
}
