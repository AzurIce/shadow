use crate::backend::{self, BlobStore};
use crate::context;
use crate::hash::hash_file;
use crate::repository::{Repository, normalize_filters, selected};
use anyhow::Result;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalState {
    Unpublished,
    Published,
    Modified,
    Missing,
    Orphaned,
}

pub struct StatusRecord {
    pub path: PathBuf,
    pub state: LocalState,
    pub cache: &'static str,
    pub remote: Option<&'static str>,
}

pub async fn run(paths: Vec<PathBuf>, check_remote: bool) -> Result<()> {
    let repo = context::discover()?;
    let filters = normalize_filters(&repo.root, &paths)?;
    let store = if check_remote {
        Some(backend::open(&repo.config)?)
    } else {
        None
    };
    let records = collect(&repo, &filters, store.as_deref()).await?;
    if records.is_empty() {
        println!("No managed files or refs.");
        return Ok(());
    }
    for record in records {
        let remote = record
            .remote
            .map(|value| format!(" remote={value}"))
            .unwrap_or_default();
        println!(
            "{:<11} {} cache={}{}",
            format!("{:?}", record.state),
            record.path.display(),
            record.cache,
            remote
        );
    }
    Ok(())
}

pub async fn collect(
    repo: &Repository,
    filters: &[PathBuf],
    store: Option<&dyn BlobStore>,
) -> Result<Vec<StatusRecord>> {
    let worktree = repo.managed_files()?;
    let refs = repo.load_refs()?;
    let paths: BTreeSet<_> = worktree.keys().chain(refs.keys()).cloned().collect();
    let mut records = Vec::new();

    for path in paths {
        if !selected(&path, filters) {
            continue;
        }
        let body = worktree.get(&path);
        let reference = refs.get(&path);
        let state = match (body, reference) {
            (Some(_), None) => LocalState::Unpublished,
            (None, Some(_)) if !repo.is_managed_path(&path) => LocalState::Orphaned,
            (None, Some(_)) => LocalState::Missing,
            (Some(_), Some(_)) if !repo.is_managed_path(&path) => LocalState::Orphaned,
            (Some(body), Some(reference)) => {
                let (oid, size) = hash_file(body)?;
                if oid == reference.oid && size == reference.size {
                    LocalState::Published
                } else {
                    LocalState::Modified
                }
            }
            (None, None) => continue,
        };
        let cache = match reference {
            None => "n/a",
            Some(reference) if repo.cache_present(reference)? => "present",
            Some(_) => "missing",
        };
        let remote = match (reference, store) {
            (Some(reference), Some(store)) => {
                let key = repo.blob_key(&reference.oid);
                match store.stat(&key).await? {
                    None => Some("missing"),
                    Some(metadata) if metadata.size == reference.size => Some("ok"),
                    Some(_) => Some("invalid"),
                }
            }
            _ => None,
        };
        records.push(StatusRecord {
            path,
            state,
            cache,
            remote,
        });
    }
    Ok(records)
}

pub fn state_requires_attention(state: LocalState) -> bool {
    state != LocalState::Published
}

pub fn path_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::testing::MemoryStore;
    use crate::commands::publish::publish_with_store;
    use crate::config::Config;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn reports_modified_and_missing_states() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".shadow/refs")).unwrap();
        fs::write(temp.path().join(".gitignore"), "# shadow\n*.bin\n").unwrap();
        fs::write(temp.path().join("a.bin"), b"a").unwrap();
        fs::write(temp.path().join("b.bin"), b"b").unwrap();
        let repo = Repository::from_parts(temp.path().to_path_buf(), Config::new()).unwrap();
        let store = MemoryStore::default();
        publish_with_store(&repo, &[], &store).await.unwrap();

        fs::write(temp.path().join("a.bin"), b"changed").unwrap();
        fs::remove_file(temp.path().join("b.bin")).unwrap();
        let records = collect(&repo, &[], None).await.unwrap();
        assert_eq!(records[0].state, LocalState::Modified);
        assert_eq!(records[1].state, LocalState::Missing);
    }
}
