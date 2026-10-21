use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, ExitStatus};

use rattler_conda_types::Subdir;
use rattler_lock::LockFile;

use crate::workspace::{
    LOCK_FILE, LockError, MANIFEST_FILE, ScopeError, SourcePackage, current_subdir, read_lock,
    scoped_packages,
};

#[derive(Debug, thiserror::Error)]
pub enum CookError {
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Scope(#[from] ScopeError),
    #[error("running pixi")]
    Spawn(#[source] std::io::Error),
}

/// Names of the source packages of one environment, spelled as the lock records them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipList(BTreeSet<String>);

impl SkipList {
    pub fn new(lock: &LockFile, name: &str, subdir: Subdir) -> Result<Self, CookError> {
        let packages = scoped_packages(lock, name, subdir)?;
        Ok(Self(
            packages
                .into_iter()
                .filter_map(|package| SourcePackage::of(package).map(|source| source.name))
                .collect(),
        ))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
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
    let lock = read_lock(&workspace.join(LOCK_FILE))?;
    let subdir = current_subdir()?;
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
