#![allow(clippy::panic_in_result_fn)]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use pixi_chef::include::{IncludeGlob, IncludeGlobError};
use pixi_chef::prepare::{IntoDir, PrepareError, PrepareOptions, prepare};

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

fn into(dir: &str) -> Result<PrepareOptions, Box<dyn Error>> {
    Ok(PrepareOptions {
        into: Some(IntoDir::new(Path::new(dir), Path::new("/repo"))?),
        include: Vec::new(),
    })
}

fn into_shared() -> Result<PrepareOptions, Box<dyn Error>> {
    into("variants/shared")
}

fn including(patterns: &[&str]) -> Result<PrepareOptions, IncludeGlobError> {
    Ok(PrepareOptions {
        into: None,
        include: patterns
            .iter()
            .map(|pattern| pattern.parse::<IncludeGlob>())
            .collect::<Result<_, _>>()?,
    })
}

#[test]
fn into_places_manifest_and_lock_at_the_given_directory() -> TestResult {
    insta::assert_snapshot!(recipe_with(
        "variants",
        "variants/base/pixi.toml",
        &into_shared()?
    )?);
    Ok(())
}

#[test]
fn variants_at_the_same_depth_share_one_recipe_layout() -> TestResult {
    let layout = |manifest| -> Result<Vec<String>, Box<dyn Error>> {
        Ok(recipe_with("variants", manifest, &into_shared()?)?
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
        include: vec!["cmake/*.cmake".parse()?],
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
        let path = repo.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(repo)
}

fn app_repo(extra: &[(&str, &str)]) -> Result<tempfile::TempDir, Box<dyn Error>> {
    let lock = fs::read_to_string(fixture("conda_source").join("pixi.lock"))?;
    let mut files = vec![
        ("pixi.toml", ""),
        ("pixi.lock", lock.as_str()),
        ("app/package.xml", "<package/>"),
    ];
    files.extend_from_slice(extra);
    repo_with(&files)
}

fn prepare_in(
    repo: &tempfile::TempDir,
    manifest: &str,
    options: &PrepareOptions,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let out = tempfile::tempdir()?;
    let recipe_dir = out.path().join("recipe");
    prepare(repo.path(), Path::new(manifest), &recipe_dir, options)?;
    let mut files = Vec::new();
    collect_files(&recipe_dir, &mut files)?;
    files
        .iter()
        .map(|file| Ok(file.strip_prefix(&recipe_dir)?.to_path_buf()))
        .collect()
}

fn including_options(patterns: &[&str]) -> Result<PrepareOptions, Box<dyn Error>> {
    Ok(including(patterns)?)
}

#[test]
fn into_at_a_different_depth_is_rejected() -> TestResult {
    let result = recipe_with("variants", "variants/base/pixi.toml", &into("deep/x/y")?);
    assert!(matches!(result, Err(e) if e.to_string().contains("--into deep/x/y")));
    Ok(())
}

#[test]
fn into_resolving_sources_elsewhere_is_rejected() -> TestResult {
    let repo = app_repo(&[("app/pixi.toml", "[package]")])?;
    let out = tempfile::tempdir()?;
    let result = prepare(
        repo.path(),
        Path::new("pixi.toml"),
        &out.path().join("r"),
        &into("app")?,
    );
    assert!(matches!(result, Err(PrepareError::IntoMismatch { .. })));
    Ok(())
}

#[test]
fn into_colliding_with_a_source_package_file_is_rejected() -> TestResult {
    let lock =
        fs::read_to_string(fixture("conda_source").join("pixi.lock"))?.replace("@ app", "@ ../ws2");
    let repo = repo_with(&[
        ("ws/pixi.toml", ""),
        ("ws/pixi.lock", &lock),
        ("ws2/pixi.toml", "[package]"),
    ])?;
    let out = tempfile::tempdir()?;
    let result = prepare(
        repo.path(),
        Path::new("ws/pixi.toml"),
        &out.path().join("r"),
        &into("ws2")?,
    );
    assert!(matches!(result, Err(PrepareError::Collision(_))));
    Ok(())
}

#[test]
fn into_keeps_the_manifest_file_name() -> TestResult {
    let lock =
        fs::read_to_string(fixture("conda_source").join("pixi.lock"))?.replace("@ app", "@ ../app");
    let repo = repo_with(&[
        ("ws/pyproject.toml", ""),
        ("ws/pixi.lock", &lock),
        ("app/package.xml", ""),
    ])?;
    let files = prepare_in(&repo, "ws/pyproject.toml", &into("other")?)?;
    assert!(files.contains(&PathBuf::from("other/pyproject.toml")));
    assert!(!files.contains(&PathBuf::from("other/pixi.toml")));
    Ok(())
}

#[test]
fn include_does_not_cross_directories_with_a_star() -> TestResult {
    insta::assert_snapshot!(recipe_with(
        "variants",
        "variants/base/pixi.toml",
        &including_options(&["*.cmake"])?
    )?);
    Ok(())
}

#[test]
fn include_directory_pattern_matches_its_contents() -> TestResult {
    let repo = app_repo(&[("app/cmake/a.txt", ""), ("app/cmake/sub/b.txt", "")])?;
    let files = prepare_in(&repo, "pixi.toml", &including_options(&["cmake"])?)?;
    assert!(files.contains(&PathBuf::from("app/cmake/a.txt")));
    assert!(files.contains(&PathBuf::from("app/cmake/sub/b.txt")));
    Ok(())
}

#[test]
fn include_skips_gitignored_files() -> TestResult {
    let repo = app_repo(&[
        ("app/.gitignore", "build/\n"),
        ("app/build/out.txt", ""),
        ("app/keep.txt", ""),
    ])?;
    let files = prepare_in(&repo, "pixi.toml", &including_options(&["**/*.txt"])?)?;
    assert!(files.contains(&PathBuf::from("app/keep.txt")));
    assert!(!files.contains(&PathBuf::from("app/build/out.txt")));
    Ok(())
}

#[cfg(unix)]
#[test]
fn include_never_follows_symlinks() -> TestResult {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("secret.txt"), "")?;
    let repo = app_repo(&[("app/real.txt", "")])?;
    let app = repo.path().join("app");
    symlink(outside.path(), app.join("escape"))?;
    symlink(".", app.join("loop"))?;
    symlink("missing.txt", app.join("broken.txt"))?;
    let files = prepare_in(&repo, "pixi.toml", &including_options(&["**/*.txt"])?)?;
    assert!(files.contains(&PathBuf::from("app/real.txt")));
    assert!(!files.iter().any(|f| f.to_string_lossy().contains("secret")));
    assert!(!files.contains(&PathBuf::from("app/broken.txt")));
    Ok(())
}

#[test]
fn include_matching_nothing_is_rejected() -> TestResult {
    let repo = app_repo(&[])?;
    let out = tempfile::tempdir()?;
    let result = prepare(
        repo.path(),
        Path::new("pixi.toml"),
        &out.path().join("r"),
        &including_options(&["nothing/*.txt"])?,
    );
    assert!(matches!(result, Err(PrepareError::UnmatchedInclude(p)) if p == "nothing/*.txt"));
    Ok(())
}

#[test]
fn include_rejects_absolute_and_parent_patterns() {
    assert!(matches!(
        "/etc/passwd".parse::<IncludeGlob>(),
        Err(IncludeGlobError::Absolute(_))
    ));
    assert!(matches!(
        "../x/*.txt".parse::<IncludeGlob>(),
        Err(IncludeGlobError::ParentDir(_))
    ));
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
