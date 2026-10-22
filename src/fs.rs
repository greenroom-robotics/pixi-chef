use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[error("source package directory {0} does not exist")]
pub struct MissingSourceDir(pub PathBuf);

#[derive(Debug, thiserror::Error)]
pub enum EmptyDirError {
    #[error("directory {0} exists and is not empty")]
    NotEmpty(PathBuf),
    #[error("{path}")]
    Io { path: PathBuf, source: io::Error },
}

/// A directory that does not exist yet or exists and holds nothing.
pub struct EmptyDir(PathBuf);

impl EmptyDir {
    pub fn new(path: &Path) -> Result<Self, EmptyDirError> {
        match fs::read_dir(path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(EmptyDirError::NotEmpty(path.to_path_buf()));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(EmptyDirError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
        Ok(Self(path.to_path_buf()))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}
