use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub async fn run() -> Result<()> {
    println!("Initializing git-shadow repository...");

    // 1. Create .shadow directory structure
    let shadow_dir = Path::new(".shadow");
    let objects_dir = shadow_dir.join("objects");

    if !shadow_dir.exists() {
        fs::create_dir(shadow_dir).context("Failed to create .shadow directory")?;
        println!("Created .shadow directory");
    }

    if !objects_dir.exists() {
        fs::create_dir(&objects_dir).context("Failed to create .shadow/objects directory")?;
        println!("Created .shadow/objects directory");
    }

    // 2. Create default config
    let config_path = shadow_dir.join("config");
    if !config_path.exists() {
        let default_config = r#"[core]
auto_add_to_gitignore = true

# Configure your remote storage here
# [remote.origin]
# provider = "s3"
# endpoint = "https://<account id>.r2.cloudflarestorage.com"
# bucket = "my-project-assets"
# region = "auto"
"#;
        fs::write(&config_path, default_config).context("Failed to write default config")?;
        println!("Created .shadow/config");
    } else {
        println!(".shadow/config already exists, skipping");
    }

    // 3. Create example .shadowtrack
    let shadowtrack_path = Path::new(".shadowtrack");
    if !shadowtrack_path.exists() {
        let default_track = r#"# Define patterns for files to be tracked by git-shadow
# These files will be uploaded to S3 and replaced by .shadow pointer files in git

# Examples:
# *.psd
# models/**/*.bin
# assets/large_*.texture
"#;
        fs::write(&shadowtrack_path, default_track).context("Failed to write .shadowtrack")?;
        println!("Created .shadowtrack");
    } else {
        println!(".shadowtrack already exists, skipping");
    }

    println!("Initialization complete.");
    Ok(())
}
