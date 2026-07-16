use crate::backend::{self, BlobStore};
use crate::context;
use crate::hash::hash_file;
use crate::media_type;
use crate::model::DEFAULT_CACHE_CONTROL;
use crate::repository::Repository;
use anyhow::Result;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

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
    pub remote: Option<&'static str>,
}

pub async fn run(check_remote: bool) -> Result<()> {
    let repo = context::discover()?;
    let store = if check_remote {
        Some(backend::open(&repo.config)?)
    } else {
        None
    };
    let records = collect(&repo, store.as_deref()).await?;
    print!("{}", render(&records));
    Ok(())
}

pub async fn collect(
    repo: &Repository,
    store: Option<&dyn BlobStore>,
) -> Result<Vec<StatusRecord>> {
    let worktree = repo.managed_files()?;
    let refs = repo.load_refs()?;
    let paths: BTreeSet<_> = worktree.keys().chain(refs.keys()).cloned().collect();
    let mut records = Vec::new();

    for path in paths {
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
        let remote = match (reference, store) {
            (Some(reference), Some(store)) => {
                let key = repo.blob_key(&reference.oid);
                let cache = repo.cache_path(&reference.oid);
                let content = cache.is_file().then_some(cache.as_path()).or_else(|| {
                    (state == LocalState::Published)
                        .then(|| body.map(PathBuf::as_path))
                        .flatten()
                });
                let content_type = media_type::detect(&path, content)?;
                match store.stat(&key).await? {
                    None => Some("missing"),
                    Some(metadata)
                        if metadata.size == reference.size
                            && metadata.content_type.as_deref() == Some(content_type.as_str())
                            && metadata.cache_control.as_deref() == Some(DEFAULT_CACHE_CONTROL) =>
                    {
                        Some("ok")
                    }
                    Some(_) => Some("invalid"),
                }
            }
            _ => None,
        };
        records.push(StatusRecord {
            path,
            state,
            remote,
        });
    }
    Ok(records)
}

pub fn render(records: &[StatusRecord]) -> String {
    let mut output = String::new();
    write_group(
        &mut output,
        "Changes not published:",
        "  (run \"shadow publish\" to publish worktree content)",
        records.iter().filter_map(|record| match record.state {
            LocalState::Unpublished => Some(("new file", &record.path)),
            LocalState::Modified => Some(("modified", &record.path)),
            _ => None,
        }),
    );
    write_group(
        &mut output,
        "Files missing from the worktree:",
        "  (run \"shadow restore\" to restore them)",
        records.iter().filter_map(|record| {
            (record.state == LocalState::Missing).then_some(("missing", &record.path))
        }),
    );
    write_group(
        &mut output,
        "Orphaned references:",
        "  (remove the corresponding files under .shadow/refs)",
        records.iter().filter_map(|record| {
            (record.state == LocalState::Orphaned).then_some(("orphaned", &record.path))
        }),
    );
    write_group(
        &mut output,
        "Remote object issues:",
        "  (run \"shadow publish\" to repair remote objects)",
        records.iter().filter_map(|record| match record.remote {
            Some("missing") => Some(("missing", &record.path)),
            Some("invalid") => Some(("invalid", &record.path)),
            _ => None,
        }),
    );
    if output.is_empty() {
        output.push_str("Shadow working tree clean.\n");
    }
    output
}

fn write_group<'a>(
    output: &mut String,
    heading: &str,
    hint: &str,
    entries: impl Iterator<Item = (&'static str, &'a PathBuf)>,
) {
    let entries: Vec<_> = entries.collect();
    if entries.is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    writeln!(output, "{heading}").unwrap();
    writeln!(output, "{hint}").unwrap();
    for (label, path) in entries {
        writeln!(output, "\t{label:<10} {}", path.display()).unwrap();
    }
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
        let repo = Repository::from_parts(temp.path().to_path_buf(), Config::new("test").unwrap())
            .unwrap();
        let store = MemoryStore::default();
        publish_with_store(&repo, &store).await.unwrap();

        fs::write(temp.path().join("a.bin"), b"changed").unwrap();
        fs::remove_file(temp.path().join("b.bin")).unwrap();
        let records = collect(&repo, None).await.unwrap();
        assert_eq!(records[0].state, LocalState::Modified);
        assert_eq!(records[1].state, LocalState::Missing);
    }

    #[test]
    fn renders_only_actionable_groups() {
        let records = vec![
            StatusRecord {
                path: PathBuf::from("published.bin"),
                state: LocalState::Published,
                remote: Some("ok"),
            },
            StatusRecord {
                path: PathBuf::from("new.bin"),
                state: LocalState::Unpublished,
                remote: None,
            },
            StatusRecord {
                path: PathBuf::from("missing.bin"),
                state: LocalState::Missing,
                remote: Some("missing"),
            },
        ];

        let output = render(&records);

        assert!(output.contains("Changes not published:"));
        assert!(output.contains("new.bin"));
        assert!(output.contains("Files missing from the worktree:"));
        assert!(output.contains("Remote object issues:"));
        assert!(!output.contains("published.bin"));
    }

    #[test]
    fn renders_clean_message() {
        let records = vec![StatusRecord {
            path: PathBuf::from("published.bin"),
            state: LocalState::Published,
            remote: None,
        }];
        assert_eq!(render(&records), "Shadow working tree clean.\n");
    }
}
