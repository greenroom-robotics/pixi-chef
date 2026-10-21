use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr};

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
        /// Directory, a sibling of the manifest's directory, to place the
        /// manifest and lock at in the recipe.
        #[arg(long)]
        into: Option<PathBuf>,
        /// Glob of extra files to copy from each source package directory.
        #[arg(long)]
        include: Vec<IncludeGlob>,
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
                let workspace = Workspace::new(&manifest, into.as_deref(), &root)?;
                prepare(&root, &workspace, &out, &include)?;
                Ok(())
            }
        }
    }
}
