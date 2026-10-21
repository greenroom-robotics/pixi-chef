#![allow(clippy::panic_in_result_fn)]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use pixi_chef::prepare::{PrepareError, prepare};

type TestResult = Result<(), Box<dyn Error>>;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    let mut entries = fs::read_dir(dir)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn tree(dir: &Path) -> Result<String, Box<dyn Error>> {
    let mut files = Vec::new();
    collect_files(dir, &mut files)?;
    let mut rendered = String::new();
    for file in files {
        let content = fs::read_to_string(&file)?;
        writeln!(
            rendered,
            "=== {}\n{content}",
            file.strip_prefix(dir)?.display()
        )?;
    }
    Ok(rendered)
}

fn recipe(fixture_name: &str, manifest: &str) -> Result<String, Box<dyn Error>> {
    let out = tempfile::tempdir()?;
    let recipe_dir = out.path().join("recipe");
    prepare(&fixture(fixture_name), Path::new(manifest), &recipe_dir)?;
    tree(&recipe_dir)
}

#[test]
fn conda_source_package() -> TestResult {
    insta::assert_snapshot!(recipe("conda_source", "pixi.toml")?);
    Ok(())
}

#[test]
fn pypi_source_package() -> TestResult {
    insta::assert_snapshot!(recipe("pypi_source", "pixi.toml")?);
    Ok(())
}

#[test]
fn source_package_depending_on_another() -> TestResult {
    insta::assert_snapshot!(recipe("source_chain", "ws/pixi.toml")?);
    Ok(())
}

#[test]
fn local_conda_binary_is_not_a_source_package() -> TestResult {
    insta::assert_snapshot!(recipe("local_binary", "pixi.toml")?);
    Ok(())
}

#[test]
fn fails_when_out_is_not_empty() -> TestResult {
    let out = tempfile::tempdir()?;
    fs::write(out.path().join("stale"), "")?;
    let result = prepare(&fixture("conda_source"), Path::new("pixi.toml"), out.path());
    assert!(matches!(result, Err(PrepareError::OutNotEmpty(_))));
    Ok(())
}

#[test]
fn accepts_existing_empty_out() -> TestResult {
    let out = tempfile::tempdir()?;
    prepare(&fixture("conda_source"), Path::new("pixi.toml"), out.path())?;
    assert!(out.path().join("app/package.xml").is_file());
    Ok(())
}

#[test]
fn fails_when_source_directory_is_missing() -> TestResult {
    let repo = tempfile::tempdir()?;
    for name in ["pixi.toml", "pixi.lock"] {
        fs::copy(fixture("conda_source").join(name), repo.path().join(name))?;
    }
    let out = tempfile::tempdir()?;
    let recipe_dir = out.path().join("recipe");
    let result = prepare(repo.path(), Path::new("pixi.toml"), &recipe_dir);
    assert!(matches!(result, Err(PrepareError::MissingSourceDir(p)) if p == Path::new("app")));
    assert!(!recipe_dir.exists());
    Ok(())
}
