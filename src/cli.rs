use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr};

use crate::bake::{Pixi, bake};
use crate::cook::{Termination, cook};
use crate::include::IncludeGlob;
use crate::prepare::{Workspace, prepare};

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Write the recipe for a workspace: its manifest, lock and the metadata
    /// files of every source package, at their real paths.
    Prepare {
        /// Workspace manifest, inside the current directory.
        manifest: PathBuf,
        /// Directory to write the recipe into; must be absent or empty.
        out: PathBuf,
        /// Directory to place the manifest and lock at in the recipe; every
        /// local path in the lock must resolve the same from there as from
        /// the manifest's directory.
        #[arg(long)]
        into: Option<PathBuf>,
        /// Glob of extra files to copy from each source package directory.
        #[arg(long)]
        include: Vec<IncludeGlob>,
    },
    /// Install an environment without its source packages, using the lock as is.
    Cook {
        /// Environment to install.
        #[arg(short, long)]
        environment: String,
    },
    /// Install an environment from the lock in the current directory and
    /// write an export of it, with every source package, under a directory.
    Bake {
        /// Environment to install and export.
        #[arg(short, long)]
        env: String,
        /// Directory to write the export into; must be absent or empty.
        #[arg(long)]
        export: PathBuf,
    },
}

impl Cli {
    pub fn run(self) -> Result<ExitCode> {
        match self.command {
            Command::Prepare {
                manifest,
                out,
                into,
                include,
            } => {
                let root = std::env::current_dir().wrap_err("reading current directory")?;
                let workspace = Workspace::new(&manifest, into.as_deref(), &root)?;
                prepare(&root, &workspace, &out, &include)?;
                Ok(ExitCode::SUCCESS)
            }
            Command::Cook { environment } => {
                let root = std::env::current_dir().wrap_err("reading current directory")?;
                let status = cook(&root, &environment)?;
                Ok(ExitCode::from(Termination::from(status).exit_byte()))
            }
            Command::Bake { env, export } => {
                let root = std::env::current_dir().wrap_err("reading current directory")?;
                bake(&Pixi::default(), &root, &env, &export)?;
                Ok(ExitCode::SUCCESS)
            }
        }
    }
}
