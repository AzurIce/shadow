use crate::context;
use crate::repository::{normalize_filters, selected};
use anyhow::{Result, bail};
use std::path::PathBuf;

pub fn run(paths: Vec<PathBuf>) -> Result<()> {
    if paths.is_empty() {
        bail!("remove requires at least one path");
    }
    let repo = context::discover()?;
    let filters = normalize_filters(&repo.root, &paths)?;
    let refs = repo.load_refs()?;
    let mut removed = 0_usize;
    for relative in refs.keys().filter(|path| selected(path, &filters)) {
        if repo.remove_ref(relative)? {
            println!("[removed] {}", relative.display());
            removed += 1;
        }
    }
    if removed == 0 {
        println!("No refs removed.");
    }
    Ok(())
}
