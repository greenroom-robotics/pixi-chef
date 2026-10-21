use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use rattler_lock::{LockFile, UrlOrPath};

use crate::fs::{EmptyDir, EmptyDirError, MissingSourceDir};
use crate::include::IncludeGlob;
use crate::path::{RepoPath, RepoPathError};
use crate::workspace::{LOCK_FILE, LockError, MANIFEST_FILE, SourcePackage, location, read_lock};

const METADATA_FILES: [&str; 8] = [
    MANIFEST_FILE,
    LOCK_FILE,
    "package.xml",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "CMakeLists.txt",
    "README.md",
];

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error(transparent)]
    OutDir(#[from] EmptyDirError),
    #[error(transparent)]
    MissingSourceDir(#[from] MissingSourceDir),
    #[error("manifest {0} does not exist")]
    MissingManifest(PathBuf),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("recipe path {0} would be written from two different files")]
    Collision(PathBuf),
    #[error("--include {0} matched no file in any source package")]
    UnmatchedInclude(String),
    #[error("walking {path}")]
    Walk {
        path: PathBuf,
        source: ignore::Error,
    },
    #[error(transparent)]
    Path(#[from] RepoPathError),
    #[error("{path}")]
    Io { path: PathBuf, source: io::Error },
}

impl PrepareError {
    fn io(path: &Path) -> impl FnOnce(io::Error) -> Self {
        let path = path.to_path_buf();
        move |source| Self::Io { path, source }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Path(#[from] RepoPathError),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("manifest {0} must be named pixi.toml")]
    NotPixiToml(PathBuf),
    #[error(
        "--into {}: lock path {lock_path} resolves differently from there than from {}",
        shown(.into),
        shown(.manifest_dir)
    )]
    IntoChangesPath {
        into: PathBuf,
        manifest_dir: PathBuf,
        lock_path: String,
    },
}

fn shown(dir: &Path) -> String {
    if dir.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        dir.display().to_string()
    }
}

/// A workspace manifest and the directory it is placed at in the recipe.
///
/// The recipe directory is the manifest's own directory, or one from which
/// every local path in the lock resolves to the same repo path as from the
/// manifest's directory.
#[must_use]
#[derive(Debug, Clone)]
pub struct Workspace {
    manifest: RepoPath,
    recipe_dir: RepoPath,
}

impl Workspace {
    pub fn new(manifest: &Path, into: Option<&Path>, root: &Path) -> Result<Self, WorkspaceError> {
        let manifest = RepoPath::new(manifest, root)?;
        if manifest.file_name() != Some(OsStr::new(MANIFEST_FILE)) {
            return Err(WorkspaceError::NotPixiToml(
                manifest.as_path().to_path_buf(),
            ));
        }
        let manifest_dir = manifest.parent();
        let recipe_dir = match into {
            None => manifest_dir,
            Some(into) => {
                let into = RepoPath::new(into, root)?;
                if into != manifest_dir {
                    check_paths_resolve_alike(root, &manifest_dir, &into)?;
                }
                into
            }
        };
        Ok(Self {
            manifest,
            recipe_dir,
        })
    }
}

fn check_paths_resolve_alike(
    root: &Path,
    manifest_dir: &RepoPath,
    into: &RepoPath,
) -> Result<(), WorkspaceError> {
    let lock = read_lock(&root.join(manifest_dir.as_path()).join(LOCK_FILE))?;
    for package in lock.packages() {
        if let UrlOrPath::Path(path) = location(package) {
            let relative = Path::new(path.as_str());
            if manifest_dir.join(relative, root).ok() != into.join(relative, root).ok() {
                return Err(WorkspaceError::IntoChangesPath {
                    into: into.as_path().to_path_buf(),
                    manifest_dir: manifest_dir.as_path().to_path_buf(),
                    lock_path: path.to_string(),
                });
            }
        }
    }
    Ok(())
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

/// Maps each recipe path to the repo path it is copied from.
struct Recipe {
    files: Files,
}

impl Recipe {
    fn gather(
        root: &Path,
        workspace: &Workspace,
        include: &[IncludeGlob],
    ) -> Result<Self, PrepareError> {
        if !root.join(workspace.manifest.as_path()).is_file() {
            return Err(PrepareError::MissingManifest(
                workspace.manifest.as_path().to_path_buf(),
            ));
        }
        let source_lock = workspace
            .manifest
            .parent()
            .join(Path::new(LOCK_FILE), root)?;
        let lock = read_lock(&root.join(source_lock.as_path()))?;

        let mut files = Files::default();
        files.add(
            workspace.recipe_dir.join(Path::new(MANIFEST_FILE), root)?,
            workspace.manifest.clone(),
        )?;
        files.add(
            workspace.recipe_dir.join(Path::new(LOCK_FILE), root)?,
            source_lock,
        )?;

        let mut matched = BTreeSet::new();
        for dir in source_dirs(&lock, &workspace.manifest.parent(), root)? {
            if !root.join(dir.as_path()).is_dir() {
                return Err(MissingSourceDir(dir.as_path().to_path_buf()).into());
            }
            let found = metadata_files(root, &dir, include)?;
            matched.extend(found.matched);
            for relative in found.files {
                let file = dir.join(&relative, root)?;
                files.add(file.clone(), file)?;
            }
        }
        if let Some((_, glob)) = include
            .iter()
            .enumerate()
            .find(|(index, _)| !matched.contains(index))
        {
            return Err(PrepareError::UnmatchedInclude(glob.pattern().to_owned()));
        }
        Ok(Self { files })
    }

    fn write(&self, root: &Path, out: &EmptyDir) -> Result<(), PrepareError> {
        for (file, source) in &self.files.0 {
            let from = root.join(source.as_path());
            let to = out.path().join(file.as_path());
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

fn metadata_files(
    root: &Path,
    dir: &RepoPath,
    include: &[IncludeGlob],
) -> Result<Found, PrepareError> {
    let dir_path = root.join(dir.as_path());
    let mut files: Vec<PathBuf> = METADATA_FILES
        .iter()
        .filter(|name| dir_path.join(name).is_file())
        .map(PathBuf::from)
        .collect();
    let mut matched = BTreeSet::new();
    if !include.is_empty() {
        let walk = WalkBuilder::new(root)
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .git_global(false)
            .git_exclude(false)
            .ignore(false)
            .parents(false)
            .filter_entry({
                let dir_path = dir_path.clone();
                move |entry| {
                    entry.file_name() != ".git"
                        && (entry.path().starts_with(&dir_path)
                            || dir_path.starts_with(entry.path()))
                }
            })
            .build();
        for entry in walk {
            let entry = entry.map_err(|source| PrepareError::Walk {
                path: dir_path.clone(),
                source,
            })?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(&dir_path) else {
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
) -> Result<BTreeSet<RepoPath>, RepoPathError> {
    let mut dirs = BTreeSet::new();
    for package in lock.packages() {
        if let Some(SourcePackage {
            location: UrlOrPath::Path(path),
            ..
        }) = SourcePackage::of(package)
        {
            dirs.insert(lock_dir.join(Path::new(path.as_str()), root)?);
        }
    }
    Ok(dirs)
}

pub fn prepare(
    root: &Path,
    workspace: &Workspace,
    out: &Path,
    include: &[IncludeGlob],
) -> Result<(), PrepareError> {
    let out = EmptyDir::new(out)?;
    Recipe::gather(root, workspace, include)?.write(root, &out)
}
