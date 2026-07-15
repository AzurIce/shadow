use crate::config::{CONFIG_FILE, Config};
use crate::context::git_root;
use crate::repository::{SHADOW_DIR, SHADOW_MARKER};
use anyhow::{Context, Result};
use std::fs;

pub fn run() -> Result<()> {
    let root = git_root()?;
    let config_path = root.join(CONFIG_FILE);
    if config_path.exists() {
        anyhow::bail!("{} already exists", config_path.display());
    }

    let shadow = root.join(SHADOW_DIR);
    fs::create_dir_all(shadow.join("refs"))?;
    fs::create_dir_all(shadow.join("cache/objects"))?;
    fs::create_dir_all(shadow.join("tmp"))?;
    fs::write(shadow.join(".gitignore"), "/cache/\n/tmp/\n")
        .context("failed to create .shadow/.gitignore")?;

    let config = Config::new();
    fs::write(&config_path, config.initial_document())
        .with_context(|| format!("failed to create {}", config_path.display()))?;
    ensure_shadow_marker(&root.join(".gitignore"))?;

    println!("Initialized Shadow in {}", root.display());
    println!("Configure the backend in {}", config_path.display());
    Ok(())
}

fn ensure_shadow_marker(path: &std::path::Path) -> Result<()> {
    let content = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    if content.lines().any(|line| line == SHADOW_MARKER) {
        return Ok(());
    }
    let mut updated = content;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    if !updated.is_empty() {
        updated.push('\n');
    }
    updated.push_str(SHADOW_MARKER);
    updated.push('\n');
    fs::write(path, updated)?;
    Ok(())
}
