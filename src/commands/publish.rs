use crate::backend::{self, BlobStore};
use crate::context;
use crate::repository::{Repository, normalize_filters, selected};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub async fn run(paths: Vec<PathBuf>) -> Result<()> {
    let repo = context::discover()?;
    let filters = normalize_filters(&repo.root, &paths)?;
    let store = backend::open(&repo.config)?;
    publish_with_store(&repo, &filters, store.as_ref()).await
}

pub async fn publish_with_store(
    repo: &Repository,
    filters: &[PathBuf],
    store: &dyn BlobStore,
) -> Result<()> {
    let worktree = repo.managed_files()?;
    let refs = repo.load_refs()?;
    let mut processed = 0_usize;
    let mut failures = Vec::new();

    for (relative, source) in worktree {
        if !selected(&relative, filters) {
            continue;
        }
        processed += 1;
        let result = async {
            let reference = repo.import_to_cache(&source)?;
            if refs.get(&relative) == Some(&reference) {
                println!("[published] {}", relative.display());
                return Ok::<(), anyhow::Error>(());
            }
            let key = repo.blob_key(&reference.oid);
            match store.stat(&key).await? {
                Some(metadata) if metadata.size == reference.size => {
                    println!("[deduplicated] {}", relative.display());
                }
                Some(metadata) => bail!(
                    "remote object {} has size {}, expected {}",
                    reference.oid,
                    metadata.size,
                    reference.size
                ),
                None => {
                    println!("[uploading] {}", relative.display());
                    store
                        .upload_file(&key, &repo.cache_path(&reference.oid), reference.size)
                        .await?;
                    let metadata = store
                        .stat(&key)
                        .await?
                        .context("uploaded object is not visible on remote")?;
                    if metadata.size != reference.size {
                        bail!("uploaded object size does not match local ref");
                    }
                }
            }
            repo.write_ref(&relative, &reference)?;
            println!("[published] {} -> {}", relative.display(), reference.oid);
            Ok(())
        }
        .await;
        if let Err(error) = result {
            eprintln!("[failed] {}: {error:#}", relative.display());
            failures.push(relative);
        }
    }

    if processed == 0 {
        println!("Nothing to publish.");
    }
    if !failures.is_empty() {
        bail!("failed to publish {} file(s)", failures.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::testing::MemoryStore;
    use crate::config::Config;
    use crate::model::BlobKey;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[tokio::test]
    async fn publishes_and_writes_ref() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".shadow/refs")).unwrap();
        fs::write(temp.path().join(".gitignore"), "# shadow\n*.bin\n").unwrap();
        fs::write(temp.path().join("a.bin"), b"hello").unwrap();
        let repo = Repository::from_parts(temp.path().to_path_buf(), Config::new("test").unwrap())
            .unwrap();
        let store = MemoryStore::default();
        publish_with_store(&repo, &[], &store).await.unwrap();
        let reference = repo
            .load_refs()
            .unwrap()
            .remove(Path::new("a.bin"))
            .unwrap();
        assert!(store.contains(&BlobKey::for_object(&repo.config.name, &reference.oid)));
    }
}
