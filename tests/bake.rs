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
    Ok(dir)
}

fn fake_pixi(dir: &Path, hook: &str) -> Result<Pixi, Box<dyn Error>> {
    let script = dir.join("fake-pixi");
    fs::write(
        &script,
        format!(
            r#"#!/bin/sh
case "$1" in
install)
    mkdir -p .pixi/envs/default/bin .pixi/task-cache
    printf '#!/bin/sh\necho "$PIXI_ENV_MARK"\n' > .pixi/envs/default/bin/hello
    chmod +x .pixi/envs/default/bin/hello
    echo stale > .pixi/stray-file
    ;;
shell-hook)
    printf '%s' '{hook}'
    ;;
esac
"#
        ),
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    Ok(Pixi::at(script))
}

const HOOK: &str = "export PIXI_ENV_MARK=active\n";

#[test]
fn leaves_only_envs_in_pixi_dir() -> TestResult {
    let ws = workspace()?;
    let tools = tempfile::tempdir()?;
    let out = tempfile::tempdir()?;
    bake(
        &fake_pixi(tools.path(), HOOK)?,
        ws.path(),
        "default",
        &out.path().join("export"),
    )?;
    let names: Vec<_> = fs::read_dir(ws.path().join(".pixi"))?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<_, _>>()?;
    assert_eq!(names, ["envs"]);
    Ok(())
}

#[test]
fn empty_hook_is_a_failure() -> TestResult {
    let ws = workspace()?;
    let tools = tempfile::tempdir()?;
    let out = tempfile::tempdir()?;
    let result = bake(
        &fake_pixi(tools.path(), " \n")?,
        ws.path(),
        "default",
        &out.path().join("export"),
    );
    assert!(matches!(result, Err(BakeError::EmptyHook(_))));
    Ok(())
}

#[test]
fn fails_when_export_is_not_empty() -> TestResult {
    let ws = workspace()?;
    let tools = tempfile::tempdir()?;
    let out = tempfile::tempdir()?;
    fs::write(out.path().join("stale"), "")?;
    let result = bake(
        &fake_pixi(tools.path(), HOOK)?,
        ws.path(),
        "default",
        out.path(),
    );
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
fn export_holds_env_sources_hook_and_entrypoint_at_real_paths() -> TestResult {
    let ws = workspace()?;
    let tools = tempfile::tempdir()?;
    let out = tempfile::tempdir()?;
    let export = out.path().join("export");
    bake(
        &fake_pixi(tools.path(), HOOK)?,
        ws.path(),
        "default",
        &export,
    )?;

    let real = ws.path().canonicalize()?;
    let rooted = |p: &Path| export.join(p.strip_prefix("/").unwrap_or(p));
    assert!(rooted(&real.join(".pixi/envs/default/bin/hello")).is_file());
    assert!(!rooted(&real.join(".pixi/task-cache")).exists());
    assert!(rooted(&real.join("app/package.xml")).is_file());
    assert!(rooted(&real.join("app/src/main.cpp")).is_file());
    assert_eq!(
        fs::read_to_string(export.join("etc/profile.d/pixi-env.sh"))?,
        HOOK
    );
    let entrypoint = fs::read_to_string(export.join("entrypoint.sh"))?;
    assert_eq!(
        entrypoint,
        "#!/bin/sh\n. /etc/profile.d/pixi-env.sh\nexec \"$@\"\n"
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
fn entrypoint_runs_a_command_with_the_environment_active() -> TestResult {
    let ws = workspace()?;
    let tools = tempfile::tempdir()?;
    let out = tempfile::tempdir()?;
    let export = out.path().join("export");
    bake(
        &fake_pixi(tools.path(), HOOK)?,
        ws.path(),
        "default",
        &export,
    )?;

    let hook = export.join("etc/profile.d/pixi-env.sh");
    let entrypoint = fs::read_to_string(export.join("entrypoint.sh"))?
        .replace("/etc/profile.d/pixi-env.sh", &hook.display().to_string());
    let script = tools.path().join("entrypoint.sh");
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
