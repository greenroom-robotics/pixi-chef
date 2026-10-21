#![allow(clippy::panic_in_result_fn)]

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use pixi_chef::cook::{CookError, SkipList, cook, install_command};
use pixi_chef::prepare::prepare;
use rattler_lock::{CondaPackageData, LockFile, LockedPackage};

type TestResult = Result<(), Box<dyn Error>>;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cook")
}

fn skip_list(environment: &str) -> Result<SkipList, Box<dyn Error>> {
    let lock = LockFile::from_path(&fixture().join("pixi.lock"))?;
    Ok(SkipList::new(&lock, environment)?)
}

fn rendered_command(environment: &str) -> Result<String, Box<dyn Error>> {
    let command = install_command(environment, &skip_list(environment)?);
    let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy()).collect();
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
        SkipList::new(&lock, "nope"),
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

struct Cooked {
    status: ExitStatus,
    expected: BTreeSet<String>,
    installed: BTreeSet<String>,
}

fn cooked(environment: &str) -> Result<Cooked, Box<dyn Error>> {
    let out = tempfile::tempdir()?;
    let recipe = out.path().join("recipe");
    prepare(&fixture(), Path::new("pixi.toml"), &recipe)?;
    let status = cook(&recipe, environment)?;
    let lock = LockFile::from_path(&recipe.join("pixi.lock"))?;
    let expected = binary_packages(&lock, environment)?;
    let installed = installed_packages(&recipe, environment).unwrap_or_default();
    Ok(Cooked {
        status,
        expected,
        installed,
    })
}

#[test]
fn cook_installs_every_binary_package_and_no_source_package() -> TestResult {
    for environment in ["default", "conda", "py"] {
        let cooked = cooked(environment)?;
        assert!(cooked.status.success(), "{environment}");
        assert_eq!(cooked.installed, cooked.expected, "{environment}");
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
