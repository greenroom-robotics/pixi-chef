use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use rattler_lock::{
    CondaPackageData, LockFile, LockedPackage, ParseCondaLockError, PypiPackageData, UrlOrPath,
};

const LOCK_FILE: &str = "pixi.lock";
const HOOK_PATH: &str = "etc/profile.d/pixi-env.sh";
const ENTRYPOINT_PATH: &str = "entrypoint.sh";
const ENTRYPOINT: &str = "#!/bin/sh\n. /etc/profile.d/pixi-env.sh\nexec \"$@\"\n";

#[derive(Debug, thiserror::Error)]
pub enum BakeError {
    #[error("export directory {0} exists and is not empty")]
    ExportNotEmpty(PathBuf),
    #[error("source package directory {0} does not exist")]
    MissingSourceDir(PathBuf),
    #[error("environment {0} was not installed")]
    MissingEnvironment(PathBuf),
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

/// A shell hook with at least one non-whitespace character.
struct ShellHook(String);

impl ShellHook {
    fn new(env: &str, text: String) -> Result<Self, BakeError> {
        if text.trim().is_empty() {
            Err(BakeError::EmptyHook(env.to_string()))
        } else {
            Ok(Self(text))
        }
    }
}

/// A directory that does not exist yet or exists and holds nothing.
struct EmptyDir(PathBuf);

impl EmptyDir {
    fn new(path: &Path) -> Result<Self, BakeError> {
        match fs::read_dir(path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(BakeError::ExportNotEmpty(path.to_path_buf()));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(BakeError::io(path)(e)),
        }
        fs::create_dir_all(path).map_err(BakeError::io(path))?;
        let absolute = path.canonicalize().map_err(BakeError::io(path))?;
        Ok(Self(absolute))
    }

    /// Where an absolute path of the build machine lands inside the export.
    fn at(&self, absolute: &Path) -> PathBuf {
        self.0.join(absolute.strip_prefix("/").unwrap_or(absolute))
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

    fn capture(&self, workspace: &Path, args: &[&str]) -> Result<Vec<u8>, BakeError> {
        let command = self.describe(args);
        let output = self
            .command(workspace, args)
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

fn source_dirs(workspace: &Path) -> Result<BTreeSet<PathBuf>, BakeError> {
    let lock_path = workspace.join(LOCK_FILE);
    let lock = LockFile::from_path(&lock_path).map_err(|source| BakeError::Lock {
        path: lock_path,
        source: Box::new(source),
    })?;
    let mut dirs = BTreeSet::new();
    for package in lock.packages() {
        let location = match package {
            LockedPackage::Conda(CondaPackageData::Source(source)) => Some(&source.location),
            LockedPackage::Pypi(PypiPackageData::Source(source)) => Some(&*source.location),
            LockedPackage::Conda(CondaPackageData::Binary(_))
            | LockedPackage::Pypi(PypiPackageData::Distribution(_)) => None,
        };
        if let Some(UrlOrPath::Path(path)) = location {
            let dir = workspace.join(path.as_str());
            let dir = dir
                .canonicalize()
                .map_err(|_| BakeError::MissingSourceDir(dir))?;
            dirs.insert(dir);
        }
    }
    Ok(dirs)
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

fn copy_tree(from: &Path, to: &Path, skip: &Path) -> Result<(), BakeError> {
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
            if path.canonicalize().is_ok_and(|real| real == skip) {
                continue;
            }
            copy_tree(&path, &to.join(entry.file_name()), skip)?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map_err(BakeError::io(from)).map(drop)
    }
}

fn copy_into_export(export: &EmptyDir, absolute: &Path) -> Result<(), BakeError> {
    let to = export.at(absolute);
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(BakeError::io(parent))?;
    }
    copy_tree(absolute, &to, &export.0)
}

fn write_file(path: &Path, content: &str, mode: u32) -> Result<(), BakeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(BakeError::io(parent))?;
    }
    fs::write(path, content).map_err(BakeError::io(path))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(BakeError::io(path))
}

pub fn bake(pixi: &Pixi, workspace: &Path, env: &str, export: &Path) -> Result<(), BakeError> {
    let workspace = workspace.canonicalize().map_err(BakeError::io(workspace))?;
    let export = EmptyDir::new(export)?;
    let sources = source_dirs(&workspace)?;

    pixi.run(&workspace, &["install", "--locked", "-e", env])?;
    prune_pixi_dir(&workspace)?;
    let hook_bytes = pixi.capture(&workspace, &["shell-hook", "--as-is", "-e", env])?;
    let hook = ShellHook::new(
        env,
        String::from_utf8(hook_bytes).map_err(|_| BakeError::HookNotUtf8)?,
    )?;

    let env_dir = workspace.join(".pixi/envs").join(env);
    if !env_dir.is_dir() {
        return Err(BakeError::MissingEnvironment(env_dir));
    }
    copy_into_export(&export, &env_dir)?;
    for dir in &sources {
        copy_into_export(&export, dir)?;
    }
    write_file(&export.0.join(HOOK_PATH), &hook.0, 0o644)?;
    write_file(&export.0.join(ENTRYPOINT_PATH), ENTRYPOINT, 0o755)
}
