use anyhow::{Context, Result};
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
    /// Run as if Shadow was started in this directory
    #[arg(short = 'C', global = true, value_name = "PATH", default_value = ".")]
    directory: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize Shadow in the current Git repository
    Init,
    /// Show managed worktree changes and optionally remote issues
    Status {
        #[arg(long)]
        remote: bool,
    },
    /// Publish worktree content and create or update refs
    Publish,
    /// Restore worktree files from refs
    Restore {
        #[arg(long)]
        force: bool,
    },
    /// Check local invariants and optionally remote objects
    Check {
        #[arg(long)]
        remote: bool,
    },
    /// Find and optionally delete remote objects unreferenced by Git history
    Gc {
        /// Delete candidates instead of only reporting them
        #[arg(long)]
        delete: bool,
        /// Keep unreferenced objects newer than this many days
        #[arg(long, default_value_t = 30)]
        grace_days: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    std::env::set_current_dir(&cli.directory)
        .with_context(|| format!("failed to enter {}", cli.directory.display()))?;
    match cli.command {
        Command::Init => commands::init::run(),
        Command::Status { remote } => commands::status::run(remote).await,
        Command::Publish => commands::publish::run().await,
        Command::Restore { force } => commands::restore::run(force).await,
        Command::Check { remote } => commands::check::run(remote).await,
        Command::Gc { delete, grace_days } => commands::gc::run(delete, grace_days).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_global_directory() {
        let cli = Cli::try_parse_from(["shadow", "-C", "repo", "status"]).unwrap();
        assert_eq!(cli.directory, PathBuf::from("repo"));
        assert!(matches!(cli.command, Command::Status { remote: false }));
    }

    #[test]
    fn rejects_removed_commands_and_path_filters() {
        assert!(Cli::try_parse_from(["shadow", "verify"]).is_err());
        assert!(Cli::try_parse_from(["shadow", "remove", "asset.bin"]).is_err());
        assert!(Cli::try_parse_from(["shadow", "status", "asset.bin"]).is_err());
    }

    #[test]
    fn parses_gc_safety_defaults() {
        let cli = Cli::try_parse_from(["shadow", "gc"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Gc {
                delete: false,
                grace_days: 30
            }
        ));

        let cli = Cli::try_parse_from(["shadow", "gc", "--delete", "--grace-days", "7"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Gc {
                delete: true,
                grace_days: 7
            }
        ));
    }
}
