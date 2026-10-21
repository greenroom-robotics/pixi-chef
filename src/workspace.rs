use std::path::{Path, PathBuf};

use rattler_conda_types::Subdir;
use rattler_lock::{
    CondaPackageData, LockFile, LockedPackage, ParseCondaLockError, PypiPackageData, UrlOrPath,
};

pub const MANIFEST_FILE: &str = "pixi.toml";
pub const LOCK_FILE: &str = "pixi.lock";

#[derive(Debug, thiserror::Error)]
#[error("reading lock file {path}")]
pub struct LockError {
    path: PathBuf,
    #[source]
    source: Box<ParseCondaLockError>,
}

pub fn read_lock(path: &Path) -> Result<LockFile, LockError> {
    LockFile::from_path(path).map_err(|source| LockError {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// A lock entry built from a local directory, named as the lock records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePackage<'lock> {
    pub name: String,
    pub location: &'lock UrlOrPath,
}

impl<'lock> SourcePackage<'lock> {
    pub fn of(package: &'lock LockedPackage) -> Option<Self> {
        match package {
            LockedPackage::Conda(CondaPackageData::Source(source)) => Some(Self {
                name: source.name().as_source().to_owned(),
                location: &source.location,
            }),
            LockedPackage::Pypi(PypiPackageData::Source(source)) => Some(Self {
                name: source.name.to_string(),
                location: &source.location,
            }),
            LockedPackage::Conda(CondaPackageData::Binary(_))
            | LockedPackage::Pypi(PypiPackageData::Distribution(_)) => None,
        }
    }
}

pub fn location(package: &LockedPackage) -> &UrlOrPath {
    match package {
        LockedPackage::Conda(package) => package.location(),
        LockedPackage::Pypi(package) => package.location(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    #[error("environment {0} is not in the lock file")]
    UnknownEnvironment(String),
    #[error("environment {0} has no packages locked for this platform")]
    NotLockedForPlatform(String),
    #[error("this platform has no conda subdir")]
    UnsupportedPlatform,
}

pub fn current_subdir() -> Result<Subdir, ScopeError> {
    Subdir::current().ok_or(ScopeError::UnsupportedPlatform)
}

/// Every package one environment locks for a subdir.
pub fn scoped_packages<'lock>(
    lock: &'lock LockFile,
    environment: &str,
    subdir: Subdir,
) -> Result<Vec<&'lock LockedPackage>, ScopeError> {
    let locked = lock
        .environment(environment)
        .ok_or_else(|| ScopeError::UnknownEnvironment(environment.to_owned()))?;
    let mut platforms = locked
        .packages_by_platform()
        .filter(|(platform, _)| platform.subdir() == subdir)
        .peekable();
    if platforms.peek().is_none() {
        return Err(ScopeError::NotLockedForPlatform(environment.to_owned()));
    }
    Ok(platforms.flat_map(|(_, packages)| packages).collect())
}
