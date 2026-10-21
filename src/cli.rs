use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr};
use globset::Glob;

use crate::prepare::{PrepareOptions, prepare};

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
        /// Directory, inside the current directory, to place the manifest and
        /// lock at in the recipe. Source package paths resolve from here.
        #[arg(long)]
        into: Option<PathBuf>,
        /// Glob of extra files to copy from each source package directory.
        #[arg(long)]
        include: Vec<Glob>,
    },
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Prepare {
                manifest,
                out,
                into,
                include,
            } => {
                let root = std::env::current_dir().wrap_err("reading current directory")?;
                prepare(&root, &manifest, &out, &PrepareOptions { into, include })?;
                Ok(())
            }
        }
    }
}
