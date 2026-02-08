use crate::stage::StagingIndex;
use crate::utils::{compute_sha256, get_metadata_path, find_project_root};
use anyhow::{Context, Result};
use glob::glob;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub async fn run() -> Result<()> {
    // 1. Find root
    let root = find_project_root()?;
    std::env::set_current_dir(&root).context("Failed to change CWD to project root")?;

    println!("Shadow Status (Root: {:?})", root);
    println!("==============");

    // Load Staging Index
    let staging = StagingIndex::load(&root)?;

    // Report Staged Files
    if !staging.entries.is_empty() {
        for (path, _) in &staging.entries {
            println!("  [Staged]     {} (Ready to push)", path);
        }
    }

    let patterns = load_shadowtrack(&root)?;
    
    // 1. Find all potential source files (from .shadowtrack)
    let mut tracked_sources = HashSet::new();
    if !patterns.is_empty() {
        for pattern in patterns {
            if let Ok(paths) = glob(&pattern) {
                for entry in paths {
                    if let Ok(path) = entry {
                        if path.is_file() {
                            // Filter out staged files from "Untracked" report?
                            // Yes, if it is staged, it is tracked (in a way).
                            // But status logic below checks for POINTER.
                            // Staged files don't have pointers yet.
                            // So they would show up as Untracked.
                            // We should skip them if they are in staging.
                            let path_lossy = path.to_string_lossy().replace("\\", "/");
                            if !staging.entries.contains_key(&path_lossy) {
                                tracked_sources.insert(path);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Find all existing pointers (.shadow files)
    let mut pointers = HashSet::new();
    if let Ok(paths) = glob("**/*.shadow") {
        for entry in paths {
            if let Ok(path) = entry {
                if path.is_file() {
                    pointers.insert(path);
                }
            }
        }
    }

    let mut has_changes = false;

    // Check for Untracked (In source, No pointer, Not Staged)
    let mut sorted_sources: Vec<_> = tracked_sources.iter().collect();
    sorted_sources.sort();

    for path in sorted_sources {
        let path_str = path.to_string_lossy();
        let shadow_path = PathBuf::from(format!("{}.shadow", path_str));

        if !pointers.contains(&shadow_path) {
            println!("  [Untracked]  {} (Matches .shadowtrack)", path_str);
            has_changes = true;
        }
    }

    // Check for Missing / Modified / Orphaned
    let mut sorted_pointers: Vec<_> = pointers.iter().collect();
    sorted_pointers.sort();

    for shadow_path in sorted_pointers {
        let source_path = shadow_path.with_extension("");
        let path_str = source_path.to_string_lossy();

        let pointer_content = match fs::read_to_string(shadow_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let stored_hash = pointer_content.trim();

        if !source_path.exists() {
            println!("  [Missing]    {} (Deleted locally)", path_str);
            has_changes = true;
        } else {
            // Source exists. Check Hash.
            let current_hash_raw = match compute_sha256(&source_path) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("  [Error]      {} (Hash fail: {})", path_str, e);
                    continue;
                }
            };
            let current_hash = format!("sha256:{}", current_hash_raw);

            if current_hash != stored_hash {
                println!("  [Modified]   {} (Content changed)", path_str);
                has_changes = true;
            } else {
                // Check Metadata
                let metadata_path = get_metadata_path(&root, stored_hash);
                if !metadata_path.exists() {
                     println!("  [NoMeta]     {} (Metadata missing)", path_str);
                }
            }
        }
    }

    if !has_changes && staging.entries.is_empty() {
        println!("Nothing to report. Working tree clean.");
    }

    Ok(())
}

fn load_shadowtrack(root: &Path) -> Result<Vec<String>> {
    let path = root.join(".shadowtrack");
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(path)?;
    let patterns = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();
    
    Ok(patterns)
}
