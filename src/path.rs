use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum RepoPathError {
    #[error("{0} leaves the repo root")]
    EscapesRoot(PathBuf),
    #[error("{path} is outside the repo root {root}")]
    OutsideRoot { path: PathBuf, root: PathBuf },
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPath(PathBuf);

impl RepoPath {
    pub fn new(path: &Path, root: &Path) -> Result<Self, RepoPathError> {
        let relative = if path.is_absolute() {
            path.strip_prefix(root)
                .map_err(|_| RepoPathError::OutsideRoot {
                    path: path.to_path_buf(),
                    root: root.to_path_buf(),
                })?
        } else {
            path
        };
        Self::normalized(relative)
    }

    pub fn join(&self, relative: &Path, root: &Path) -> Result<Self, RepoPathError> {
        if relative.is_absolute() {
            Self::new(relative, root)
        } else {
            Self::normalized(&self.0.join(relative))
        }
    }

    pub fn file_name(&self) -> Option<&OsStr> {
        self.0.file_name()
    }

    pub fn parent(&self) -> Self {
        Self(self.0.parent().map(Path::to_path_buf).unwrap_or_default())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    fn normalized(path: &Path) -> Result<Self, RepoPathError> {
        let mut parts: Vec<Component<'_>> = Vec::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !matches!(parts.pop(), Some(Component::Normal(_))) {
                        return Err(RepoPathError::EscapesRoot(path.to_path_buf()));
                    }
                }
                Component::Normal(_) => parts.push(component),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(RepoPathError::EscapesRoot(path.to_path_buf()));
                }
            }
        }
        Ok(Self(parts.into_iter().collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new("/repo")
    }

    #[test]
    fn normalizes_dots() {
        let base = RepoPath::new(Path::new("./app/sub"), root()).unwrap();
        let joined = base.join(Path::new("../../pkg/./a"), root()).unwrap();
        assert_eq!(joined.as_path(), Path::new("pkg/a"));
    }

    #[test]
    fn rejects_escape() {
        let base = RepoPath::new(Path::new("app"), root()).unwrap();
        assert!(base.join(Path::new("../.."), root()).is_err());
    }

    #[test]
    fn strips_absolute_root() {
        let p = RepoPath::new(Path::new("/repo/app/pixi.toml"), root()).unwrap();
        assert_eq!(p.as_path(), Path::new("app/pixi.toml"));
        assert!(RepoPath::new(Path::new("/elsewhere/x"), root()).is_err());
    }

    #[test]
    fn parent_of_top_level_is_empty() {
        let p = RepoPath::new(Path::new("pixi.toml"), root()).unwrap();
        assert_eq!(p.parent().as_path(), Path::new(""));
    }
}
