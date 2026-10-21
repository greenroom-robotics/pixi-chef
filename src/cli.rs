use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr};

use crate::bake::{Pixi, bake};
use crate::prepare::prepare;

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
    pub fn run(self) -> Result<()> {
        let root = std::env::current_dir().wrap_err("reading current directory")?;
        match self.command {
            Command::Prepare { manifest, out } => prepare(&root, &manifest, &out)?,
            Command::Bake { env, export } => bake(&Pixi::default(), &root, &env, &export)?,
        }
        Ok(())
    }
}
