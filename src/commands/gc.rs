use crate::backend::{self, BlobInventory};
use crate::context;
use crate::model::{BlobKey, BlobKeyPrefix, InventoryObject, ObjectId, ShadowRef};
use crate::repository::Repository;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::{Duration, SystemTime};

const DELETE_BATCH_SIZE: usize = 1000;
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

struct Candidate {
    key: BlobKey,
    oid: ObjectId,
    size: u64,
}

pub async fn run(delete: bool, grace_days: u64) -> Result<()> {
    let repo = context::discover()?;
    let inventory = backend::open_inventory(&repo.config)?;
    gc_with_inventory(
        &repo,
        inventory.as_ref(),
        delete,
        grace_days,
        SystemTime::now(),
    )
    .await
}

pub async fn gc_with_inventory(
    repo: &Repository,
    inventory: &dyn BlobInventory,
    delete: bool,
    grace_days: u64,
    now: SystemTime,
) -> Result<()> {
    let referenced = reachable_objects(&repo.root)?;
    let prefix = BlobKeyPrefix::for_project_objects(&repo.config.name);
    let remote = inventory.list_prefix(&prefix).await?;
    let grace = Duration::from_secs(
        grace_days
            .checked_mul(SECONDS_PER_DAY)
            .context("GC grace period is too large")?,
    );
    let cutoff = now
        .checked_sub(grace)
        .context("GC grace period predates the system clock")?;
    let mut candidates = Vec::new();
    let mut recent = 0_usize;
    let mut unknown = 0_usize;
    let mut missing_time = 0_usize;
    let remote_objects = remote
        .iter()
        .filter_map(|object| canonical_object_id(repo, object).map(|oid| (oid, object.size)))
        .collect::<BTreeMap<_, _>>();
    let missing_remote = referenced
        .keys()
        .filter(|oid| !remote_objects.contains_key(*oid))
        .cloned()
        .collect::<Vec<_>>();
    let invalid_remote = referenced
        .iter()
        .filter_map(|(oid, expected_size)| {
            remote_objects
                .get(oid)
                .filter(|actual_size| *actual_size != expected_size)
                .map(|actual_size| (oid.clone(), *expected_size, *actual_size))
        })
        .collect::<Vec<_>>();

    for object in &remote {
        match classify_object(repo, object, &referenced, cutoff) {
            ObjectClass::Candidate(candidate) => candidates.push(candidate),
            ObjectClass::Referenced => {}
            ObjectClass::Recent => recent += 1,
            ObjectClass::Unknown => unknown += 1,
            ObjectClass::MissingTime => missing_time += 1,
        }
    }
    candidates.sort_by(|left, right| left.key.cmp(&right.key));

    println!("Git-referenced objects: {}", referenced.len());
    println!("Remote objects scanned: {}", remote.len());
    for oid in &missing_remote {
        println!("[missing remote] {oid}");
    }
    for (oid, expected_size, actual_size) in &invalid_remote {
        println!("[invalid remote] {oid} (expected {expected_size} bytes, found {actual_size})");
    }
    for candidate in &candidates {
        println!("[candidate] {} ({} bytes)", candidate.oid, candidate.size);
    }
    println!(
        "GC candidates: {}; recent objects kept: {}; unknown keys kept: {}; objects without modification time kept: {}",
        candidates.len(),
        recent,
        unknown,
        missing_time
    );

    if !missing_remote.is_empty() || !invalid_remote.is_empty() {
        bail!(
            "refusing GC because {} Git-referenced object(s) are missing or invalid remotely",
            missing_remote.len() + invalid_remote.len()
        );
    }

    if candidates.is_empty() {
        println!("Nothing to delete.");
        return Ok(());
    }
    if !delete {
        println!("Dry run only. Re-run with --delete to remove these objects.");
        return Ok(());
    }

    // Re-read Git immediately before deletion so refs created during the inventory scan win.
    let referenced_before_delete = reachable_objects(&repo.root)?;
    candidates.retain(|candidate| !referenced_before_delete.contains_key(&candidate.oid));
    if candidates.is_empty() {
        println!("All candidates became referenced before deletion; nothing was deleted.");
        return Ok(());
    }

    let mut deleted = 0_usize;
    for batch in candidates.chunks(DELETE_BATCH_SIZE) {
        let keys = batch
            .iter()
            .map(|candidate| candidate.key.clone())
            .collect::<Vec<_>>();
        inventory.delete_batch(&keys).await?;
        deleted += keys.len();
        for candidate in batch {
            println!("[deleted] {}", candidate.oid);
        }
    }
    println!("Deleted {deleted} unreferenced remote object(s).");
    Ok(())
}

enum ObjectClass {
    Candidate(Candidate),
    Referenced,
    Recent,
    Unknown,
    MissingTime,
}

fn classify_object(
    repo: &Repository,
    object: &InventoryObject,
    referenced: &BTreeMap<ObjectId, u64>,
    cutoff: SystemTime,
) -> ObjectClass {
    let Some(oid) = canonical_object_id(repo, object) else {
        return ObjectClass::Unknown;
    };
    let key = BlobKey::parse(&object.key).expect("canonical object keys are valid blob keys");
    if referenced.contains_key(&oid) {
        return ObjectClass::Referenced;
    }
    let Some(modified_at) = object.modified_at else {
        return ObjectClass::MissingTime;
    };
    if modified_at > cutoff {
        return ObjectClass::Recent;
    }
    ObjectClass::Candidate(Candidate {
        key,
        oid,
        size: object.size,
    })
}

fn canonical_object_id(repo: &Repository, object: &InventoryObject) -> Option<ObjectId> {
    BlobKey::parse(&object.key)
        .ok()?
        .object_id_for_project(&repo.config.name)
}

fn reachable_objects(root: &std::path::Path) -> Result<BTreeMap<ObjectId, u64>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-list", "--objects", "--all", "-z", "--", ".shadow/refs"])
        .output()
        .context("failed to run git rev-list for Shadow refs")?;
    if !output.status.success() {
        bail!(
            "failed to enumerate committed Shadow refs: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut blobs = BTreeSet::new();
    let mut pending_oid = None;
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        if let Some(separator) = record.iter().position(|byte| *byte == b' ') {
            let path = &record[separator + 1..];
            if path.starts_with(b".shadow/refs/") && path.ends_with(b".ref") {
                let git_oid = std::str::from_utf8(&record[..separator])
                    .context("Git returned a non-UTF-8 object ID")?;
                blobs.insert((
                    git_oid.to_string(),
                    String::from_utf8_lossy(path).into_owned(),
                ));
            }
            continue;
        }
        if record.len() >= 40 && record.iter().all(u8::is_ascii_hexdigit) {
            pending_oid = Some(
                std::str::from_utf8(record)
                    .context("Git returned a non-UTF-8 object ID")?
                    .to_string(),
            );
            continue;
        }
        let path = record.strip_prefix(b"path=").unwrap_or(record);
        if path.starts_with(b".shadow/refs/") && path.ends_with(b".ref") {
            let git_oid = pending_oid
                .take()
                .context("Git returned a ref path without an object ID")?;
            blobs.insert((git_oid, String::from_utf8_lossy(path).into_owned()));
        }
    }

    let mut referenced = BTreeMap::new();
    for (git_oid, path) in blobs {
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
        if let Some(existing_size) = referenced.insert(reference.oid.clone(), reference.size)
            && existing_size != reference.size
        {
            bail!(
                "committed refs disagree about the size of {}: {} and {}",
                reference.oid,
                existing_size,
                reference.size
            );
        }
    }
    Ok(referenced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::testing::MemoryStore;
    use crate::config::Config;
    use crate::model::ShadowRef;
    use std::fs;
    use std::path::Path;
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

    #[test]
    fn scans_ref_versions_from_reachable_git_history() {
        let temp = TempDir::new().unwrap();
        let repo = setup_repository(&temp);
        let first = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        let second = ObjectId::from_sha256_hex("b".repeat(64)).unwrap();
        commit_ref(&temp, first.clone(), "first ref");
        commit_ref(&temp, second.clone(), "second ref");

        let referenced = reachable_objects(&repo.root).unwrap();

        assert_eq!(referenced, BTreeMap::from([(first, 1), (second, 1)]));
    }

    #[tokio::test]
    async fn dry_run_keeps_and_delete_removes_only_unreferenced_objects() {
        let temp = TempDir::new().unwrap();
        let repo = setup_repository(&temp);
        let referenced_oid = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        let unused_oid = ObjectId::from_sha256_hex("b".repeat(64)).unwrap();
        commit_ref(&temp, referenced_oid.clone(), "referenced");
        let referenced_key = BlobKey::for_object("test", &referenced_oid);
        let unused_key = BlobKey::for_object("test", &unused_oid);
        let store = MemoryStore::default();
        store.insert_without_metadata(&referenced_key, vec![1]);
        store.insert_without_metadata(&unused_key, vec![2]);
        let now = SystemTime::now() + Duration::from_secs(1);

        gc_with_inventory(&repo, &store, false, 0, now)
            .await
            .unwrap();
        assert!(store.contains(&referenced_key));
        assert!(store.contains(&unused_key));

        gc_with_inventory(&repo, &store, true, 0, now)
            .await
            .unwrap();
        assert!(store.contains(&referenced_key));
        assert!(!store.contains(&unused_key));
        assert!(repo.ref_path(Path::new("a.bin")).unwrap().exists());
    }

    #[tokio::test]
    async fn refuses_deletion_when_a_committed_ref_is_missing_remotely() {
        let temp = TempDir::new().unwrap();
        let repo = setup_repository(&temp);
        let referenced_oid = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        let unused_oid = ObjectId::from_sha256_hex("b".repeat(64)).unwrap();
        commit_ref(&temp, referenced_oid, "referenced");
        let unused_key = BlobKey::for_object("test", &unused_oid);
        let store = MemoryStore::default();
        store.insert_without_metadata(&unused_key, vec![2]);

        let result = gc_with_inventory(
            &repo,
            &store,
            true,
            0,
            SystemTime::now() + Duration::from_secs(1),
        )
        .await;

        assert!(result.is_err());
        assert!(store.contains(&unused_key));
    }
}
