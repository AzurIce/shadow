use crate::backend::{self, BlobStore};
use crate::context;
use crate::media_type;
use crate::model::{ObjectId, ShadowRef, UploadOptions};
use crate::repository::Repository;
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::PathBuf;

struct PublishItem {
    relative: PathBuf,
    reference: ShadowRef,
    content_type: String,
}

pub async fn run() -> Result<()> {
    let repo = context::discover()?;
    let store = backend::open(&repo.config)?;
    publish_with_store(&repo, store.as_ref()).await
}

pub async fn publish_with_store(repo: &Repository, store: &dyn BlobStore) -> Result<()> {
    let worktree = repo.managed_files()?;
    let refs = repo.load_refs()?;
    let mut declared_types = HashMap::new();

    let mut items = Vec::with_capacity(worktree.len());
    for (relative, source) in worktree {
        let reference = repo.import_to_cache(&source)?;
        let content_type =
            media_type::detect(&relative, Some(repo.cache_path(&reference.oid).as_path()))?;
        register_content_type(
            &mut declared_types,
            &reference.oid,
            &content_type,
            &relative,
        )?;
        items.push(PublishItem {
            relative,
            reference,
            content_type,
        });
    }

    let processed = items.len();
    let mut failures = Vec::new();

    for item in items {
        let relative = item.relative;
        let reference = item.reference;
        let content_type = item.content_type;
        let result = async {
            let ref_unchanged = refs.get(&relative) == Some(&reference);
            let key = repo.blob_key(&reference.oid);
            let options = UploadOptions::new(&content_type);
            match store.stat(&key).await? {
                Some(metadata) if metadata.size != reference.size => bail!(
                    "remote object {} has size {}, expected {}",
                    reference.oid,
                    metadata.size,
                    reference.size
                ),
                Some(metadata)
                    if metadata.content_type.as_deref() == Some(content_type.as_str())
                        && metadata.cache_control.as_deref()
                            == Some(options.cache_control.as_str()) => {}
                Some(_) => {
                    println!("[updating metadata] {}", relative.display());
                    store.update_metadata(&key, &options).await?;
                    verify_remote(store, &key, &reference, &options).await?;
                }
                None => {
                    println!("[uploading] {}", relative.display());
                    store
                        .upload_file(
                            &key,
                            &repo.cache_path(&reference.oid),
                            reference.size,
                            &options,
                        )
                        .await?;
                    verify_remote(store, &key, &reference, &options).await?;
                }
            }
            repo.write_ref(&relative, &reference)?;
            if ref_unchanged {
                println!("[published] {}", relative.display());
            } else {
                println!("[published] {} -> {}", relative.display(), reference.oid);
            }
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

async fn verify_remote(
    store: &dyn BlobStore,
    key: &crate::model::BlobKey,
    reference: &ShadowRef,
    options: &UploadOptions,
) -> Result<()> {
    let metadata = store
        .stat(key)
        .await?
        .context("published object is not visible on remote")?;
    if metadata.size != reference.size {
        bail!("published object size does not match local ref");
    }
    if metadata.content_type.as_deref() != Some(options.content_type.as_str()) {
        bail!("published object content type does not match expected value");
    }
    if metadata.cache_control.as_deref() != Some(options.cache_control.as_str()) {
        bail!("published object cache control does not match expected value");
    }
    Ok(())
}

fn register_content_type(
    declared: &mut HashMap<ObjectId, (String, PathBuf)>,
    oid: &ObjectId,
    content_type: &str,
    path: &std::path::Path,
) -> Result<()> {
    if let Some((existing, existing_path)) = declared.get(oid) {
        if existing != content_type {
            bail!(
                "the same object {} has conflicting content types: {} ({}) and {} ({})",
                oid,
                existing,
                existing_path.display(),
                content_type,
                path.display()
            );
        }
        return Ok(());
    }
    declared.insert(oid.clone(), (content_type.to_string(), path.to_path_buf()));
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
        publish_with_store(&repo, &store).await.unwrap();
        let reference = repo
            .load_refs()
            .unwrap()
            .remove(Path::new("a.bin"))
            .unwrap();
        let key = BlobKey::for_object(&repo.config.name, &reference.oid);
        assert!(store.contains(&key));
        assert_eq!(
            store.content_type(&key).as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(
            store.cache_control(&key).as_deref(),
            Some(crate::model::DEFAULT_CACHE_CONTROL)
        );
    }

    #[tokio::test]
    async fn repairs_existing_object_metadata_without_uploading_content() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".shadow/refs")).unwrap();
        fs::write(temp.path().join(".gitignore"), "# shadow\n*.png\n").unwrap();
        let bytes = [
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R',
        ];
        fs::write(temp.path().join("a.png"), bytes).unwrap();
        let repo = Repository::from_parts(temp.path().to_path_buf(), Config::new("test").unwrap())
            .unwrap();
        let reference = repo.import_to_cache(&temp.path().join("a.png")).unwrap();
        repo.write_ref(Path::new("a.png"), &reference).unwrap();
        let key = BlobKey::for_object(&repo.config.name, &reference.oid);
        let store = MemoryStore::default();
        store.insert_without_metadata(&key, bytes.to_vec());

        publish_with_store(&repo, &store).await.unwrap();

        assert_eq!(store.content_type(&key).as_deref(), Some("image/png"));
        assert_eq!(
            store.cache_control(&key).as_deref(),
            Some(crate::model::DEFAULT_CACHE_CONTROL)
        );
        assert_eq!(store.upload_count(), 0);
        assert_eq!(store.metadata_update_count(), 1);
        let serialized = fs::read_to_string(repo.ref_path(Path::new("a.png")).unwrap()).unwrap();
        assert!(!serialized.contains("content_type"));
    }

    #[tokio::test]
    async fn rejects_conflicting_content_types_for_same_object() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".shadow/refs")).unwrap();
        fs::write(temp.path().join(".gitignore"), "# shadow\n*.css\n*.txt\n").unwrap();
        fs::write(temp.path().join("a.css"), b"same bytes").unwrap();
        fs::write(temp.path().join("a.txt"), b"same bytes").unwrap();
        let repo = Repository::from_parts(temp.path().to_path_buf(), Config::new("test").unwrap())
            .unwrap();
        let store = MemoryStore::default();

        let error = publish_with_store(&repo, &store).await.unwrap_err();

        assert!(error.to_string().contains("conflicting content types"));
    }
}
