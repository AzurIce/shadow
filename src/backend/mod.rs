mod tos;

use crate::config::Config;
use crate::model::{BlobKey, BlobMetadata};
use anyhow::{Result, bail};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

pub use tos::TosStore;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendErrorKind {
    NotFound,
    Unauthorized,
    Forbidden,
    RateLimited,
    Timeout,
    IntegrityMismatch,
    Unavailable,
    Unsupported,
    Other,
}

#[derive(Debug, Error)]
#[error("{kind:?}: {message}")]
pub struct BackendError {
    pub kind: BackendErrorKind,
    pub message: String,
    pub request_id: Option<String>,
}

impl BackendError {
    pub fn new(kind: BackendErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            request_id: None,
        }
    }
}

pub type BackendResult<T> = std::result::Result<T, BackendError>;

#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn stat(&self, key: &BlobKey) -> BackendResult<Option<BlobMetadata>>;
    async fn upload_file(&self, key: &BlobKey, source: &Path, size: u64) -> BackendResult<()>;
    async fn download_file(&self, key: &BlobKey, destination: &Path) -> BackendResult<()>;
}

pub fn open(config: &Config) -> Result<Arc<dyn BlobStore>> {
    let Some(backend) = &config.backend else {
        bail!("backend is not configured in shadow.toml");
    };
    match backend.kind.as_str() {
        "volcengine_tos" => Ok(Arc::new(TosStore::new(backend)?)),
        other => bail!("unsupported backend type: {other}"),
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryStore {
        objects: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MemoryStore {
        pub fn contains(&self, key: &BlobKey) -> bool {
            self.objects.lock().unwrap().contains_key(key.as_str())
        }
    }

    #[async_trait]
    impl BlobStore for MemoryStore {
        async fn stat(&self, key: &BlobKey) -> BackendResult<Option<BlobMetadata>> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .get(key.as_str())
                .map(|data| BlobMetadata {
                    size: data.len() as u64,
                    etag: None,
                }))
        }

        async fn upload_file(&self, key: &BlobKey, source: &Path, _: u64) -> BackendResult<()> {
            let data = fs::read(source)
                .map_err(|error| BackendError::new(BackendErrorKind::Other, error.to_string()))?;
            self.objects
                .lock()
                .unwrap()
                .insert(key.as_str().to_string(), data);
            Ok(())
        }

        async fn download_file(&self, key: &BlobKey, destination: &Path) -> BackendResult<()> {
            let data = self
                .objects
                .lock()
                .unwrap()
                .get(key.as_str())
                .cloned()
                .ok_or_else(|| BackendError::new(BackendErrorKind::NotFound, "object missing"))?;
            fs::write(destination, data)
                .map_err(|error| BackendError::new(BackendErrorKind::Other, error.to_string()))
        }
    }
}
