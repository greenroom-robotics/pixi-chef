use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use rattler_conda_types::Subdir;
use rattler_lock::{
    CondaPackageData, LockFile, LockedPackage, ParseCondaLockError, PypiPackageData, UrlOrPath,
};

const LOCK_FILE: &str = "pixi.lock";
const EXPORT_PIXI_EXE: &str = "export PIXI_EXE=";
const HOOK_PATH: &str = "etc/profile.d/pixi-env.sh";
const ENTRYPOINT_PATH: &str = "entrypoint.sh";

#[derive(Debug, thiserror::Error)]
pub enum BakeError {
    #[error("export directory {0} exists and is not empty")]
    ExportNotEmpty(PathBuf),
    #[error("source package directory {0} does not exist")]
    MissingSourceDir(PathBuf),
    #[error("environment {0} was not installed")]
    MissingEnvironment(PathBuf),
    #[error("environment {0} has no bin/bash; add `bash` to the workspace dependencies")]
    MissingBash(PathBuf),
    #[error("environment {0} is not in the lock file")]
    UnknownEnvironment(String),
    #[error("environment {0} is not locked for this platform")]
    NotLockedForPlatform(String),
    #[error("this platform has no conda subdir")]
    UnsupportedPlatform,
    #[error("`pixi shell-hook` output for environment {0} never used the host PATH marker")]
    HookIgnoresPath(String),
    #[error("hook sources {0}, which does not exist")]
    MissingSourcedFile(PathBuf),
    #[error("cannot move the export into {path}; is it a mount point? {source}")]
    Publish { path: PathBuf, source: io::Error },
    #[error("{error}; also failed to remove {path}: {cleanup}")]
    Cleanup {
        error: Box<Self>,
        path: PathBuf,
        cleanup: io::Error,
    },
    #[error("`pixi shell-hook` printed nothing for environment {0}")]
    EmptyHook(String),
    #[error("`{command}` failed with {status}")]
    CommandFailed { command: String, status: ExitStatus },
    #[error("running `{command}`: {source}")]
    Spawn { command: String, source: io::Error },
    #[error("`pixi shell-hook` output is not UTF-8")]
    HookNotUtf8,
    #[error("reading lock file {path}: {source}")]
    Lock {
        path: PathBuf,
        source: Box<ParseCondaLockError>,
    },
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

impl BakeError {
    fn io(path: &Path) -> impl FnOnce(io::Error) -> Self {
        let path = path.to_path_buf();
        move |source| Self::Io { path, source }
    }
}

/// An existing absolute path with symlinks resolved.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AbsPath {
    path: PathBuf,
    below_root: PathBuf,
}

impl AbsPath {
    fn new(path: &Path) -> Result<Self, BakeError> {
        let path = path.canonicalize().map_err(BakeError::io(path))?;
        let below_root = path
            .strip_prefix("/")
            .map_err(|_| BakeError::io(&path)(io::ErrorKind::InvalidInput.into()))?
            .to_path_buf();
        Ok(Self { path, below_root })
    }

    fn as_path(&self) -> &Path {
        &self.path
    }

    fn starts_with(&self, other: &Self) -> bool {
        self.path.starts_with(&other.path)
    }
}

const PATH_MARKER: &str = "PIXI_CHEF_HOST_PATH_MARKER_7f3a91c2";

/// A shell hook that sets up the environment on top of the runtime `PATH`.
struct ShellHook(String);

impl ShellHook {
    fn new(env: &str, text: &str) -> Result<Self, BakeError> {
        if text.trim().is_empty() {
            return Err(BakeError::EmptyHook(env.to_string()));
        }
        if !text.contains(PATH_MARKER) {
            return Err(BakeError::HookIgnoresPath(env.to_string()));
        }
        let rewritten: String = text
            .lines()
            .filter(|line| !line.starts_with(EXPORT_PIXI_EXE))
            .flat_map(|line| [line.replace(PATH_MARKER, "$PATH"), "\n".to_string()])
            .collect();
        Ok(Self(rewritten))
    }

    /// Absolute files the hook sources.
    fn sourced_files(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.0
            .lines()
            .filter_map(|line| line.strip_prefix(". "))
            .map(|path| PathBuf::from(path.trim().trim_matches(['"', '\''])))
            .filter(|path| path.is_absolute())
    }
}

/// An export being written into a uniquely named directory next to its final location.
struct Staging {
    temp: tempfile::TempDir,
    dir: AbsPath,
    target: PathBuf,
}

impl Staging {
    fn new(export: &Path) -> Result<Self, BakeError> {
        let target = std::path::absolute(export).map_err(BakeError::io(export))?;
        match fs::read_dir(&target) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(BakeError::ExportNotEmpty(target));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(BakeError::io(&target)(e)),
        }
        let parent = target.parent().unwrap_or_else(|| Path::new("/"));
        fs::create_dir_all(parent).map_err(BakeError::io(parent))?;
        let temp = tempfile::Builder::new()
            .prefix(".pixi-chef-export-")
            .tempdir_in(parent)
            .map_err(BakeError::io(parent))?;
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755))
            .map_err(BakeError::io(temp.path()))?;
        let dir = AbsPath::new(temp.path())?;
        Ok(Self { temp, dir, target })
    }

    /// Where a path of the build machine lands inside the export.
    fn at(&self, path: &AbsPath) -> PathBuf {
        self.dir.as_path().join(&path.below_root)
    }

    fn within(&self, relative: &str) -> PathBuf {
        self.dir.as_path().join(relative)
    }

    fn publish(&self) -> Result<(), BakeError> {
        fs::rename(self.dir.as_path(), &self.target).map_err(|source| BakeError::Publish {
            path: self.target.clone(),
            source,
        })
    }

    fn discard(self, error: BakeError) -> BakeError {
        let path = self.dir.as_path().to_path_buf();
        match self.temp.close() {
            Ok(()) => error,
            Err(e) if e.kind() == io::ErrorKind::NotFound => error,
            Err(cleanup) => BakeError::Cleanup {
                error: Box::new(error),
                path,
                cleanup,
            },
        }
    }
}

/// The pixi executable that `bake` drives.
#[derive(Debug, Clone)]
pub struct Pixi(OsString);

impl Pixi {
    pub fn at(program: impl Into<OsString>) -> Self {
        Self(program.into())
    }

    fn command(&self, workspace: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(&self.0);
        command.args(args).current_dir(workspace);
        command
    }

    fn describe(&self, args: &[&str]) -> String {
        format!("{} {}", self.0.to_string_lossy(), args.join(" "))
    }

    fn run(&self, workspace: &Path, args: &[&str]) -> Result<(), BakeError> {
        let command = self.describe(args);
        let status = self
            .command(workspace, args)
            .status()
            .map_err(|source| BakeError::Spawn {
                command: command.clone(),
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(BakeError::CommandFailed { command, status })
        }
    }

    fn resolved(&self) -> PathBuf {
        let program = Path::new(&self.0);
        if program.components().count() > 1 {
            return program.to_path_buf();
        }
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .map(|dir| dir.join(program))
            .find(|candidate| is_executable(candidate))
            .unwrap_or_else(|| program.to_path_buf())
    }

    fn capture_with_marked_path(
        &self,
        workspace: &Path,
        args: &[&str],
    ) -> Result<Vec<u8>, BakeError> {
        let command = self.describe(args);
        let output = Command::new(self.resolved())
            .args(args)
            .current_dir(workspace)
            .env("PATH", PATH_MARKER)
            .output()
            .map_err(|source| BakeError::Spawn {
                command: command.clone(),
                source,
            })?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(BakeError::CommandFailed {
                command,
                status: output.status,
            })
        }
    }
}

impl Default for Pixi {
    fn default() -> Self {
        Self::at("pixi")
    }
}

fn source_dirs(workspace: &AbsPath, env: &str) -> Result<Vec<AbsPath>, BakeError> {
    let lock_path = workspace.as_path().join(LOCK_FILE);
    let lock = LockFile::from_path(&lock_path).map_err(|source| BakeError::Lock {
        path: lock_path,
        source: Box::new(source),
    })?;
    let subdir = Subdir::current().ok_or(BakeError::UnsupportedPlatform)?;
    let environment = lock
        .environment(env)
        .ok_or_else(|| BakeError::UnknownEnvironment(env.to_string()))?;
    let mut platforms = environment
        .packages_by_platform()
        .filter(|(platform, _)| platform.subdir() == subdir)
        .peekable();
    if platforms.peek().is_none() {
        return Err(BakeError::NotLockedForPlatform(env.to_string()));
    }
    let mut dirs = BTreeSet::new();
    for (_, packages) in platforms {
        for package in packages {
            let location = match package {
                LockedPackage::Conda(CondaPackageData::Source(source)) => Some(&source.location),
                LockedPackage::Pypi(PypiPackageData::Source(source)) => Some(&*source.location),
                LockedPackage::Conda(CondaPackageData::Binary(_))
                | LockedPackage::Pypi(PypiPackageData::Distribution(_)) => None,
            };
            if let Some(UrlOrPath::Path(path)) = location {
                let dir = workspace.as_path().join(path.as_str());
                dirs.insert(AbsPath::new(&dir).map_err(|_| BakeError::MissingSourceDir(dir))?);
            }
        }
    }
    Ok(outermost(dirs))
}

/// Drops every directory that sits inside another one of the set.
fn outermost(dirs: BTreeSet<AbsPath>) -> Vec<AbsPath> {
    let mut kept: Vec<AbsPath> = Vec::new();
    for dir in dirs {
        if !kept.last().is_some_and(|outer| dir.starts_with(outer)) {
            kept.push(dir);
        }
    }
    kept
}

fn prune_pixi_dir(workspace: &Path) -> Result<(), BakeError> {
    let pixi_dir = workspace.join(".pixi");
    for entry in fs::read_dir(&pixi_dir).map_err(BakeError::io(&pixi_dir))? {
        let entry = entry.map_err(BakeError::io(&pixi_dir))?;
        if entry.file_name() == "envs" {
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type().map_err(BakeError::io(&path))?;
        if kind.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        }
        .map_err(BakeError::io(&path))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path, skip: &[&Path]) -> Result<(), BakeError> {
    let metadata = fs::symlink_metadata(from).map_err(BakeError::io(from))?;
    let kind = metadata.file_type();
    if kind.is_symlink() {
        let target = fs::read_link(from).map_err(BakeError::io(from))?;
        symlink(target, to).map_err(BakeError::io(to))
    } else if kind.is_dir() {
        fs::create_dir_all(to).map_err(BakeError::io(to))?;
        for entry in fs::read_dir(from).map_err(BakeError::io(from))? {
            let entry = entry.map_err(BakeError::io(from))?;
            let path = entry.path();
            if skip.contains(&path.as_path()) {
                continue;
            }
            copy_tree(&path, &to.join(entry.file_name()), skip)?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map_err(BakeError::io(from)).map(drop)
    }
}

fn copy_into_export(export: &Staging, source: &AbsPath, skip: &[&Path]) -> Result<(), BakeError> {
    let to = export.at(source);
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(BakeError::io(parent))?;
    }
    copy_tree(source.as_path(), &to, skip)
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn copy_sourced_file(export: &Staging, file: &Path) -> Result<(), BakeError> {
    if !file.is_file() {
        return Err(BakeError::MissingSourcedFile(file.to_path_buf()));
    }
    let to = export
        .dir
        .as_path()
        .join(file.strip_prefix("/").unwrap_or(file));
    if to.exists() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(BakeError::io(parent))?;
    }
    fs::copy(file, &to).map_err(BakeError::io(file)).map(drop)
}

fn write_file(path: &Path, content: &str, mode: u32) -> Result<(), BakeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(BakeError::io(parent))?;
    }
    fs::write(path, content).map_err(BakeError::io(path))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(BakeError::io(path))
}

pub fn bake(pixi: &Pixi, workspace: &Path, env: &str, export: &Path) -> Result<(), BakeError> {
    let workspace = AbsPath::new(workspace)?;
    let staging = Staging::new(export)?;
    match assemble(pixi, &workspace, env, &staging).and_then(|()| staging.publish()) {
        Ok(()) => Ok(()),
        Err(error) => Err(staging.discard(error)),
    }
}

fn assemble(
    pixi: &Pixi,
    workspace: &AbsPath,
    env: &str,
    staging: &Staging,
) -> Result<(), BakeError> {
    let sources = source_dirs(workspace, env)?;
    let work = workspace.as_path();

    pixi.run(work, &["install", "--locked", "-e", env])?;
    let env_dir = AbsPath::new(&work.join(".pixi/envs").join(env))
        .map_err(|_| BakeError::MissingEnvironment(work.join(".pixi/envs").join(env)))?;
    let bash = env_dir.as_path().join("bin/bash");
    if !bash.is_file() {
        return Err(BakeError::MissingBash(env_dir.as_path().to_path_buf()));
    }
    let hook_bytes = pixi.capture_with_marked_path(
        work,
        &[
            "shell-hook",
            "--as-is",
            "--shell",
            "bash",
            "--frozen",
            "-e",
            env,
        ],
    )?;
    let hook_text = String::from_utf8(hook_bytes).map_err(|_| BakeError::HookNotUtf8)?;
    let hook = ShellHook::new(env, &hook_text)?;

    let pixi_dir = work.join(".pixi");
    let staged = staging.dir.as_path();
    copy_into_export(staging, &env_dir, &[staged])?;
    for dir in &sources {
        copy_into_export(staging, dir, &[staged, &pixi_dir])?;
    }
    for file in hook.sourced_files() {
        copy_sourced_file(staging, &file)?;
    }
    write_file(&staging.within(HOOK_PATH), &hook.0, 0o644)?;
    write_file(&staging.within(ENTRYPOINT_PATH), &entrypoint(&bash), 0o755)?;
    prune_pixi_dir(work)
}

fn entrypoint(bash: &Path) -> String {
    let bash = bash.to_string_lossy().replace('\'', "'\\''");
    format!(
        "#!/bin/sh\nexec '{bash}' -c '. /etc/profile.d/pixi-env.sh && exec \"$@\"' bash \"$@\"\n"
    )
}
