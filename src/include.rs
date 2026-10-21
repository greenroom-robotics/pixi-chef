use std::path::{Component, Path};
use std::str::FromStr;

use globset::{GlobBuilder, GlobMatcher};

#[derive(Debug, thiserror::Error)]
pub enum IncludeGlobError {
    #[error("include pattern {0} must be relative to a source package directory")]
    Absolute(String),
    #[error("include pattern {0} must not contain `..`")]
    ParentDir(String),
    #[error("invalid include pattern: {0}")]
    Invalid(#[from] globset::Error),
}

/// A glob confined to a source package directory: relative and free of `..`.
///
/// `*` does not cross a `/` within one match, but a match on a directory
/// selects every file below it, so `*` alone selects the whole directory tree.
#[derive(Debug, Clone)]
pub struct IncludeGlob {
    pattern: String,
    matcher: GlobMatcher,
}

impl IncludeGlob {
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn matches(&self, relative: &Path) -> bool {
        relative
            .ancestors()
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
            .any(|ancestor| self.matcher.is_match(ancestor))
    }
}

impl FromStr for IncludeGlob {
    type Err = IncludeGlobError;

    fn from_str(pattern: &str) -> Result<Self, Self::Err> {
        let path = Path::new(pattern);
        if path.is_absolute() {
            return Err(IncludeGlobError::Absolute(pattern.to_owned()));
        }
        if path.components().any(|part| part == Component::ParentDir) {
            return Err(IncludeGlobError::ParentDir(pattern.to_owned()));
        }
        let matcher = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()?
            .compile_matcher();
        Ok(Self {
            pattern: pattern.to_owned(),
            matcher,
        })
    }
}
