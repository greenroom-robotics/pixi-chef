use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rattler_lock::{CondaPackageData, LockFile, LockedPackage, PypiPackageData, UrlOrPath};

use crate::path::{RepoPath, RepoPathError};

pub const METADATA_FILES: [&str; 8] = [
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
    #[error("reading lock file {path}: {message}")]
    Lock { path: PathBuf, message: String },
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

/// Repo-relative files that all exist under the repo root.
struct Recipe {
    files: BTreeSet<RepoPath>,
}

impl Recipe {
    fn gather(root: &Path, manifest: &RepoPath) -> Result<Self, PrepareError> {
        if !root.join(manifest.as_path()).is_file() {
            return Err(PrepareError::MissingManifest(
                manifest.as_path().to_path_buf(),
            ));
        }
        let workspace_dir = manifest.parent();
        let lock_path = root.join(workspace_dir.as_path()).join(LOCK_FILE);
        let lock = LockFile::from_path(&lock_path).map_err(|e| PrepareError::Lock {
            path: lock_path.clone(),
            message: e.to_string(),
        })?;

        let mut files = BTreeSet::new();
        files.insert(manifest.clone());
        files.insert(workspace_dir.join(Path::new(LOCK_FILE), root)?);

        for dir in source_dirs(&lock, &workspace_dir, root)? {
            if !root.join(dir.as_path()).is_dir() {
                return Err(PrepareError::MissingSourceDir(dir.as_path().to_path_buf()));
            }
            for name in METADATA_FILES {
                let file = dir.join(Path::new(name), root)?;
                if root.join(file.as_path()).is_file() {
                    files.insert(file);
                }
            }
        }
        Ok(Self { files })
    }

    fn write(&self, root: &Path, out: &EmptyDir) -> Result<(), PrepareError> {
        for file in &self.files {
            let from = root.join(file.as_path());
            let to = out.0.join(file.as_path());
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(PrepareError::io(parent))?;
            }
            fs::copy(&from, &to).map_err(PrepareError::io(&from))?;
        }
        Ok(())
    }
}

fn source_dirs(
    lock: &LockFile,
    lock_dir: &RepoPath,
    root: &Path,
) -> Result<BTreeSet<RepoPath>, PrepareError> {
    let mut dirs = BTreeSet::new();
    for (_, environment) in lock.environments() {
        for (_, packages) in environment.packages_by_platform() {
            for package in packages {
                if let Some(location) = source_location(package)
                    && let UrlOrPath::Path(path) = location
                {
                    dirs.insert(lock_dir.join(Path::new(path.as_str()), root)?);
                }
            }
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

pub fn prepare(root: &Path, manifest: &Path, out: &Path) -> Result<(), PrepareError> {
    let out = EmptyDir::new(out)?;
    let manifest = RepoPath::new(manifest, root)?;
    let recipe = Recipe::gather(root, &manifest)?;
    recipe.write(root, &out)
}
