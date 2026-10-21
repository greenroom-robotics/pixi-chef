#![allow(clippy::panic_in_result_fn)]

use std::error::Error;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use pixi_chef::bake::{BakeError, Pixi, bake};

type TestResult = Result<(), Box<dyn Error>>;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn copy_dir(from: &Path, to: &Path) -> TestResult {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn workspace() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    copy_dir(&fixture("bake"), dir.path())?;
    fs::write(dir.path().join("act.sh"), "export ACT=1\n")?;
    Ok(dir)
}

struct Fake {
    pixi: Pixi,
    _dir: tempfile::TempDir,
}

fn fake_pixi(with_bash: bool, hook_tail: &str) -> Result<Fake, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let script = dir.path().join("fake-pixi");
    let bash = if with_bash {
        "ln -s /bin/sh .pixi/envs/default/bin/bash"
    } else {
        ":"
    };
    fs::write(
        &script,
        format!(
            r#"#!/bin/sh
case "$1" in
install)
    mkdir -p .pixi/envs/default/bin .pixi/task-cache
    printf '#!/bin/sh\necho "$PIXI_ENV_MARK"\n' > .pixi/envs/default/bin/hello
    chmod +x .pixi/envs/default/bin/hello
    {bash}
    ;;
shell-hook)
    [ "$*" = "shell-hook --as-is --shell bash --frozen -e default" ] || exit 9
    echo ignored > .pixi/.gitignore
    printf 'export PATH="%s/.pixi/envs/default/bin:%s/tools:%s"\n' "$PWD" "$PWD" "$PATH"
    printf 'export PIXI_EXE=/host/pixi\n'
    printf '{hook_tail}'
    printf '. %s/act.sh\n' "$PWD"
    ;;
esac
"#
        ),
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    Ok(Fake {
        pixi: Pixi::at(script),
        _dir: dir,
    })
}

const HOOK_TAIL: &str = "export ZED=1\\nexport PIXI_ENV_MARK=active\\n";

fn run_bake(ws: &Path, fake: &Fake, export: &Path) -> Result<(), BakeError> {
    bake(&fake.pixi, ws, "default", export)
}

#[test]
fn leaves_only_envs_in_pixi_dir() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    run_bake(
        ws.path(),
        &fake_pixi(true, HOOK_TAIL)?,
        &out.path().join("export"),
    )?;
    let names: Vec<_> = fs::read_dir(ws.path().join(".pixi"))?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<_, _>>()?;
    assert_eq!(names, ["envs"]);
    Ok(())
}

#[test]
fn empty_hook_is_a_failure_and_leaves_no_export() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    let export = out.path().join("export");
    let dir = tempfile::tempdir()?;
    let script = dir.path().join("pixi");
    fs::write(
        &script,
        "#!/bin/sh\n[ \"$1\" = install ] && mkdir -p .pixi/envs/default/bin && ln -s /bin/sh .pixi/envs/default/bin/bash\nexit 0\n",
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    let result = bake(&Pixi::at(script), ws.path(), "default", &export);
    assert!(matches!(result, Err(BakeError::EmptyHook(_))));
    assert_eq!(fs::read_dir(out.path())?.count(), 0);
    Ok(())
}

#[test]
fn missing_bash_is_a_failure_naming_the_fix() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    let result = run_bake(
        ws.path(),
        &fake_pixi(false, HOOK_TAIL)?,
        &out.path().join("export"),
    );
    let error = result.expect_err("bake must fail");
    assert!(matches!(error, BakeError::MissingBash(_)));
    assert!(error.to_string().contains("add `bash`"));
    assert_eq!(fs::read_dir(out.path())?.count(), 0);
    Ok(())
}

#[test]
fn fails_when_export_is_not_empty() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    fs::write(out.path().join("stale"), "")?;
    let result = run_bake(ws.path(), &fake_pixi(true, HOOK_TAIL)?, out.path());
    assert!(matches!(result, Err(BakeError::ExportNotEmpty(_))));
    Ok(())
}

#[test]
fn fails_when_pixi_fails() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    let result = bake(
        &Pixi::at("false"),
        ws.path(),
        "default",
        &out.path().join("export"),
    );
    assert!(matches!(result, Err(BakeError::CommandFailed { .. })));
    Ok(())
}

#[test]
fn fails_for_an_environment_missing_from_the_lock() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    let result = bake(
        &fake_pixi(true, HOOK_TAIL)?.pixi,
        ws.path(),
        "nope",
        &out.path().join("export"),
    );
    assert!(matches!(result, Err(BakeError::UnknownEnvironment(_))));
    Ok(())
}

#[test]
fn export_holds_env_sources_hook_and_entrypoint_at_real_paths() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    let export = out.path().join("export");
    run_bake(ws.path(), &fake_pixi(true, HOOK_TAIL)?, &export)?;

    let real = ws.path().canonicalize()?;
    let rooted = |p: &Path| export.join(p.strip_prefix("/").unwrap_or(p));
    assert!(rooted(&real.join(".pixi/envs/default/bin/hello")).is_file());
    assert!(!rooted(&real.join(".pixi/task-cache")).exists());
    assert!(rooted(&real.join("app/package.xml")).is_file());
    assert!(rooted(&real.join("app/src/main.cpp")).is_file());
    assert!(rooted(&real.join("act.sh")).is_file());
    let prefix = real.join(".pixi/envs/default");
    assert_eq!(
        fs::read_to_string(export.join("etc/profile.d/pixi-env.sh"))?,
        format!(
            "export PATH=\"{}/bin:{}/tools:$PATH\"\nexport ZED=1\nexport PIXI_ENV_MARK=active\n. {}/act.sh\n",
            prefix.display(),
            real.display(),
            real.display()
        )
    );
    assert_eq!(
        fs::read_to_string(export.join("entrypoint.sh"))?,
        format!(
            "#!/bin/sh\nexec '{}/bin/bash' -c '. /etc/profile.d/pixi-env.sh && exec \"$@\"' bash \"$@\"\n",
            prefix.display()
        )
    );
    assert_eq!(
        fs::metadata(export.join("entrypoint.sh"))?
            .permissions()
            .mode()
            & 0o111,
        0o111
    );
    Ok(())
}

#[test]
fn leaves_unrelated_sibling_directories_alone_and_fills_an_empty_export() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    let export = out.path().join("export");
    let sibling = out.path().join(".export.partial");
    fs::create_dir_all(&sibling)?;
    fs::write(sibling.join("mine"), "")?;
    fs::create_dir_all(&export)?;
    run_bake(ws.path(), &fake_pixi(true, HOOK_TAIL)?, &export)?;
    assert!(sibling.join("mine").is_file());
    assert!(export.join("entrypoint.sh").is_file());
    Ok(())
}

#[test]
fn sourced_file_that_does_not_exist_is_a_failure() -> TestResult {
    let ws = workspace()?;
    fs::remove_file(ws.path().join("act.sh"))?;
    let out = tempfile::tempdir()?;
    let result = run_bake(
        ws.path(),
        &fake_pixi(true, HOOK_TAIL)?,
        &out.path().join("export"),
    );
    assert!(matches!(result, Err(BakeError::MissingSourcedFile(_))));
    assert_eq!(fs::read_dir(out.path())?.count(), 0);
    Ok(())
}

#[test]
fn root_source_package_does_not_pull_in_other_environments() -> TestResult {
    let ws = tempfile::tempdir()?;
    fs::write(ws.path().join("pixi.toml"), "")?;
    fs::write(ws.path().join("package.xml"), "<package/>")?;
    fs::write(ws.path().join("act.sh"), "")?;
    let lock = fs::read_to_string(fixture("bake").join("pixi.lock"))?.replace("@ app", "@ .");
    fs::write(ws.path().join("pixi.lock"), lock)?;
    fs::create_dir_all(ws.path().join(".pixi/envs/other"))?;
    fs::write(ws.path().join(".pixi/envs/other/leak"), "")?;
    let out = tempfile::tempdir()?;
    let export = out.path().join("export");
    run_bake(ws.path(), &fake_pixi(true, HOOK_TAIL)?, &export)?;

    let real = ws.path().canonicalize()?;
    let rooted = |p: &Path| export.join(p.strip_prefix("/").unwrap_or(p));
    assert!(rooted(&real.join("package.xml")).is_file());
    assert!(rooted(&real.join(".pixi/envs/default/bin/hello")).is_file());
    assert!(!rooted(&real.join(".pixi/envs/other")).exists());
    Ok(())
}

#[test]
fn entrypoint_runs_a_command_with_the_environment_active() -> TestResult {
    let ws = workspace()?;
    let out = tempfile::tempdir()?;
    let export = out.path().join("export");
    run_bake(ws.path(), &fake_pixi(true, HOOK_TAIL)?, &export)?;

    let hook = export.join("etc/profile.d/pixi-env.sh");
    let entrypoint = fs::read_to_string(export.join("entrypoint.sh"))?
        .replace("/etc/profile.d/pixi-env.sh", &hook.display().to_string());
    let script = out.path().join("entrypoint.sh");
    fs::write(&script, entrypoint)?;
    let output = Command::new("/bin/sh")
        .arg(&script)
        .arg("sh")
        .arg("-c")
        .arg("echo $PIXI_ENV_MARK")
        .env_clear()
        .output()?;
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout)?, "active\n");
    Ok(())
}

#[test]
fn real_pixi_export_runs_through_the_entrypoint_without_system_bash() -> TestResult {
    let ws = tempfile::tempdir()?;
    copy_dir(&fixture("bake_e2e"), ws.path())?;
    let out = tempfile::tempdir()?;
    let export = out.path().join("export");
    bake(&Pixi::default(), ws.path(), "default", &export)?;
    let names: Vec<_> = fs::read_dir(ws.path().join(".pixi"))?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<_, _>>()?;
    assert_eq!(names, ["envs"]);

    let hook = export.join("etc/profile.d/pixi-env.sh");
    let entrypoint = fs::read_to_string(export.join("entrypoint.sh"))?
        .replace("/etc/profile.d/pixi-env.sh", &hook.display().to_string());
    let script = out.path().join("entrypoint.sh");
    fs::write(&script, entrypoint)?;
    let output = Command::new("/bin/sh")
        .arg(&script)
        .args(["/bin/sh", "-c", "echo $BAKE_E2E_MARK; echo $AA; echo $PATH"])
        .env_clear()
        .env("PATH", "/runtime/bin")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("active"));
    assert_eq!(lines.next(), Some("bar baz/aa"));
    let path = lines.next().unwrap_or_default();
    let entries: Vec<&str> = path.split(':').collect();
    let root = ws.path().canonicalize()?;
    let tools = root.join("tools");
    let position = |suffix: &Path| entries.iter().position(|e| Path::new(e) == suffix);
    let bin = root.join(".pixi/envs/default/bin");
    assert!(position(&tools) < position(&bin), "{path}");
    assert!(position(&tools).is_some(), "{path}");
    assert!(path.ends_with(":/runtime/bin"), "{path}");
    Ok(())
}
