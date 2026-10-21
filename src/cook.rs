use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, ExitStatus};

use rattler_conda_types::Subdir;
use rattler_lock::{LockFile, LockedPackage, Platform};

use crate::workspace::{LOCK_FILE, LockError, MANIFEST_FILE, SourcePackage, read_lock};

#[derive(Debug, thiserror::Error)]
pub enum CookError {
    #[error(transparent)]
    Lock(#[from] LockError),
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
        .flat_map(|(_, packages)| {
            packages
                .filter_map(|package| SourcePackage::of(package).map(|source| source.name))
                .collect::<Vec<_>>()
        })
        .collect()
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
    let lock = read_lock(&workspace.join(LOCK_FILE))?;
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
