use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The pixi executable that `bake` and `cook` drive.
#[derive(Debug, Clone)]
pub struct Pixi(OsString);

impl Pixi {
    pub fn at(program: impl Into<OsString>) -> Self {
        Self(program.into())
    }

    pub fn command(&self, workspace: &Path) -> Command {
        let mut command = Command::new(&self.0);
        command.current_dir(workspace);
        command
    }

    pub fn describe(&self, args: &[&str]) -> String {
        format!("{} {}", self.0.to_string_lossy(), args.join(" "))
    }

    pub fn resolved(&self) -> PathBuf {
        let program = Path::new(&self.0);
        if program.components().count() > 1 {
            return program.to_path_buf();
        }
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .map(|dir| dir.join(program))
            .find(|candidate| is_executable(candidate))
            .unwrap_or_else(|| program.to_path_buf())
    }
}

impl Default for Pixi {
    fn default() -> Self {
        Self::at("pixi")
    }
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
