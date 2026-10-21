#![allow(clippy::panic_in_result_fn)]

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use pixi_chef::cook::{CookError, SkipList, Termination, cook, install_command};
use pixi_chef::prepare::prepare;
use rattler_conda_types::Subdir;
use rattler_lock::{CondaPackageData, LockFile, LockedPackage, PypiPackageData};

type TestResult = Result<(), Box<dyn Error>>;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cook")
}

fn current_subdir() -> Subdir {
    Subdir::current().unwrap_or(Subdir::Linux64)
}

fn skip_list(environment: &str) -> Result<SkipList, Box<dyn Error>> {
    let lock = LockFile::from_path(&fixture().join("pixi.lock"))?;
    Ok(SkipList::new(&lock, environment, Subdir::Linux64)?)
}

fn rendered_command(environment: &str) -> Result<String, Box<dyn Error>> {
    let workspace = fixture();
    let command = install_command(&workspace, environment, &skip_list(environment)?);
    let args: Vec<_> = command
        .get_args()
        .map(|a| {
            a.to_string_lossy()
                .replace(&*workspace.to_string_lossy(), "<workspace>")
        })
        .collect();
    Ok(format!(
        "{} {}",
        command.get_program().to_string_lossy(),
        args.join(" ")
    ))
}

#[test]
fn default_environment_skips_nothing() -> TestResult {
    insta::assert_snapshot!(rendered_command("default")?);
    Ok(())
}

#[test]
fn conda_environment_skips_the_conda_source_package() -> TestResult {
    insta::assert_snapshot!(rendered_command("conda")?);
    Ok(())
}

#[test]
fn py_environment_skips_the_pypi_source_package_by_normalised_name() -> TestResult {
    insta::assert_snapshot!(rendered_command("py")?);
    Ok(())
}

#[test]
fn unknown_environment_is_an_error() -> TestResult {
    let lock = LockFile::from_path(&fixture().join("pixi.lock"))?;
    assert!(matches!(
        SkipList::new(&lock, "nope", Subdir::Linux64),
        Err(CookError::UnknownEnvironment(name)) if name == "nope"
    ));
    Ok(())
}

#[test]
fn missing_lock_is_an_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    assert!(matches!(
        cook(dir.path(), "default"),
        Err(CookError::Lock { .. })
    ));
    Ok(())
}

fn installed_packages(
    workspace: &Path,
    environment: &str,
) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let meta = workspace
        .join(".pixi/envs")
        .join(environment)
        .join("conda-meta");
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(meta)? {
        let file = entry?.file_name().to_string_lossy().into_owned();
        if let Some(stem) = file.strip_suffix(".json") {
            names.insert(stem.to_owned());
        }
    }
    Ok(names)
}

fn binary_packages(lock: &LockFile, environment: &str) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let environment = lock.environment(environment).ok_or("environment missing")?;
    Ok(environment
        .packages_by_platform()
        .filter(|(platform, _)| platform.subdir() == current_subdir())
        .flat_map(|(_, packages)| packages.collect::<Vec<_>>())
        .filter_map(|package| match package {
            LockedPackage::Conda(CondaPackageData::Binary(binary)) => Some(format!(
                "{}-{}-{}",
                binary.package_record.name.as_normalized(),
                binary.package_record.version,
                binary.package_record.build
            )),
            _ => None,
        })
        .collect())
}

fn installed_pypi(workspace: &Path, environment: &str) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let lib = workspace.join(".pixi/envs").join(environment).join("lib");
    let mut names = BTreeSet::new();
    let mut pythons = Vec::new();
    for entry in fs::read_dir(lib)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("python") && entry.path().is_dir() {
            pythons.push(entry.path());
        }
    }
    if pythons.is_empty() {
        return Err("no python installed".into());
    }
    for python in pythons {
        let site_packages = python.join("site-packages");
        for entry in fs::read_dir(site_packages)? {
            let file = entry?.file_name().to_string_lossy().into_owned();
            if let Some(stem) = file.strip_suffix(".dist-info") {
                let name = stem.split('-').next().unwrap_or(stem);
                names.insert(name.to_lowercase().replace('_', "-"));
            }
        }
    }
    Ok(names)
}

fn locked_pypi(
    lock: &LockFile,
    environment: &str,
) -> Result<(BTreeSet<String>, BTreeSet<String>), Box<dyn Error>> {
    let environment = lock.environment(environment).ok_or("environment missing")?;
    let mut distributions = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for (_, packages) in environment
        .packages_by_platform()
        .filter(|(platform, _)| platform.subdir() == current_subdir())
    {
        for package in packages {
            match package {
                LockedPackage::Pypi(PypiPackageData::Distribution(d)) => {
                    distributions.insert(d.name.to_string());
                }
                LockedPackage::Pypi(PypiPackageData::Source(s)) => {
                    sources.insert(s.name.to_string());
                }
                LockedPackage::Conda(_) => {}
            }
        }
    }
    Ok((distributions, sources))
}

struct Cooked {
    status: ExitStatus,
    expected: BTreeSet<String>,
    installed: BTreeSet<String>,
    pypi_distributions: BTreeSet<String>,
    pypi_sources: BTreeSet<String>,
    pypi_installed: Option<BTreeSet<String>>,
}

fn cooked(environment: &str) -> Result<Cooked, Box<dyn Error>> {
    let out = tempfile::tempdir()?;
    let recipe = out.path().join("recipe");
    prepare(&fixture(), Path::new("pixi.toml"), &recipe)?;
    let status = cook(&recipe, environment)?;
    let lock = LockFile::from_path(&recipe.join("pixi.lock"))?;
    let (pypi_distributions, pypi_sources) = locked_pypi(&lock, environment)?;
    let has_pypi = !pypi_distributions.is_empty() || !pypi_sources.is_empty();
    let pypi_installed = if has_pypi {
        Some(installed_pypi(&recipe, environment)?)
    } else {
        None
    };
    Ok(Cooked {
        status,
        expected: binary_packages(&lock, environment)?,
        installed: installed_packages(&recipe, environment)?,
        pypi_distributions,
        pypi_sources,
        pypi_installed,
    })
}

#[test]
fn cook_installs_every_binary_package_and_no_source_package() -> TestResult {
    for environment in ["default", "conda", "py"] {
        let cooked = cooked(environment)?;
        assert!(cooked.status.success(), "{environment}");
        assert_eq!(cooked.installed, cooked.expected, "{environment}");
        if let Some(installed) = &cooked.pypi_installed {
            assert!(!cooked.pypi_distributions.is_empty(), "{environment}");
            assert!(
                cooked.pypi_distributions.is_subset(installed),
                "{environment}"
            );
            assert!(cooked.pypi_sources.is_disjoint(installed), "{environment}");
        }
    }
    Ok(())
}

#[test]
fn cook_propagates_a_failing_exit_status() -> TestResult {
    let out = tempfile::tempdir()?;
    let recipe = out.path().join("recipe");
    prepare(&fixture(), Path::new("pixi.toml"), &recipe)?;
    fs::write(
        recipe.join("pixi.toml"),
        "[workspace]\nname = \"other\"\nchannels = []\nplatforms = []\n",
    )?;
    let status = cook(&recipe, "default")?;
    assert!(!status.success());
    Ok(())
}

#[test]
fn exit_byte_passes_codes_through() {
    assert_eq!(Termination::Code(0).exit_byte(), 0);
    assert_eq!(Termination::Code(7).exit_byte(), 7);
}

#[test]
fn exit_byte_maps_out_of_range_codes_to_failure() {
    assert_eq!(Termination::Code(256).exit_byte(), 1);
    assert_eq!(Termination::Code(-1).exit_byte(), 1);
}

#[test]
fn exit_byte_maps_signals_to_128_plus_signal() {
    assert_eq!(Termination::Signal(15).exit_byte(), 143);
}

#[test]
fn termination_reads_codes_and_signals_from_exit_status() -> TestResult {
    let code = std::process::Command::new("sh")
        .args(["-c", "exit 7"])
        .status()?;
    assert_eq!(Termination::from(code), Termination::Code(7));
    let signal = std::process::Command::new("sh")
        .args(["-c", "kill -TERM $$"])
        .status()?;
    assert_eq!(Termination::from(signal), Termination::Signal(15));
    Ok(())
}

fn run_binary_with_stub_pixi(script: &str) -> Result<Option<i32>, Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;
    let bin = tempfile::tempdir()?;
    let stub = bin.path().join("pixi");
    fs::write(&stub, format!("#!/bin/sh\n{script}\n"))?;
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755))?;
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_pixi-chef"))
        .args(["cook", "-e", "default"])
        .current_dir(fixture())
        .env("PATH", bin.path())
        .status()?;
    Ok(status.code())
}

#[test]
fn binary_exits_with_the_exit_code_of_pixi() -> TestResult {
    assert_eq!(run_binary_with_stub_pixi("exit 0")?, Some(0));
    assert_eq!(run_binary_with_stub_pixi("exit 42")?, Some(42));
    Ok(())
}

#[test]
fn binary_exits_128_plus_signal_when_pixi_is_killed() -> TestResult {
    assert_eq!(run_binary_with_stub_pixi("kill -TERM $$")?, Some(143));
    Ok(())
}

#[test]
fn skip_list_only_covers_the_requested_platform() -> TestResult {
    let lock = LockFile::from_path(&fixture().join("pixi.lock"))?;
    let on = |subdir| -> Result<Vec<String>, Box<dyn Error>> {
        Ok(SkipList::new(&lock, "conda", subdir)?
            .names()
            .map(str::to_owned)
            .collect())
    };
    assert_eq!(on(Subdir::Linux64)?, ["my_app"]);
    assert_eq!(on(Subdir::LinuxAarch64)?, [] as [std::string::String; 0]);
    Ok(())
}

#[test]
fn environment_without_the_platform_is_an_error() -> TestResult {
    let lock = LockFile::from_path(&fixture().join("pixi.lock"))?;
    assert!(matches!(
        SkipList::new(&lock, "conda", Subdir::Osx64),
        Err(CookError::NotLockedForPlatform(_))
    ));
    Ok(())
}
