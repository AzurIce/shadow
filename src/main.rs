use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use shadow::commands;

#[derive(Debug, Parser)]
#[command(
    name = "shadow",
    version,
    about = "Explicit large-file storage for Git repositories"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize Shadow in the current Git repository
    Init,
    /// Compare managed worktree files, refs, cache, and optionally the remote
    Status {
        paths: Vec<PathBuf>,
        #[arg(long)]
        remote: bool,
    },
    /// Publish worktree content and create or update refs
    Publish { paths: Vec<PathBuf> },
    /// Restore worktree files from refs
    Restore {
        paths: Vec<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Remove refs while keeping worktree and remote objects
    Remove { paths: Vec<PathBuf> },
    /// Verify local invariants and optionally remote objects
    Verify {
        #[arg(long)]
        remote: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => commands::init::run(),
        Command::Status { paths, remote } => commands::status::run(paths, remote).await,
        Command::Publish { paths } => commands::publish::run(paths).await,
        Command::Restore { paths, force } => commands::restore::run(paths, force).await,
        Command::Remove { paths } => commands::remove::run(paths),
        Command::Verify { remote } => commands::verify::run(remote).await,
    }
}
