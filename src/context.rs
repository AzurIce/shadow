use crate::config::{CONFIG_FILE, Config};
use crate::repository::Repository;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("failed to run git rev-parse")?;
    if !output.status.success() {
        bail!("not inside a Git repository");
    }
    let root = String::from_utf8(output.stdout).context("Git returned a non-UTF-8 root path")?;
    Ok(PathBuf::from(root.trim()))
}

pub fn discover() -> Result<Repository> {
    let root = git_root()?;
    if !root.join(CONFIG_FILE).is_file() {
        bail!(
            "{} not found at Git repository root {}; run 'shadow init' first",
            CONFIG_FILE,
            root.display()
        );
    }
    Repository::load(root)
}

pub fn load_at(root: impl AsRef<Path>) -> Result<Repository> {
    let root = root.as_ref().to_path_buf();
    let config = Config::load(&root)?;
    Repository::from_parts(root, config)
}
