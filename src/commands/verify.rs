use crate::backend;
use crate::commands::status::{collect, state_requires_attention};
use crate::context;
use anyhow::{Result, bail};

pub async fn run(check_remote: bool) -> Result<()> {
    let repo = context::discover()?;
    let store = if check_remote {
        Some(backend::open(&repo.config)?)
    } else {
        None
    };
    let records = collect(&repo, &[], store.as_deref()).await?;
    let refs = repo.load_refs()?;
    let mut issues = 0_usize;
    for record in records {
        let remote_issue = record.remote.is_some_and(|state| state != "ok");
        if state_requires_attention(record.state) || remote_issue {
            eprintln!(
                "[issue] {:?} {} cache={}{}",
                record.state,
                record.path.display(),
                record.cache,
                record
                    .remote
                    .map(|state| format!(" remote={state}"))
                    .unwrap_or_default()
            );
            issues += 1;
        }
    }
    for (path, reference) in refs {
        let cache_path = repo.cache_path(&reference.oid);
        if cache_path.exists() && !repo.validate_cache(&reference)? {
            eprintln!("[issue] invalid cache object for {}", path.display());
            issues += 1;
        }
    }
    if issues > 0 {
        bail!("verification found {issues} issue(s)");
    }
    println!("Shadow repository verified.");
    Ok(())
}
