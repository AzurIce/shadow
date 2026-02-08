use clap::{Parser, Subcommand};
use anyhow::Result;

mod init;
mod config;
mod object;
mod utils;
mod add;
mod status;
mod push;
mod remote;
mod pull;
mod stage;

#[derive(Parser)]
#[command(name = "git-shadow")]
#[command(about = "A lightweight large file management tool for Git", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new shadow repository
    Init,
    
    /// Track files and create shadow pointers
    Add {
        /// Files to add
        #[arg(required = true)]
        files: Vec<String>,
    },

    /// Show status of shadowed files
    Status,

    /// Upload shadowed files to remote storage
    Push {
        /// Remote name (default: origin)
        #[arg(default_value = "origin")]
        remote: String,
    },

    /// Download shadowed files from remote storage
    Pull,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Init => {
            init::run().await?;
        }
        Commands::Add { files } => {
            add::run(files.clone()).await?;
        }
        Commands::Status => {
            status::run().await?;
        }
        Commands::Push { remote } => {
            push::run(remote.clone()).await?;
        }
        Commands::Pull => {
            pull::run().await?;
        }
    }

    Ok(())
}
