use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr};

use crate::cook::{Termination, cook};
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
    /// Install an environment without its source packages, using the lock as is.
    Cook {
        /// Environment to install.
        #[arg(short, long)]
        environment: String,
    },
}

impl Cli {
    pub fn run(self) -> Result<ExitCode> {
        match self.command {
            Command::Prepare { manifest, out } => {
                let root = std::env::current_dir().wrap_err("reading current directory")?;
                prepare(&root, &manifest, &out)?;
                Ok(ExitCode::SUCCESS)
            }
            Command::Cook { environment } => {
                let root = std::env::current_dir().wrap_err("reading current directory")?;
                let status = cook(&root, &environment)?;
                Ok(ExitCode::from(Termination::from(status).exit_byte()))
            }
        }
    }
}
