use crate::config::Config;
use crate::remote::RemoteClient;
use crate::stage::StagingIndex;
use crate::utils::{add_to_gitignore, compute_sha256, find_project_root};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

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

    // 3. Load Staging Index
    let mut index = StagingIndex::load(&root)?;
    if index.entries.is_empty() {
        println!("Nothing to push (Staging area is empty).");
        return Ok(());
    }

    // 4. Process Staged Entries
    let mut pushed_paths = Vec::new();

    // Iterate over a clone of keys/values to allow modification of index later
    // or just collect success keys.
    for (rel_path_str, hash) in &index.entries {
        let path = Path::new(rel_path_str);
        
        // Safety Check: Does file match hash?
        if !path.exists() {
            eprintln!("  [Error]   {} (File missing, cannot push)", rel_path_str);
            continue;
        }

        // Optional: Re-hash check?
        // If file changed, we are pushing 'current file' under 'staged hash'. 
        // This is dangerous if content doesn't match hash.
        // User expects 'git add' snapshot behavior. But we don't have snapshot.
        // So we MUST verify hash.
        match compute_sha256(path) {
            Ok(current_hash_raw) => {
                let current_hash = format!("sha256:{}", current_hash_raw);
                if current_hash != *hash {
                    eprintln!("  [Error]   {} (Content modified since add. Please add again.)", rel_path_str);
                    continue;
                }
            },
            Err(e) => {
                eprintln!("  [Error]   {} (Failed to read: {})", rel_path_str, e);
                continue;
            }
        }

        // Check if exists on remote
        if client.exists(hash).await? {
            println!("  [Skipped] {} (Already exists)", rel_path_str);
        } else {
            println!("  [Uploading] {}...", rel_path_str);
            if let Err(e) = client.upload_file(hash, path).await {
                eprintln!("    -> Failed: {}", e);
                continue;
            }
            println!("    -> Done");
        }

        pushed_paths.push(rel_path_str.clone());
    }

    // 5. Finalize Pushed Items
    for path_str in pushed_paths {
        // Create Pointer
        let path = Path::new(&path_str);
        let hash = index.entries.get(&path_str).unwrap(); // Must exist
        let pointer_path_str = format!("{}.shadow", path_str);
        let pointer_path = Path::new(&pointer_path_str);
        
        if let Err(e) = fs::write(pointer_path, hash) {
            eprintln!("  [Error] Failed to create pointer {}: {}", pointer_path_str, e);
            continue;
        }
        
        // Add to gitignore
        if config.core.auto_add_to_gitignore {
            if let Err(e) = add_to_gitignore(&root, &path_str) {
                eprintln!("  [Error] Failed to update gitignore for {}: {}", path_str, e);
            }
        }

        // Remove from index
        index.remove(&path_str);
        println!("  [Shadowed] {}", path_str);
    }

    // 6. Save Index
    index.save(&root)?;

    Ok(())
}
