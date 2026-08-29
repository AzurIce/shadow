use crate::backend::{self, BlobInventory};
use crate::context;
use crate::model::{ObjectId, ShadowRef};
use crate::repository::Repository;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

use super::gc::{
    CollectOptions, collect_with_inventory, historical_ref_versions, read_ref_objects,
};

/// Number of ref versions (and distinct ref paths) in Git history that point at
/// objects not kept by the retention snapshots — they become dangling after `free`.
pub(crate) struct DanglingRefs {
    pub versions: usize,
    pub paths: usize,
}

pub async fn run(keep_at: Vec<String>, delete: bool, grace_days: u64) -> Result<()> {
    let repo = context::discover()?;
    let inventory = backend::open_inventory(&repo.config)?;
    free_with_inventory(
        &repo,
        inventory.as_ref(),
        &keep_at,
        delete,
        grace_days,
        SystemTime::now(),
    )
    .await
}

/// Release remote objects that are not referenced by refs in the given commits'
/// trees. Unlike `gc`, released objects may still be referenced by refs elsewhere
/// in Git history: those refs become dangling, which is the documented contract
/// of this command (mirroring `free()` invalidating pointers).
pub async fn free_with_inventory(
    repo: &Repository,
    inventory: &dyn BlobInventory,
    keep_at: &[String],
    delete: bool,
    grace_days: u64,
    now: SystemTime,
) -> Result<()> {
    if keep_at.is_empty() {
        bail!("at least one --keep-at <ref> is required");
    }
    let commits = resolve_commits(&repo.root, keep_at)?;
    let referenced = snapshot_objects(&repo.root, &commits)?;
    for (commit, value) in commits.iter().zip(keep_at) {
        println!("Keeping refs at {commit} (--keep-at {value})");
    }
    let dangling = dangling_ref_versions(&repo.root, &referenced)?;
    println!(
        "{} dangling ref version(s) across {} path(s) will remain in Git history",
        dangling.versions, dangling.paths
    );
    collect_with_inventory(
        repo,
        inventory,
        CollectOptions {
            referenced,
            refresh_referenced: &|| {
                snapshot_objects(&repo.root, &resolve_commits(&repo.root, keep_at)?)
            },
            label: "free",
            delete,
            grace_days,
            now,
        },
    )
    .await
}

fn resolve_commits(root: &Path, keep_at: &[String]) -> Result<Vec<String>> {
    keep_at
        .iter()
        .map(|value| {
            let output = Command::new("git")
                .current_dir(root)
                .args(["rev-parse", "--verify", &format!("{value}^{{commit}}")])
                .output()
                .with_context(|| format!("failed to resolve --keep-at {value}"))?;
            if !output.status.success() {
                bail!("--keep-at {value} does not resolve to a commit");
            }
            let commit = std::str::from_utf8(&output.stdout)
                .with_context(|| format!("git rev-parse returned non-UTF-8 output for {value}"))?
                .trim();
            if commit.is_empty() {
                bail!("--keep-at {value} does not resolve to a commit");
            }
            Ok(commit.to_string())
        })
        .collect()
}

/// Read the refs contained in exactly the given commits — no ancestry is traversed.
fn snapshot_objects(root: &Path, commits: &[String]) -> Result<BTreeMap<ObjectId, u64>> {
    let mut blobs = BTreeSet::new();
    for commit in commits {
        let output = Command::new("git")
            .current_dir(root)
            .args(["ls-tree", "-r", "-z", commit, "--", ".shadow/refs"])
            .output()
            .with_context(|| format!("failed to run git ls-tree for {commit}"))?;
        if !output.status.success() {
            bail!(
                "failed to list Shadow refs at {commit}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        for entry in output.stdout.split(|byte| *byte == 0) {
            if entry.is_empty() {
                continue;
            }
            let entry =
                std::str::from_utf8(entry).context("git ls-tree returned non-UTF-8 output")?;
            let Some((metadata, path)) = entry.split_once('\t') else {
                continue;
            };
            let mut fields = metadata.split(' ');
            let (Some(_mode), Some(kind), Some(git_oid)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if kind != "blob" || !path.starts_with(".shadow/refs/") || !path.ends_with(".ref") {
                continue;
            }
            blobs.insert((git_oid.to_string(), path.to_string()));
        }
    }
    read_ref_objects(root, blobs)
}

fn dangling_ref_versions(root: &Path, kept: &BTreeMap<ObjectId, u64>) -> Result<DanglingRefs> {
    let mut versions = 0_usize;
    let mut paths = BTreeSet::new();
    for (git_oid, path) in historical_ref_versions(root)? {
        let output = Command::new("git")
            .current_dir(root)
            .args(["cat-file", "blob", &git_oid])
            .output()
            .with_context(|| format!("failed to read committed ref {path}"))?;
        if !output.status.success() {
            bail!("failed to read committed ref {path} ({git_oid})");
        }
        let content = std::str::from_utf8(&output.stdout)
            .with_context(|| format!("committed ref {path} is not UTF-8"))?;
        let reference = ShadowRef::parse(content)
            .with_context(|| format!("invalid committed ref {path} ({git_oid})"))?;
        if !kept.contains_key(&reference.oid) {
            versions += 1;
            paths.insert(path);
        }
    }
    Ok(DanglingRefs {
        versions,
        paths: paths.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::testing::MemoryStore;
    use crate::config::Config;
    use crate::model::BlobKey;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    fn git(temp: &TempDir, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(temp.path())
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn setup_repository(temp: &TempDir) -> Repository {
        git(temp, &["init", "-q"]);
        git(temp, &["config", "user.name", "Shadow Tests"]);
        git(temp, &["config", "user.email", "shadow@example.invalid"]);
        fs::create_dir_all(temp.path().join(".shadow/refs")).unwrap();
        fs::write(temp.path().join(".gitignore"), "# shadow\n*.bin\n").unwrap();
        Repository::from_parts(temp.path().to_path_buf(), Config::new("test").unwrap()).unwrap()
    }

    fn commit_ref(temp: &TempDir, oid: ObjectId, message: &str) {
        let reference = ShadowRef { oid, size: 1 };
        fs::write(
            temp.path().join(".shadow/refs/a.bin.ref"),
            reference.serialize().unwrap(),
        )
        .unwrap();
        git(temp, &["add", ".shadow/refs/a.bin.ref"]);
        git(temp, &["commit", "-q", "-m", message]);
    }

    #[tokio::test]
    async fn requires_at_least_one_keep_at() {
        let temp = TempDir::new().unwrap();
        let repo = setup_repository(&temp);
        let store = MemoryStore::default();

        let result = free_with_inventory(&repo, &store, &[], false, 0, SystemTime::now()).await;

        assert!(result.is_err());
    }

    #[test]
    fn snapshot_reads_refs_only_at_kept_commits() {
        let temp = TempDir::new().unwrap();
        let repo = setup_repository(&temp);
        let first = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        let second = ObjectId::from_sha256_hex("b".repeat(64)).unwrap();
        commit_ref(&temp, first.clone(), "first ref");
        commit_ref(&temp, second.clone(), "second ref");

        let head = resolve_commits(&repo.root, &["HEAD".to_string()]).unwrap();
        let parent = resolve_commits(&repo.root, &["HEAD~1".to_string()]).unwrap();

        assert_ne!(head, parent);
        assert_eq!(
            snapshot_objects(&repo.root, &head).unwrap(),
            BTreeMap::from([(second, 1)])
        );
        assert_eq!(
            snapshot_objects(&repo.root, &parent).unwrap(),
            BTreeMap::from([(first, 1)])
        );
    }

    #[tokio::test]
    async fn dry_run_keeps_and_delete_releases_objects_not_referenced_at_snapshot() {
        let temp = TempDir::new().unwrap();
        let repo = setup_repository(&temp);
        let superseded = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        let current = ObjectId::from_sha256_hex("b".repeat(64)).unwrap();
        commit_ref(&temp, superseded.clone(), "superseded");
        commit_ref(&temp, current.clone(), "current");
        let superseded_key = BlobKey::for_object("test", &superseded);
        let current_key = BlobKey::for_object("test", &current);
        let store = MemoryStore::default();
        store.insert_without_metadata(&superseded_key, vec![1]);
        store.insert_without_metadata(&current_key, vec![2]);
        let now = SystemTime::now() + Duration::from_secs(1);

        free_with_inventory(&repo, &store, &["HEAD".to_string()], false, 0, now)
            .await
            .unwrap();
        assert!(store.contains(&superseded_key));
        assert!(store.contains(&current_key));

        free_with_inventory(&repo, &store, &["HEAD".to_string()], true, 0, now)
            .await
            .unwrap();
        assert!(!store.contains(&superseded_key));
        assert!(store.contains(&current_key));
        assert!(repo.ref_path(Path::new("a.bin")).unwrap().exists());
    }

    #[tokio::test]
    async fn refuses_when_a_snapshot_ref_is_missing_remotely() {
        let temp = TempDir::new().unwrap();
        let repo = setup_repository(&temp);
        let superseded = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        let current = ObjectId::from_sha256_hex("b".repeat(64)).unwrap();
        commit_ref(&temp, superseded.clone(), "superseded");
        commit_ref(&temp, current, "current");
        let superseded_key = BlobKey::for_object("test", &superseded);
        let store = MemoryStore::default();
        store.insert_without_metadata(&superseded_key, vec![1]);

        let result = free_with_inventory(
            &repo,
            &store,
            &["HEAD".to_string()],
            true,
            0,
            SystemTime::now() + Duration::from_secs(1),
        )
        .await;

        assert!(result.is_err());
        assert!(store.contains(&superseded_key));
    }

    #[test]
    fn counts_dangling_ref_versions_before_release() {
        let temp = TempDir::new().unwrap();
        let repo = setup_repository(&temp);
        let superseded = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        let current = ObjectId::from_sha256_hex("b".repeat(64)).unwrap();
        commit_ref(&temp, superseded.clone(), "superseded");
        commit_ref(&temp, current.clone(), "current");

        let dangling = dangling_ref_versions(&repo.root, &BTreeMap::from([(current, 1)])).unwrap();
        assert_eq!(dangling.versions, 1);
        assert_eq!(dangling.paths, 1);

        let all_dangling = dangling_ref_versions(&repo.root, &BTreeMap::new()).unwrap();
        assert_eq!(all_dangling.versions, 2);
        assert_eq!(all_dangling.paths, 1);
    }
}
