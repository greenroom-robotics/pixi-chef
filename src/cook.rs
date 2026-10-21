use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use rattler_conda_types::Subdir;
use rattler_lock::{
    CondaPackageData, LockFile, LockedPackage, ParseCondaLockError, Platform, PypiPackageData,
};

const LOCK_FILE: &str = "pixi.lock";
const MANIFEST_FILE: &str = "pixi.toml";

#[derive(Debug, thiserror::Error)]
pub enum CookError {
    #[error("reading lock file {path}")]
    Lock {
        path: PathBuf,
        source: Box<ParseCondaLockError>,
    },
    #[error("environment {0} is not in the lock file")]
    UnknownEnvironment(String),
    #[error("environment {0} has no packages locked for this platform")]
    NotLockedForPlatform(String),
    #[error("this platform has no conda subdir")]
    UnsupportedPlatform,
    #[error("running pixi")]
    Spawn(#[source] std::io::Error),
}

/// Names of the source packages of one environment, spelled as the lock records them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipList(BTreeSet<String>);

impl SkipList {
    pub fn new(lock: &LockFile, name: &str, subdir: Subdir) -> Result<Self, CookError> {
        let environment = lock
            .environment(name)
            .ok_or_else(|| CookError::UnknownEnvironment(name.to_owned()))?;
        let mut platforms = environment
            .packages_by_platform()
            .filter(|(platform, _)| platform.subdir() == subdir)
            .peekable();
        if platforms.peek().is_none() {
            return Err(CookError::NotLockedForPlatform(name.to_owned()));
        }
        Ok(Self(source_names(platforms)))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

fn source_names<'lock, P, I>(platforms: P) -> BTreeSet<String>
where
    P: Iterator<Item = (Platform<'lock>, I)>,
    I: Iterator<Item = &'lock LockedPackage>,
{
    platforms
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

pub fn install_command(workspace: &Path, environment: &str, skip: &SkipList) -> Command {
    let mut command = Command::new("pixi");
    command
        .arg("install")
        .arg("--manifest-path")
        .arg(workspace.join(MANIFEST_FILE))
        .args(["--locked", "-e", environment]);
    command.current_dir(workspace);
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
    let subdir = Subdir::current().ok_or(CookError::UnsupportedPlatform)?;
    let skip = SkipList::new(&lock, environment, subdir)?;
    install_command(workspace, environment, &skip)
        .status()
        .map_err(CookError::Spawn)
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Termination {
    Code(i32),
    Signal(i32),
}

impl From<ExitStatus> for Termination {
    fn from(status: ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt;
        status
            .signal()
            .map_or_else(|| Self::Code(status.code().unwrap_or(1)), Self::Signal)
    }
}

impl Termination {
    pub fn exit_byte(self) -> u8 {
        match self {
            Self::Code(code) => u8::try_from(code).unwrap_or(1),
            Self::Signal(signal) => u8::try_from(signal.saturating_add(128)).unwrap_or(1),
        }
    }
}
