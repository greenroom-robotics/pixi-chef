use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use rattler_lock::{
    CondaPackageData, Environment, LockFile, LockedPackage, ParseCondaLockError, PypiPackageData,
};

const LOCK_FILE: &str = "pixi.lock";

#[derive(Debug, thiserror::Error)]
pub enum CookError {
    #[error("reading lock file {path}: {source}")]
    Lock {
        path: PathBuf,
        source: Box<ParseCondaLockError>,
    },
    #[error("environment {0} is not in the lock file")]
    UnknownEnvironment(String),
    #[error("running pixi: {0}")]
    Spawn(#[from] std::io::Error),
}

/// Names of the source packages of one environment, spelled as the lock records them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipList(BTreeSet<String>);

impl SkipList {
    pub fn new(lock: &LockFile, environment: &str) -> Result<Self, CookError> {
        let environment = lock
            .environment(environment)
            .ok_or_else(|| CookError::UnknownEnvironment(environment.to_owned()))?;
        Ok(Self(source_names(&environment)))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

fn source_names(environment: &Environment<'_>) -> BTreeSet<String> {
    environment
        .packages_by_platform()
        .flat_map(|(_, packages)| packages.filter_map(source_name).collect::<Vec<_>>())
        .collect()
}

fn source_name(package: &LockedPackage) -> Option<String> {
    match package {
        LockedPackage::Conda(CondaPackageData::Source(source)) => {
            Some(source.name().as_source().to_owned())
        }
        LockedPackage::Pypi(PypiPackageData::Source(source)) => Some(source.name.to_string()),
        LockedPackage::Conda(CondaPackageData::Binary(_))
        | LockedPackage::Pypi(PypiPackageData::Distribution(_)) => None,
    }
}

pub fn install_command(environment: &str, skip: &SkipList) -> Command {
    let mut command = Command::new("pixi");
    command.args(["install", "--locked", "-e", environment]);
    for name in skip.names() {
        command.args(["--skip", name]);
    }
    command
}

pub fn cook(workspace: &Path, environment: &str) -> Result<ExitStatus, CookError> {
    let lock_path = workspace.join(LOCK_FILE);
    let lock = LockFile::from_path(&lock_path).map_err(|source| CookError::Lock {
        path: lock_path,
        source: Box::new(source),
    })?;
    let skip = SkipList::new(&lock, environment)?;
    Ok(install_command(environment, &skip)
        .current_dir(workspace)
        .status()?)
}
