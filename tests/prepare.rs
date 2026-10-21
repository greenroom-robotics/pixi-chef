#![allow(clippy::panic_in_result_fn)]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use globset::Glob;
use pixi_chef::prepare::{PrepareError, PrepareOptions, prepare};

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

fn recipe_with(
    fixture_name: &str,
    manifest: &str,
    options: &PrepareOptions,
) -> Result<String, Box<dyn Error>> {
    let out = tempfile::tempdir()?;
    let recipe_dir = out.path().join("recipe");
    prepare(
        &fixture(fixture_name),
        Path::new(manifest),
        &recipe_dir,
        options,
    )?;
    tree(&recipe_dir)
}

fn recipe(fixture_name: &str, manifest: &str) -> Result<String, Box<dyn Error>> {
    recipe_with(fixture_name, manifest, &PrepareOptions::default())
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

fn into_shared() -> PrepareOptions {
    PrepareOptions {
        into: Some(PathBuf::from("variants/shared")),
        include: Vec::new(),
    }
}

#[test]
fn into_places_manifest_and_lock_at_the_given_directory() -> TestResult {
    insta::assert_snapshot!(recipe_with(
        "variants",
        "variants/base/pixi.toml",
        &into_shared()
    )?);
    Ok(())
}

#[test]
fn variants_at_the_same_depth_share_one_recipe_layout() -> TestResult {
    let layout = |manifest| -> Result<Vec<String>, Box<dyn Error>> {
        Ok(recipe_with("variants", manifest, &into_shared())?
            .lines()
            .filter(|line| line.starts_with("==="))
            .map(str::to_owned)
            .collect())
    };
    assert_eq!(
        layout("variants/base/pixi.toml")?,
        layout("variants/a/pixi.toml")?
    );
    assert_eq!(
        layout("variants/base/pixi.toml")?,
        layout("variants/b/pixi.toml")?
    );
    Ok(())
}

#[test]
fn include_copies_matching_files_and_skips_the_rest() -> TestResult {
    let options = PrepareOptions {
        into: None,
        include: vec![Glob::new("cmake/*.cmake")?],
    };
    insta::assert_snapshot!(recipe_with(
        "variants",
        "variants/base/pixi.toml",
        &options
    )?);
    Ok(())
}

#[test]
fn fails_when_out_is_not_empty() -> TestResult {
    let out = tempfile::tempdir()?;
    fs::write(out.path().join("stale"), "")?;
    let result = prepare(
        &fixture("conda_source"),
        Path::new("pixi.toml"),
        out.path(),
        &PrepareOptions::default(),
    );
    assert!(matches!(result, Err(PrepareError::OutNotEmpty(_))));
    Ok(())
}

#[test]
fn accepts_existing_empty_out() -> TestResult {
    let out = tempfile::tempdir()?;
    prepare(
        &fixture("conda_source"),
        Path::new("pixi.toml"),
        out.path(),
        &PrepareOptions::default(),
    )?;
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
    let result = prepare(
        repo.path(),
        Path::new("pixi.toml"),
        &recipe_dir,
        &PrepareOptions::default(),
    );
    assert!(matches!(result, Err(PrepareError::MissingSourceDir(p)) if p == Path::new("app")));
    assert!(!recipe_dir.exists());
    Ok(())
}

fn repo_with(files: &[(&str, &str)]) -> Result<tempfile::TempDir, Box<dyn Error>> {
    let repo = tempfile::tempdir()?;
    for (name, content) in files {
        fs::write(repo.path().join(name), content)?;
    }
    Ok(repo)
}

#[test]
fn fails_when_manifest_is_missing() -> TestResult {
    let repo = repo_with(&[])?;
    let out = tempfile::tempdir()?;
    let result = prepare(
        repo.path(),
        Path::new("pixi.toml"),
        &out.path().join("r"),
        &PrepareOptions::default(),
    );
    assert!(matches!(result, Err(PrepareError::MissingManifest(_))));
    Ok(())
}

#[test]
fn fails_when_lock_is_missing() -> TestResult {
    let repo = repo_with(&[("pixi.toml", "")])?;
    let out = tempfile::tempdir()?;
    let result = prepare(
        repo.path(),
        Path::new("pixi.toml"),
        &out.path().join("r"),
        &PrepareOptions::default(),
    );
    assert!(matches!(result, Err(PrepareError::Lock { .. })));
    Ok(())
}

#[test]
fn fails_when_source_path_escapes_root() -> TestResult {
    let lock =
        fs::read_to_string(fixture("conda_source").join("pixi.lock"))?.replace("@ app", "@ ../app");
    let repo = repo_with(&[("pixi.toml", ""), ("pixi.lock", &lock)])?;
    let out = tempfile::tempdir()?;
    let result = prepare(
        repo.path(),
        Path::new("pixi.toml"),
        &out.path().join("r"),
        &PrepareOptions::default(),
    );
    assert!(matches!(result, Err(PrepareError::Path(_))));
    Ok(())
}

#[test]
fn fails_when_out_is_a_file() -> TestResult {
    let out = tempfile::tempdir()?;
    let file = out.path().join("file");
    fs::write(&file, "")?;
    let result = prepare(
        &fixture("conda_source"),
        Path::new("pixi.toml"),
        &file,
        &PrepareOptions::default(),
    );
    assert!(result.is_err());
    Ok(())
}
