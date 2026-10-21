#![allow(clippy::panic_in_result_fn)]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use pixi_chef::include::{IncludeGlob, IncludeGlobError};
use pixi_chef::prepare::{PrepareError, Workspace, WorkspaceError, prepare};

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
    options: &Opts,
) -> Result<String, Box<dyn Error>> {
    let out = tempfile::tempdir()?;
    let recipe_dir = out.path().join("recipe");
    run(&fixture(fixture_name), manifest, &recipe_dir, options)?;
    tree(&recipe_dir)
}

fn recipe(fixture_name: &str, manifest: &str) -> Result<String, Box<dyn Error>> {
    recipe_with(fixture_name, manifest, &Opts::default())
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

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Prepare(#[from] PrepareError),
}

#[derive(Default)]
struct Opts {
    into: Option<&'static str>,
    include: Vec<IncludeGlob>,
}

impl Opts {
    const fn into(dir: &'static str) -> Self {
        Self {
            into: Some(dir),
            include: Vec::new(),
        }
    }

    fn including(patterns: &[&str]) -> Result<Self, IncludeGlobError> {
        Ok(Self {
            into: None,
            include: patterns
                .iter()
                .map(|pattern| pattern.parse())
                .collect::<Result<_, _>>()?,
        })
    }
}

fn run(root: &Path, manifest: &str, out: &Path, opts: &Opts) -> Result<(), RunError> {
    let workspace = Workspace::new(Path::new(manifest), opts.into.map(Path::new), root)?;
    Ok(prepare(root, &workspace, out, &opts.include)?)
}

const fn into_shared() -> Opts {
    Opts::into("variants/shared")
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
    let options = Opts::including(&["cmake/*.cmake"])?;
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
    let result = run(
        &fixture("conda_source"),
        "pixi.toml",
        out.path(),
        &Opts::default(),
    );
    assert!(matches!(
        result,
        Err(RunError::Prepare(PrepareError::OutNotEmpty(_)))
    ));
    Ok(())
}

#[test]
fn accepts_existing_empty_out() -> TestResult {
    let out = tempfile::tempdir()?;
    run(
        &fixture("conda_source"),
        "pixi.toml",
        out.path(),
        &Opts::default(),
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
    let result = run(repo.path(), "pixi.toml", &recipe_dir, &Opts::default());
    assert!(
        matches!(result, Err(RunError::Prepare(PrepareError::MissingSourceDir(p))) if p == Path::new("app"))
    );
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
    options: &Opts,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let out = tempfile::tempdir()?;
    let recipe_dir = out.path().join("recipe");
    run(repo.path(), manifest, &recipe_dir, options)?;
    let mut files = Vec::new();
    collect_files(&recipe_dir, &mut files)?;
    files
        .iter()
        .map(|file| Ok(file.strip_prefix(&recipe_dir)?.to_path_buf()))
        .collect()
}

fn sibling_repo(lock_path: &str) -> Result<tempfile::TempDir, Box<dyn Error>> {
    let lock = fs::read_to_string(fixture("conda_source").join("pixi.lock"))?
        .replace("@ app", &format!("@ {lock_path}"));
    repo_with(&[
        ("ws/pixi.toml", ""),
        ("ws/pixi.lock", &lock),
        ("app/package.xml", ""),
        ("ws/sub/app/package.xml", ""),
    ])
}

fn into_sibling_error(lock_path: &str) -> Result<String, Box<dyn Error>> {
    let repo = sibling_repo(lock_path)?;
    let out = tempfile::tempdir()?;
    let result = run(
        repo.path(),
        "ws/pixi.toml",
        &out.path().join("r"),
        &Opts::into("shared"),
    );
    Ok(result.err().map(|e| e.to_string()).unwrap_or_default())
}

#[test]
fn into_is_rejected_when_a_source_path_stays_inside_the_workspace() -> TestResult {
    for lock_path in ["sub/app", ".", "./sub/app"] {
        let message = into_sibling_error(lock_path)?;
        assert!(
            message.contains("--into shared") && message.contains(lock_path),
            "{lock_path}: {message}"
        );
    }
    Ok(())
}

#[test]
fn into_is_rejected_for_a_local_binary_path() -> TestResult {
    let lock = fs::read_to_string(fixture("local_binary").join("pixi.lock"))?;
    let repo = repo_with(&[("ws/pixi.toml", ""), ("ws/pixi.lock", &lock)])?;
    let out = tempfile::tempdir()?;
    let result = run(
        repo.path(),
        "ws/pixi.toml",
        &out.path().join("r"),
        &Opts::into("shared"),
    );
    assert!(matches!(
        result,
        Err(RunError::Workspace(WorkspaceError::IntoChangesPath { .. }))
    ));
    Ok(())
}

#[test]
fn into_is_rejected_at_a_different_depth_for_a_local_binary_path() {
    let result = recipe_with("local_binary", "pixi.toml", &Opts::into("some/where/else"));
    assert!(matches!(
        result,
        Err(e) if e.to_string().contains("--into some/where/else")
    ));
}

#[test]
fn into_at_a_sibling_accepts_a_lock_with_only_parent_relative_paths() -> TestResult {
    let repo = sibling_repo("../app")?;
    let files = prepare_in(&repo, "ws/pixi.toml", &Opts::into("shared"))?;
    assert!(files.contains(&PathBuf::from("shared/pixi.lock")));
    assert!(files.contains(&PathBuf::from("app/package.xml")));
    Ok(())
}

#[test]
fn into_error_shows_the_root_manifest_dir_as_a_dot() {
    let result = recipe_with("conda_source", "pixi.toml", &Opts::into("elsewhere"));
    assert!(matches!(result, Err(e) if e.to_string().contains("than from .")));
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
    let result = run(
        repo.path(),
        "ws/pixi.toml",
        &out.path().join("r"),
        &Opts::into("ws2"),
    );
    assert!(matches!(
        result,
        Err(RunError::Prepare(PrepareError::Collision(_)))
    ));
    Ok(())
}

#[test]
fn manifest_not_named_pixi_toml_is_rejected() -> TestResult {
    let repo = repo_with(&[("ws/pyproject.toml", ""), ("ws/pixi.lock", "")])?;
    let out = tempfile::tempdir()?;
    let result = run(
        repo.path(),
        "ws/pyproject.toml",
        &out.path().join("r"),
        &Opts::default(),
    );
    assert!(matches!(
        result,
        Err(RunError::Workspace(WorkspaceError::NotPixiToml(_)))
    ));
    Ok(())
}

#[test]
fn include_does_not_cross_directories_with_a_star() -> TestResult {
    insta::assert_snapshot!(recipe_with(
        "variants",
        "variants/base/pixi.toml",
        &Opts::including(&["*.cmake"])?
    )?);
    Ok(())
}

#[test]
fn include_directory_pattern_matches_its_contents() -> TestResult {
    let repo = app_repo(&[("app/cmake/a.txt", ""), ("app/cmake/sub/b.txt", "")])?;
    let files = prepare_in(&repo, "pixi.toml", &Opts::including(&["cmake"])?)?;
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
    let files = prepare_in(&repo, "pixi.toml", &Opts::including(&["**/*.txt"])?)?;
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
    let files = prepare_in(&repo, "pixi.toml", &Opts::including(&["**/*.txt"])?)?;
    assert!(files.contains(&PathBuf::from("app/real.txt")));
    assert!(!files.iter().any(|f| f.to_string_lossy().contains("secret")));
    assert!(!files.contains(&PathBuf::from("app/broken.txt")));
    Ok(())
}

#[test]
fn include_matching_nothing_is_rejected() -> TestResult {
    let repo = app_repo(&[])?;
    let out = tempfile::tempdir()?;
    let result = run(
        repo.path(),
        "pixi.toml",
        &out.path().join("r"),
        &Opts::including(&["nothing/*.txt"])?,
    );
    assert!(
        matches!(result, Err(RunError::Prepare(PrepareError::UnmatchedInclude(p))) if p == "nothing/*.txt")
    );
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
    let result = run(
        repo.path(),
        "pixi.toml",
        &out.path().join("r"),
        &Opts::default(),
    );
    assert!(matches!(
        result,
        Err(RunError::Prepare(PrepareError::MissingManifest(_)))
    ));
    Ok(())
}

#[test]
fn fails_when_lock_is_missing() -> TestResult {
    let repo = repo_with(&[("pixi.toml", "")])?;
    let out = tempfile::tempdir()?;
    let result = run(
        repo.path(),
        "pixi.toml",
        &out.path().join("r"),
        &Opts::default(),
    );
    assert!(matches!(
        result,
        Err(RunError::Prepare(PrepareError::Lock(_)))
    ));
    Ok(())
}

#[test]
fn fails_when_source_path_escapes_root() -> TestResult {
    let lock =
        fs::read_to_string(fixture("conda_source").join("pixi.lock"))?.replace("@ app", "@ ../app");
    let repo = repo_with(&[("pixi.toml", ""), ("pixi.lock", &lock)])?;
    let out = tempfile::tempdir()?;
    let result = run(
        repo.path(),
        "pixi.toml",
        &out.path().join("r"),
        &Opts::default(),
    );
    assert!(matches!(
        result,
        Err(RunError::Prepare(PrepareError::Path(_)))
    ));
    Ok(())
}

#[test]
fn fails_when_out_is_a_file() -> TestResult {
    let out = tempfile::tempdir()?;
    let file = out.path().join("file");
    fs::write(&file, "")?;
    let result = run(
        &fixture("conda_source"),
        "pixi.toml",
        &file,
        &Opts::default(),
    );
    assert!(result.is_err());
    Ok(())
}

fn gitignore_fixture() -> Result<(tempfile::TempDir, PathBuf), Box<dyn Error>> {
    let outer = tempfile::tempdir()?;
    fs::write(outer.path().join(".gitignore"), "*.txt\n")?;
    fs::write(outer.path().join(".ignore"), "*.txt\n")?;
    let repo = outer.path().join("repo");
    let lock = fs::read_to_string(fixture("conda_source").join("pixi.lock"))?;
    fs::create_dir_all(repo.join("app"))?;
    fs::write(repo.join("pixi.toml"), "")?;
    fs::write(repo.join("pixi.lock"), lock)?;
    fs::write(repo.join("app/notes.txt"), "")?;
    fs::write(repo.join("app/.ignore"), "notes.txt\n")?;
    Ok((outer, repo))
}

#[test]
fn include_ignores_ignore_files_outside_the_repo_and_dot_ignore() -> TestResult {
    let (_outer, repo) = gitignore_fixture()?;
    let out = tempfile::tempdir()?;
    run(
        &repo,
        "pixi.toml",
        &out.path().join("r"),
        &Opts::including(&["*.txt"])?,
    )?;
    assert!(out.path().join("r/app/notes.txt").is_file());
    Ok(())
}

#[test]
fn include_honors_gitignore_between_the_root_and_the_package() -> TestResult {
    let repo = app_repo(&[
        (".gitignore", "root_ignored.txt\n"),
        ("app/root_ignored.txt", ""),
        ("app/kept.txt", ""),
    ])?;
    let files = prepare_in(&repo, "pixi.toml", &Opts::including(&["*.txt"])?)?;
    assert!(files.contains(&PathBuf::from("app/kept.txt")));
    assert!(!files.contains(&PathBuf::from("app/root_ignored.txt")));
    Ok(())
}

#[test]
fn include_ignores_the_global_git_excludes_file() -> TestResult {
    let (_outer, repo) = gitignore_fixture()?;
    let home = tempfile::tempdir()?;
    fs::create_dir_all(home.path().join("git"))?;
    fs::write(home.path().join("git/ignore"), "*.txt\n")?;
    let out = tempfile::tempdir()?;
    let recipe_dir = out.path().join("r");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_pixi-chef"))
        .args(["prepare", "pixi.toml"])
        .arg(&recipe_dir)
        .args(["--include", "*.txt"])
        .current_dir(&repo)
        .env("XDG_CONFIG_HOME", home.path())
        .env("HOME", home.path())
        .status()?;
    assert!(status.success());
    assert!(recipe_dir.join("app/notes.txt").is_file());
    Ok(())
}
