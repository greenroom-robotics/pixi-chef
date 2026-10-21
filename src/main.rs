use clap::Parser;
use color_eyre::eyre::Result;
use pixi_chef::cli::Cli;

fn main() -> Result<()> {
    color_eyre::install()?;
    Cli::parse().run()
}
