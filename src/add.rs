use crate::object::ObjectMetadata;
use crate::stage::StagingIndex;
use crate::utils::{add_to_shadowtrack, compute_sha256, find_project_root};
use anyhow::{Context, Result};
use std::fs;

pub async fn run(files: Vec<String>) -> Result<()> {
    let root = find_project_root()?;
    let cwd = std::env::current_dir()?;

    // Load staging index
    let mut index = StagingIndex::load(&root)?;

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

        // Calculate path relative to root
        let rel_path = match abs_path.strip_prefix(&root) {
            Ok(p) => p.to_string_lossy().replace("\\", "/"),
            Err(_) => {
                eprintln!("File {} is outside the shadow repository root", file_arg);
                continue;
            }
        };

        println!("Processing: {}", rel_path);

        // 1. Compute Hash
        let raw_hash = compute_sha256(&abs_path).context("Failed to compute hash")?;
        let full_hash = format!("sha256:{}", raw_hash);
        let file_size = fs::metadata(&abs_path)?.len();

        // 2. Create Metadata (InMemory)
        let metadata = ObjectMetadata::new(full_hash.clone(), file_size);
        
        // 3. Update Staging Index
        index.add(rel_path.clone(), metadata);

        // 4. Update .shadowtrack (Ensure it's tracked)
        add_to_shadowtrack(&root, &rel_path)?;

        println!("  - Staged for push: {}", rel_path);
    }

    // Save index
    index.save(&root)?;
    
    println!("Add complete. Files are staged. Run 'shadow push' to upload and finalize shadowing.");
    Ok(())
}
