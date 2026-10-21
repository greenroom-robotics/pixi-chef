use clap::Parser;
use color_eyre::eyre::Result;
use pixi_chef::cli::Cli;
use std::process::ExitCode;

fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    Cli::parse().run()
}
