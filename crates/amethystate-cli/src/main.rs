use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod app;
mod inspector;
mod report;
mod ui;

#[derive(Parser)]
#[command(name = "amethystate")]
enum Cli {
    Inspect { path: PathBuf },
}

fn main() -> Result<()> {
    let Cli::Inspect { path } = Cli::parse();
    let backend = inspector::open_inspector(&path)?;
    ui::run(backend)?;
    Ok(())
}
