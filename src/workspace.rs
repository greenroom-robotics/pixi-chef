use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rattler_conda_types::Subdir;
use rattler_lock::{
    CondaPackageData, LockFile, LockedPackage, ParseCondaLockError, PypiPackageData, UrlOrPath,
};

pub const MANIFEST_FILE: &str = "pixi.toml";
pub const LOCK_FILE: &str = "pixi.lock";

#[derive(Debug, thiserror::Error)]
#[error("reading lock file {path}")]
pub struct LockError {
    path: PathBuf,
    #[source]
    source: Box<ParseCondaLockError>,
}

pub fn read_lock(path: &Path) -> Result<LockFile, LockError> {
    LockFile::from_path(path).map_err(|source| LockError {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// A lock entry built from source (a local directory, git or URL), named as the lock records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePackage<'lock> {
    pub name: String,
    pub location: &'lock UrlOrPath,
}

impl<'lock> SourcePackage<'lock> {
    pub fn of(package: &'lock LockedPackage) -> Option<Self> {
        match package {
            LockedPackage::Conda(CondaPackageData::Source(source)) => Some(Self {
                name: source.name().as_source().to_owned(),
                location: &source.location,
            }),
            LockedPackage::Pypi(PypiPackageData::Source(source)) => Some(Self {
                name: source.name.to_string(),
                location: &source.location,
            }),
            LockedPackage::Conda(CondaPackageData::Binary(_))
            | LockedPackage::Pypi(PypiPackageData::Distribution(_)) => None,
        }
    }
}

pub fn location(package: &LockedPackage) -> &UrlOrPath {
    match package {
        LockedPackage::Conda(package) => package.location(),
        LockedPackage::Pypi(package) => package.location(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    #[error("environment {0} is not in the lock file")]
    UnknownEnvironment(String),
    #[error("environment {0} has no packages locked for this platform")]
    NotLockedForPlatform(String),
    #[error("this platform has no conda subdir")]
    UnsupportedPlatform,
}

pub fn current_subdir() -> Result<Subdir, ScopeError> {
    Subdir::current().ok_or(ScopeError::UnsupportedPlatform)
}

/// Every package one environment locks for a subdir.
pub fn scoped_packages<'lock>(
    lock: &'lock LockFile,
    environment: &str,
    subdir: Subdir,
) -> Result<Vec<&'lock LockedPackage>, ScopeError> {
    let locked = lock
        .environment(environment)
        .ok_or_else(|| ScopeError::UnknownEnvironment(environment.to_owned()))?;
    let mut platforms = locked
        .packages_by_platform()
        .filter(|(platform, _)| platform.subdir() == subdir)
        .peekable();
    if platforms.peek().is_none() {
        return Err(ScopeError::NotLockedForPlatform(environment.to_owned()));
    }
    Ok(platforms.flat_map(|(_, packages)| packages).collect())
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("reading manifest {path}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parsing manifest {path}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("manifest key `{0}` has the wrong type")]
    WrongType(String),
    #[error("environment {0} is not in the manifest")]
    UnknownEnvironment(String),
}

/// A path the manifest lists, relative to the manifest's directory unless absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestPath {
    pub key: String,
    pub path: PathBuf,
}

/// The parts of a workspace manifest that name files outside the lock.
#[derive(Debug)]
pub struct Manifest(toml::Table);

struct Features<'manifest> {
    default: bool,
    named: BTreeSet<&'manifest str>,
}

struct Activation<'manifest> {
    key: String,
    feature: Option<&'manifest str>,
    target: Option<&'manifest str>,
    table: &'manifest toml::Table,
}

impl Activation<'_> {
    fn scripts(&self) -> Result<Vec<ManifestPath>, ManifestError> {
        let key = format!("[{}] scripts", self.key);
        strings(self.table.get("scripts"), &key)?
            .into_iter()
            .map(|path| {
                Ok(ManifestPath {
                    key: key.clone(),
                    path: PathBuf::from(path),
                })
            })
            .collect()
    }

    fn applies_to(&self, features: &Features<'_>, subdir: Subdir) -> bool {
        let in_environment = self
            .feature
            .map_or(features.default, |name| features.named.contains(name));
        in_environment && self.target.is_none_or(|target| selects(target, subdir))
    }
}

fn selects(selector: &str, subdir: Subdir) -> bool {
    match selector {
        "unix" => subdir.is_unix(),
        "win" => subdir.is_windows(),
        "linux" => subdir.is_linux(),
        "osx" => subdir.is_osx(),
        other => other == subdir.as_str(),
    }
}

fn as_table<'a>(value: &'a toml::Value, key: &str) -> Result<&'a toml::Table, ManifestError> {
    value
        .as_table()
        .ok_or_else(|| ManifestError::WrongType(key.to_owned()))
}

fn child<'a>(
    table: &'a toml::Table,
    name: &str,
    key: &str,
) -> Result<Option<&'a toml::Table>, ManifestError> {
    table
        .get(name)
        .map(|value| as_table(value, key))
        .transpose()
}

fn strings<'a>(value: Option<&'a toml::Value>, key: &str) -> Result<Vec<&'a str>, ManifestError> {
    let wrong = || ManifestError::WrongType(key.to_owned());
    value.map_or(Ok(Vec::new()), |value| {
        value
            .as_array()
            .ok_or_else(wrong)?
            .iter()
            .map(|item| item.as_str().ok_or_else(wrong))
            .collect()
    })
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self, ManifestError> {
        let text = fs::read_to_string(path).map_err(|source| ManifestError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        text.parse()
            .map(Self)
            .map_err(|source| ManifestError::Parse {
                path: path.to_path_buf(),
                source: Box::new(source),
            })
    }

    fn activations(&self) -> Result<Vec<Activation<'_>>, ManifestError> {
        let mut scopes = vec![(None, String::new(), &self.0)];
        if let Some(features) = child(&self.0, "feature", "feature")? {
            for (name, value) in features {
                let prefix = format!("feature.{name}.");
                scopes.push((
                    Some(name.as_str()),
                    prefix.clone(),
                    as_table(value, &prefix)?,
                ));
            }
        }
        let mut found = Vec::new();
        for (feature, prefix, scope) in scopes {
            let key = format!("{prefix}activation");
            if let Some(table) = child(scope, "activation", &key)? {
                found.push(Activation {
                    key,
                    feature,
                    target: None,
                    table,
                });
            }
            let target_tables_key = format!("{prefix}target");
            for (target, value) in child(scope, "target", &target_tables_key)?
                .into_iter()
                .flatten()
            {
                let target_key = format!("{target_tables_key}.{target}");
                let key = format!("{target_key}.activation");
                if let Some(table) = child(as_table(value, &target_key)?, "activation", &key)? {
                    found.push(Activation {
                        key,
                        feature,
                        target: Some(target.as_str()),
                        table,
                    });
                }
            }
        }
        Ok(found)
    }

    fn features_of(&self, environment: &str) -> Result<Features<'_>, ManifestError> {
        let key = format!("environments.{environment}");
        let Some(value) = child(&self.0, "environments", "environments")?
            .and_then(|environments| environments.get(environment))
        else {
            return if environment == "default" {
                Ok(Features {
                    default: true,
                    named: BTreeSet::new(),
                })
            } else {
                Err(ManifestError::UnknownEnvironment(environment.to_owned()))
            };
        };
        let (features, default) = match value {
            toml::Value::Table(table) => (
                table.get("features"),
                table.get("no-default-feature").map_or(Ok(true), |flag| {
                    flag.as_bool().map(|disabled| !disabled).ok_or_else(|| {
                        ManifestError::WrongType(format!("{key}.no-default-feature"))
                    })
                })?,
            ),
            other => (Some(other), true),
        };
        Ok(Features {
            default,
            named: strings(features, &key)?.into_iter().collect(),
        })
    }

    /// Every path the manifest lists that pixi resolves at install or activation time.
    pub fn relative_paths(&self) -> Result<Vec<ManifestPath>, ManifestError> {
        let mut paths = Vec::new();
        for activation in self.activations()? {
            paths.extend(activation.scripts()?);
        }
        for section in ["workspace", "project"] {
            let key = format!("{section}.build-variants-files");
            let files = child(&self.0, section, section)?
                .and_then(|table| table.get("build-variants-files"));
            paths.extend(strings(files, &key)?.into_iter().map(|path| ManifestPath {
                key: format!("[{section}] build-variants-files"),
                path: PathBuf::from(path),
            }));
        }
        Ok(paths)
    }

    /// The activation scripts one environment runs on a platform.
    pub fn activation_scripts(
        &self,
        environment: &str,
        subdir: Subdir,
    ) -> Result<Vec<ManifestPath>, ManifestError> {
        let features = self.features_of(environment)?;
        let mut paths = Vec::new();
        for activation in self.activations()? {
            if activation.applies_to(&features, subdir) {
                paths.extend(activation.scripts()?);
            }
        }
        Ok(paths)
    }
}
