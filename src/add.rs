use crate::config::Config;
use crate::object::ObjectMetadata;
use crate::remote::RemoteClient;
use crate::utils::{add_to_gitignore, compute_sha256, get_metadata_path, find_project_root};
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::Path;

pub async fn run(files: Vec<String>) -> Result<()> {
    let root = find_project_root()?;
    let cwd = std::env::current_dir()?;

    // 1. Load config
    let config = Config::load().context("Failed to load config")?;
    
    // 2. Initialize Remote (Enforce "Upload First" policy)
    // We assume "origin" is the target. In future, allow flags.
    let remote_name = "origin";
    let remote_config = config.get_remote(remote_name)
        .ok_or_else(|| anyhow!("Remote '{}' not configured. 'shadow add' requires a configured remote to ensure upload.", remote_name))?;
    
    let client = RemoteClient::new(remote_config)?;

    for file_arg in files {
        let abs_path = cwd.join(&file_arg);
        
        if !abs_path.exists() {
            eprintln!("File not found: {}", file_arg);
            continue;
        }

        if abs_path.is_dir() {
            eprintln!("Skipping directory: {}. Use glob patterns or implement recursion later.", file_arg);
            continue;
        }

        // Calculate path relative to root for metadata and gitignore
        let rel_path = match abs_path.strip_prefix(&root) {
            Ok(p) => p.to_string_lossy().replace("\\", "/"),
            Err(_) => {
                eprintln!("File {} is outside the shadow repository root", file_arg);
                continue;
            }
        };

        println!("Processing: {}", rel_path);

        // 3. Compute Hash
        let raw_hash = compute_sha256(&abs_path).context("Failed to compute hash")?;
        let full_hash = format!("sha256:{}", raw_hash);
        let file_size = fs::metadata(&abs_path)?.len();

        // 4. Upload (Ensure object exists in remote BEFORE creating pointer)
        println!("  - Verifying remote status...");
        match client.exists(&full_hash).await {
            Ok(true) => {
                println!("  - Object already exists on remote. Skipping upload.");
            },
            Ok(false) => {
                println!("  - Object missing on remote. Uploading...");
                if let Err(e) = client.upload_file(&full_hash, &abs_path).await {
                    eprintln!("  [Error] Failed to upload {}: {}", file_arg, e);
                    eprintln!("  [Abort] Pointer NOT created to ensure safety.");
                    continue; // Skip to next file, do not create pointer
                }
                println!("  - Upload complete.");
            },
            Err(e) => {
                eprintln!("  [Error] Failed to check remote status: {}", e);
                eprintln!("  [Abort] Pointer NOT created.");
                continue;
            }
        }

        // 5. Create/Update Metadata
        let metadata = ObjectMetadata::new(full_hash.clone(), file_size);
        let metadata_path = get_metadata_path(&root, &full_hash);
        
        if let Some(parent) = metadata_path.parent() {
            fs::create_dir_all(parent).context("Failed to create metadata directory")?;
        }

        let json_content = serde_json::to_string_pretty(&metadata)?;
        fs::write(&metadata_path, json_content).context("Failed to write metadata file")?;
        println!("  - Metadata saved to: {:?}", metadata_path);

        // 6. Create Pointer File
        let pointer_path_str = format!("{}.shadow", abs_path.to_string_lossy());
        let pointer_path = Path::new(&pointer_path_str);
        fs::write(pointer_path, &full_hash).context("Failed to write pointer file")?;
        
        println!("  - Pointer created: {}.shadow", file_arg);

        // 7. Update .gitignore
        if config.core.auto_add_to_gitignore {
            add_to_gitignore(&root, &rel_path).context("Failed to update .gitignore")?;
            println!("  - Added to .gitignore");
        }
    }

    Ok(())
}
