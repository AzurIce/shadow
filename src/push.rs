use crate::config::Config;
use crate::remote::RemoteClient;
use crate::utils::find_project_root;
use anyhow::{Context, Result};
use glob::glob;
use std::fs;

pub async fn run(remote_name: String) -> Result<()> {
    // 0. Find Root and switch CWD
    let root = find_project_root()?;
    std::env::set_current_dir(&root).context("Failed to change CWD to project root")?;

    // 1. Load Config
    let config = Config::load()?;
    let remote_config = config.get_remote(&remote_name)
        .ok_or_else(|| anyhow::anyhow!("Remote '{}' not found in config", remote_name))?;

    // 2. Initialize Remote Client
    let client = RemoteClient::new(remote_config)?;
    println!("Pushing to remote: {}", remote_name);

    // 3. Find all .shadow files
    let pattern = "**/*.shadow";
    let paths = glob(pattern).context("Failed to read glob pattern")?;

    for entry in paths {
        let shadow_path = entry?;
        if !shadow_path.is_file() { continue; }

        let pointer_content = fs::read_to_string(&shadow_path)
            .with_context(|| format!("Failed to read {:?}", shadow_path))?;
        let hash = pointer_content.trim();

        // 4. Check if exists on remote
        if client.exists(hash).await? {
            println!("  [Skipped] {} (Already exists)", shadow_path.display());
            continue;
        }

        // 5. Upload
        // Determine source path: remove .shadow extension
        let source_path = shadow_path.with_extension(""); // This removes the last extension. .shadow -> ""
        
        if !source_path.exists() {
            eprintln!("  [Error]   {} (Source file missing, cannot upload)", source_path.display());
            continue;
        }

        println!("  [Uploading] {}...", source_path.display());
        client.upload_file(hash, &source_path).await?;
        println!("    -> Done");
    }

    Ok(())
}
