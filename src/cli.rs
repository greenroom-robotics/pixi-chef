use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr};

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
        /// Workspace manifest, relative to the current directory.
        manifest: PathBuf,
        /// Directory to write the recipe into; must be absent or empty.
        out: PathBuf,
    },
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Prepare { manifest, out } => {
                let root = std::env::current_dir().wrap_err("reading current directory")?;
                prepare(&root, &manifest, &out)?;
                Ok(())
            }
        }
    }
}
