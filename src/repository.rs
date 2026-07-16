use crate::config::Config;
use crate::hash::{finalize_sha256, hash_file};
use crate::model::{BlobKey, ObjectId, ShadowRef};
use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

pub const SHADOW_DIR: &str = ".shadow";
pub const SHADOW_MARKER: &str = "# shadow";

pub struct Repository {
    pub root: PathBuf,
    pub config: Config,
    rules: ShadowRules,
}

struct ShadowRules {
    managed: Gitignore,
}

impl Repository {
    pub fn load(root: PathBuf) -> Result<Self> {
        let config = Config::load(&root)?;
        Self::from_parts(root, config)
    }

    pub fn from_parts(root: PathBuf, config: Config) -> Result<Self> {
        let rules = ShadowRules::load(&root)?;
        Ok(Self {
            root,
            config,
            rules,
        })
    }

    pub fn refs_dir(&self) -> PathBuf {
        self.root.join(SHADOW_DIR).join("refs")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join(SHADOW_DIR).join("cache").join("objects")
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join(SHADOW_DIR).join("tmp")
    }

    pub fn managed_files(&self) -> Result<BTreeMap<PathBuf, PathBuf>> {
        let mut files = BTreeMap::new();
        let root = self.root.clone();

        let walker = WalkBuilder::new(&root)
            .standard_filters(false)
            .hidden(false)
            .follow_links(false)
            .filter_entry(move |entry| {
                if entry.depth() == 0 {
                    return true;
                }
                let Ok(relative) = entry.path().strip_prefix(&root) else {
                    return false;
                };
                if first_component_is(relative, ".git") || first_component_is(relative, SHADOW_DIR)
                {
                    return false;
                }
                true
            })
            .build();

        for entry in walker {
            let entry = entry.context("failed while scanning worktree")?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .context("worktree path escaped repository")?
                .to_path_buf();
            if self
                .rules
                .managed
                .matched_path_or_any_parents(&relative, false)
                .is_ignore()
            {
                files.insert(relative, entry.into_path());
            }
        }
        Ok(files)
    }

    pub fn load_refs(&self) -> Result<BTreeMap<PathBuf, ShadowRef>> {
        let mut refs = BTreeMap::new();
        let refs_dir = self.refs_dir();
        if !refs_dir.exists() {
            return Ok(refs);
        }
        let walker = WalkBuilder::new(&refs_dir)
            .standard_filters(false)
            .hidden(false)
            .build();
        for entry in walker {
            let entry = entry.context("failed while scanning refs")?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.path();
            let relative_ref = path
                .strip_prefix(&refs_dir)
                .context("ref escaped refs directory")?;
            let Some(relative_source) = source_path_from_ref(relative_ref) else {
                continue;
            };
            let content = fs::read_to_string(path)
                .with_context(|| format!("failed to read ref {}", path.display()))?;
            let reference = ShadowRef::parse(&content)
                .with_context(|| format!("invalid ref {}", path.display()))?;
            refs.insert(relative_source, reference);
        }
        Ok(refs)
    }

    pub fn is_managed_path(&self, relative: &Path) -> bool {
        self.rules
            .managed
            .matched_path_or_any_parents(relative, false)
            .is_ignore()
    }

    pub fn ref_path(&self, relative: &Path) -> Result<PathBuf> {
        validate_relative_path(relative)?;
        let mut path = self.refs_dir().join(relative);
        let name = path
            .file_name()
            .context("managed path has no file name")?
            .to_os_string();
        let mut ref_name = name;
        ref_name.push(".ref");
        path.set_file_name(ref_name);
        Ok(path)
    }

    pub fn cache_path(&self, oid: &ObjectId) -> PathBuf {
        self.cache_dir()
            .join("sha256")
            .join(&oid.hex()[..2])
            .join(&oid.hex()[2..])
    }

    pub fn blob_key(&self, oid: &ObjectId) -> BlobKey {
        BlobKey::for_object(&self.config.name, oid)
    }

    pub fn write_ref(&self, relative: &Path, reference: &ShadowRef) -> Result<()> {
        let target = self.ref_path(relative)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = reference.serialize()?;
        write_atomic(&target, content.as_bytes())
    }

    pub fn import_to_cache(&self, source: &Path) -> Result<ShadowRef> {
        fs::create_dir_all(self.tmp_dir())?;
        let temporary = self.tmp_dir().join(format!("{}.import", Uuid::new_v4()));
        let input =
            File::open(source).with_context(|| format!("failed to open {}", source.display()))?;
        let output = File::create(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        let mut reader = BufReader::with_capacity(1024 * 1024, input);
        let mut writer = BufWriter::with_capacity(1024 * 1024, output);
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            writer.write_all(&buffer[..read])?;
            size += read as u64;
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;

        let oid = finalize_sha256(hasher)?;
        let cache_path = self.cache_path(&oid);
        if cache_path.exists() {
            let (cached_oid, cached_size) = hash_file(&cache_path)?;
            if cached_oid != oid || cached_size != size {
                let _ = fs::remove_file(&temporary);
                bail!("cache object {} failed integrity validation", oid);
            }
            fs::remove_file(&temporary)?;
        } else {
            if let Some(parent) = cache_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&temporary, &cache_path).with_context(|| {
                format!(
                    "failed to move imported object into cache {}",
                    cache_path.display()
                )
            })?;
        }
        Ok(ShadowRef { oid, size })
    }

    pub fn validate_cache(&self, reference: &ShadowRef) -> Result<bool> {
        let path = self.cache_path(&reference.oid);
        if !path.is_file() {
            return Ok(false);
        }
        let (oid, size) = hash_file(&path)?;
        Ok(oid == reference.oid && size == reference.size)
    }

    pub fn cache_present(&self, reference: &ShadowRef) -> Result<bool> {
        let path = self.cache_path(&reference.oid);
        if !path.is_file() {
            return Ok(false);
        }
        Ok(fs::metadata(path)?.len() == reference.size)
    }

    pub fn promote_download(&self, temporary: &Path, reference: &ShadowRef) -> Result<PathBuf> {
        let (oid, size) = hash_file(temporary)?;
        if oid != reference.oid || size != reference.size {
            bail!("downloaded object failed SHA-256 or size validation");
        }
        let target = self.cache_path(&reference.oid);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if target.exists() {
            fs::remove_file(temporary)?;
        } else {
            fs::rename(temporary, &target)?;
        }
        Ok(target)
    }

    pub fn materialize(&self, cache: &Path, target: &Path, replace: bool) -> Result<()> {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let file_name = target
            .file_name()
            .context("target has no file name")?
            .to_string_lossy();
        let temporary = target.with_file_name(format!(".{}.shadow-{}", file_name, Uuid::new_v4()));
        fs::copy(cache, &temporary)
            .with_context(|| format!("failed to copy cache object to {}", temporary.display()))?;
        replace_path(&temporary, target, replace)
    }
}

impl ShadowRules {
    fn load(root: &Path) -> Result<Self> {
        let gitignore_path = root.join(".gitignore");
        let content = if gitignore_path.exists() {
            fs::read_to_string(&gitignore_path)
                .with_context(|| format!("failed to read {}", gitignore_path.display()))?
        } else {
            String::new()
        };
        let marker_count = content
            .lines()
            .filter(|line| *line == SHADOW_MARKER)
            .count();
        if marker_count > 1 {
            bail!(
                ".gitignore contains more than one '{}' marker",
                SHADOW_MARKER
            );
        }
        let mut managed_builder = GitignoreBuilder::new(root);
        let mut in_shadow = false;
        for line in content.lines() {
            if line == SHADOW_MARKER {
                in_shadow = true;
                continue;
            }
            if in_shadow {
                managed_builder.add_line(Some(gitignore_path.clone()), line)?;
            }
        }
        Ok(Self {
            managed: managed_builder.build()?,
        })
    }
}

fn first_component_is(path: &Path, expected: &str) -> bool {
    path.components()
        .next()
        .is_some_and(|component| matches!(component, Component::Normal(value) if value == expected))
}

fn source_path_from_ref(relative_ref: &Path) -> Option<PathBuf> {
    let name = relative_ref.file_name()?.to_string_lossy();
    let source_name = name.strip_suffix(".ref")?;
    let mut source = relative_ref.to_path_buf();
    source.set_file_name(source_name);
    Some(source)
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("managed path must be a non-empty relative path");
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!(
            "managed path contains invalid components: {}",
            path.display()
        );
    }
    Ok(())
}

fn write_atomic(target: &Path, content: &[u8]) -> Result<()> {
    let parent = target.parent().context("target has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".shadow-write-{}", Uuid::new_v4()));
    {
        let mut file = File::create(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
    }
    replace_path(&temporary, target, true)
}

fn replace_path(temporary: &Path, target: &Path, replace: bool) -> Result<()> {
    if !target.exists() {
        fs::rename(temporary, target)?;
        return Ok(());
    }
    if !replace {
        let _ = fs::remove_file(temporary);
        bail!("refusing to replace existing file {}", target.display());
    }
    let mut backup_name = OsString::from(".shadow-backup-");
    backup_name.push(Uuid::new_v4().to_string());
    let backup = target.with_file_name(backup_name);
    fs::rename(target, &backup)?;
    if let Err(error) = fs::rename(temporary, target) {
        let _ = fs::rename(&backup, target);
        return Err(error.into());
    }
    fs::remove_file(backup)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn repository(temp: &TempDir, gitignore: &str) -> Repository {
        fs::write(temp.path().join(".gitignore"), gitignore).unwrap();
        fs::create_dir_all(temp.path().join(".shadow/refs")).unwrap();
        Repository::from_parts(temp.path().to_path_buf(), Config::new("test").unwrap()).unwrap()
    }

    #[test]
    fn scans_only_shadow_section() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("assets")).unwrap();
        fs::write(temp.path().join("assets/a.bin"), b"a").unwrap();
        fs::write(temp.path().join("assets/a.log"), b"b").unwrap();
        let repo = repository(&temp, "*.log\n# shadow\n/assets/**/*.bin\n");
        let files = repo.managed_files().unwrap();
        assert!(files.contains_key(Path::new("assets/a.bin")));
        assert!(!files.contains_key(Path::new("assets/a.log")));
    }

    #[test]
    fn maps_ref_paths() {
        let temp = TempDir::new().unwrap();
        let repo = repository(&temp, "# shadow\n*.bin\n");
        assert_eq!(
            repo.ref_path(Path::new("models/a.bin")).unwrap(),
            temp.path().join(".shadow/refs/models/a.bin.ref")
        );
    }

    #[test]
    fn honors_gitignore_negation_in_shadow_section() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("assets")).unwrap();
        fs::write(temp.path().join("assets/a.bin"), b"a").unwrap();
        fs::write(temp.path().join("assets/keep.bin"), b"b").unwrap();
        let repo = repository(&temp, "# shadow\n/assets/*.bin\n!/assets/keep.bin\n");
        let files = repo.managed_files().unwrap();
        assert!(files.contains_key(Path::new("assets/a.bin")));
        assert!(!files.contains_key(Path::new("assets/keep.bin")));
    }

    #[test]
    fn rejects_duplicate_markers() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join(".gitignore"),
            "# shadow\n*.bin\n# shadow\n",
        )
        .unwrap();
        assert!(
            Repository::from_parts(temp.path().to_path_buf(), Config::new("test").unwrap())
                .is_err()
        );
    }
}
