use crate::config::Config;
use crate::remote::RemoteClient;
use crate::utils::{compute_sha256, find_project_root};
use anyhow::{Context, Result};
use glob::glob;
use std::fs;

pub async fn run() -> Result<()> {
    // 0. Find Root
    let root = find_project_root()?;
    std::env::set_current_dir(&root).context("Failed to change CWD to project root")?;

    // 1. Load Config (Default remote "origin")
    let remote_name = "origin";
    
    let config = Config::load()?;
    let remote_config = config.get_remote(remote_name)
        .ok_or_else(|| anyhow::anyhow!("Remote '{}' not found in config", remote_name))?;

    let client = RemoteClient::new(remote_config)?;
    println!("Pulling from remote: {}", remote_name);

    // 2. Find .shadow files
    let pattern = "**/*.shadow";
    let paths = glob(pattern).context("Failed to read glob pattern")?;

    for entry in paths {
        let shadow_path = entry?;
        if !shadow_path.is_file() { continue; }

        let pointer_content = fs::read_to_string(&shadow_path)
            .with_context(|| format!("Failed to read {:?}", shadow_path))?;
        let hash = pointer_content.trim();
        let full_hash = if !hash.starts_with("sha256:") {
            format!("sha256:{}", hash)
        } else {
            hash.to_string()
        };

        let source_path = shadow_path.with_extension("");

        // 3. Check if we need to download
        let need_download = if !source_path.exists() {
            true
        } else {
            // Check hash
            let current_hash = match compute_sha256(&source_path) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("Failed to hash local file {:?}: {}", source_path, e);
                    continue;
                }
            };
            format!("sha256:{}", current_hash) != full_hash
        };

        if !need_download {
            // println!("  [Up-to-date] {}", source_path.display());
            continue;
        }

        println!("  [Downloading] {}...", source_path.display());
        match client.download_file(&full_hash, &source_path).await {
            Ok(_) => println!("    -> Done"),
            Err(e) => eprintln!("    -> Failed: {}", e),
        }
    }

    Ok(())
}
