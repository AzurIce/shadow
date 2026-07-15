use crate::backend::{self, BlobStore};
use crate::context;
use crate::hash::hash_file;
use crate::repository::{Repository, normalize_filters, selected};
use anyhow::{Result, bail};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

pub async fn run(paths: Vec<PathBuf>, force: bool) -> Result<()> {
    let repo = context::discover()?;
    let filters = normalize_filters(&repo.root, &paths)?;
    restore_with_factory(&repo, &filters, force, || backend::open(&repo.config)).await
}

async fn restore_with_factory<F>(
    repo: &Repository,
    filters: &[PathBuf],
    force: bool,
    mut factory: F,
) -> Result<()>
where
    F: FnMut() -> Result<Arc<dyn BlobStore>>,
{
    let refs = repo.load_refs()?;
    let mut store = None;
    let mut processed = 0_usize;
    let mut failures = Vec::new();

    for (relative, reference) in refs {
        if !selected(&relative, filters) {
            continue;
        }
        processed += 1;
        let result = async {
            if !repo.is_managed_path(&relative) {
                bail!("ref is orphaned; path is outside the # shadow rules");
            }
            let target = repo.root.join(&relative);
            if target.exists() {
                let (oid, size) = hash_file(&target)?;
                if oid == reference.oid && size == reference.size {
                    println!("[present] {}", relative.display());
                    return Ok::<(), anyhow::Error>(());
                }
                if !force {
                    bail!("worktree file differs from ref; use --force to replace it");
                }
            }

            let cache = repo.cache_path(&reference.oid);
            if !repo.validate_cache(&reference)? {
                if cache.exists() {
                    fs::remove_file(&cache)?;
                }
                fs::create_dir_all(repo.tmp_dir())?;
                let temporary = repo.tmp_dir().join(format!("{}.download", Uuid::new_v4()));
                let store = match &store {
                    Some(store) => Arc::clone(store),
                    None => {
                        let opened = factory()?;
                        store = Some(Arc::clone(&opened));
                        opened
                    }
                };
                let key = repo.blob_key(&reference.oid);
                if let Err(error) = store.download_file(&key, &temporary).await {
                    let _ = fs::remove_file(&temporary);
                    return Err(error.into());
                }
                repo.promote_download(&temporary, &reference)?;
            }
            repo.materialize(&cache, &target, target.exists())?;
            println!("[restored] {}", relative.display());
            Ok(())
        }
        .await;
        if let Err(error) = result {
            eprintln!("[failed] {}: {error:#}", relative.display());
            failures.push(relative);
        }
    }

    if processed == 0 {
        println!("Nothing to restore.");
    }
    if !failures.is_empty() {
        bail!("failed to restore {} file(s)", failures.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::testing::MemoryStore;
    use crate::commands::publish::publish_with_store;
    use crate::config::Config;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn repository(temp: &TempDir) -> Repository {
        fs::create_dir_all(temp.path().join(".shadow/refs")).unwrap();
        fs::write(temp.path().join(".gitignore"), "# shadow\n*.bin\n").unwrap();
        Repository::from_parts(temp.path().to_path_buf(), Config::new("test").unwrap()).unwrap()
    }

    #[tokio::test]
    async fn restores_missing_file_through_cache() {
        let temp = TempDir::new().unwrap();
        let repo = repository(&temp);
        fs::write(temp.path().join("a.bin"), b"hello").unwrap();
        let store = Arc::new(MemoryStore::default());
        publish_with_store(&repo, &[], store.as_ref())
            .await
            .unwrap();
        let reference = repo
            .load_refs()
            .unwrap()
            .remove(Path::new("a.bin"))
            .unwrap();
        fs::remove_file(temp.path().join("a.bin")).unwrap();
        fs::remove_file(repo.cache_path(&reference.oid)).unwrap();

        let opened = Arc::clone(&store);
        restore_with_factory(&repo, &[], false, move || {
            Ok(Arc::clone(&opened) as Arc<dyn BlobStore>)
        })
        .await
        .unwrap();
        assert_eq!(fs::read(temp.path().join("a.bin")).unwrap(), b"hello");
    }

    #[tokio::test]
    async fn refuses_to_replace_modified_file_without_force() {
        let temp = TempDir::new().unwrap();
        let repo = repository(&temp);
        fs::write(temp.path().join("a.bin"), b"hello").unwrap();
        let store = Arc::new(MemoryStore::default());
        publish_with_store(&repo, &[], store.as_ref())
            .await
            .unwrap();
        fs::write(temp.path().join("a.bin"), b"changed").unwrap();

        let opened = Arc::clone(&store);
        let result = restore_with_factory(&repo, &[], false, move || {
            Ok(Arc::clone(&opened) as Arc<dyn BlobStore>)
        })
        .await;
        assert!(result.is_err());
        assert_eq!(fs::read(temp.path().join("a.bin")).unwrap(), b"changed");
    }
}
