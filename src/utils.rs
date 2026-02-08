use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

pub fn find_project_root() -> Result<PathBuf> {
    let current_dir = std::env::current_dir()?;
    let mut dir = current_dir.clone();

    loop {
        if dir.join(".shadow").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }

    Err(anyhow!(
        "Not a shadow repository (or any of the parent directories): .shadow not found"
    ))
}

/// Calculate SHA256 hash of a file
pub fn compute_sha256(path: &Path) -> Result<String> {
    let file = File::open(path).context("Failed to open file for hashing")?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

fn parse_hash(hash: &str) -> String {
    if let Some(stripped) = hash.strip_prefix("sha256:") {
        stripped.to_string()
    } else {
        hash.to_string()
    }
}

/// Get relative path components for object storage (e.g., objects/ab/cd...)
pub fn get_object_path_components(hash: &str) -> PathBuf {
    let raw_hex = parse_hash(hash);
    if raw_hex.len() < 4 {
        return PathBuf::from("objects").join(raw_hex);
    }
    let dir_1 = &raw_hex[0..2];
    PathBuf::from("objects").join(dir_1).join(&raw_hex[2..])
}

/// Get the path to the metadata file for a given hash
pub fn get_metadata_path(root: &Path, hash: &str) -> PathBuf {
    root.join(".shadow").join(get_object_path_components(hash))
}

/// Get the S3 key for a given hash (always forward slashes)
pub fn get_s3_key(hash: &str) -> String {
    let components = get_object_path_components(hash);
    // Force forward slashes for S3 keys even on Windows
    components
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Ensure a path is in .gitignore (Relative to root)
pub fn add_to_gitignore(root: &Path, path_str: &str) -> Result<()> {
    let gitignore_path = root.join(".gitignore");

    // Normalize path separators to forward slash for gitignore
    let path_str = path_str.replace("\\", "/");

    if gitignore_path.exists() {
        let file = File::open(&gitignore_path)?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line?;
            if line.trim() == path_str {
                return Ok(()); // Already exists
            }
        }
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(gitignore_path)
        .context("Failed to open .gitignore")?;

    writeln!(file, "{}", path_str).context("Failed to append to .gitignore")?;

    Ok(())
}

/// Ensure a path is in .shadowtrack (Relative to root)
pub fn add_to_shadowtrack(root: &Path, path_str: &str) -> Result<()> {
    let track_path = root.join(".shadowtrack");
    let path_str = path_str.replace("\\", "/");

    if track_path.exists() {
        let file = File::open(&track_path)?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line?;
            if line.trim() == path_str {
                return Ok(());
            }
        }
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(track_path)
        .context("Failed to open .shadowtrack")?;

    writeln!(file, "{}", path_str).context("Failed to append to .shadowtrack")?;
    Ok(())
}
