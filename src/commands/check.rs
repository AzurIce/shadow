use crate::backend;
use crate::commands::status::{LocalState, collect};
use crate::context;
use anyhow::{Result, bail};

pub async fn run(check_remote: bool) -> Result<()> {
    let repo = context::discover()?;
    let store = if check_remote {
        Some(backend::open(&repo.config)?)
    } else {
        None
    };
    let records = collect(&repo, store.as_deref()).await?;
    let refs = repo.load_refs()?;
    let mut issues = 0_usize;

    for record in records {
        let message = match record.state {
            LocalState::Unpublished => Some("unpublished worktree file"),
            LocalState::Modified => Some("worktree file differs from ref"),
            LocalState::Missing => Some("worktree file is missing"),
            LocalState::Orphaned => Some("ref is outside the # shadow rules"),
            LocalState::Published => None,
        };
        if let Some(message) = message {
            eprintln!("error: {message}: {}", record.path.display());
            issues += 1;
        }
        if let Some(remote) = record.remote.filter(|state| *state != "ok") {
            eprintln!(
                "error: remote object is {remote}: {}",
                record.path.display()
            );
            issues += 1;
        }
    }

    for (path, reference) in refs {
        let cache_path = repo.cache_path(&reference.oid);
        if cache_path.exists() && !repo.validate_cache(&reference)? {
            eprintln!("error: cache object is invalid: {}", path.display());
            issues += 1;
        }
    }

    if issues > 0 {
        bail!("Shadow check failed with {issues} issue(s)");
    }
    println!("Shadow repository is healthy.");
    Ok(())
}
