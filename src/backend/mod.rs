mod tos;

use crate::config::Config;
use crate::model::{BlobKey, BlobMetadata, UploadOptions};
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
    async fn upload_file(
        &self,
        key: &BlobKey,
        source: &Path,
        size: u64,
        options: &UploadOptions,
    ) -> BackendResult<()>;
    async fn update_metadata(&self, key: &BlobKey, options: &UploadOptions) -> BackendResult<()>;
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

    #[derive(Clone)]
    struct MemoryObject {
        data: Vec<u8>,
        content_type: Option<String>,
    }

    #[derive(Default)]
    pub struct MemoryStore {
        objects: Mutex<HashMap<String, MemoryObject>>,
        uploads: Mutex<usize>,
        metadata_updates: Mutex<usize>,
    }

    impl MemoryStore {
        pub fn contains(&self, key: &BlobKey) -> bool {
            self.objects.lock().unwrap().contains_key(key.as_str())
        }

        pub fn content_type(&self, key: &BlobKey) -> Option<String> {
            self.objects
                .lock()
                .unwrap()
                .get(key.as_str())
                .and_then(|object| object.content_type.clone())
        }

        pub fn insert_legacy(&self, key: &BlobKey, data: Vec<u8>) {
            self.objects.lock().unwrap().insert(
                key.as_str().to_string(),
                MemoryObject {
                    data,
                    content_type: None,
                },
            );
        }

        pub fn upload_count(&self) -> usize {
            *self.uploads.lock().unwrap()
        }

        pub fn metadata_update_count(&self) -> usize {
            *self.metadata_updates.lock().unwrap()
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
                .map(|object| BlobMetadata {
                    size: object.data.len() as u64,
                    etag: None,
                    content_type: object.content_type.clone(),
                }))
        }

        async fn upload_file(
            &self,
            key: &BlobKey,
            source: &Path,
            _: u64,
            options: &UploadOptions,
        ) -> BackendResult<()> {
            let data = fs::read(source)
                .map_err(|error| BackendError::new(BackendErrorKind::Other, error.to_string()))?;
            self.objects.lock().unwrap().insert(
                key.as_str().to_string(),
                MemoryObject {
                    data,
                    content_type: Some(options.content_type.clone()),
                },
            );
            *self.uploads.lock().unwrap() += 1;
            Ok(())
        }

        async fn update_metadata(
            &self,
            key: &BlobKey,
            options: &UploadOptions,
        ) -> BackendResult<()> {
            let mut objects = self.objects.lock().unwrap();
            let object = objects
                .get_mut(key.as_str())
                .ok_or_else(|| BackendError::new(BackendErrorKind::NotFound, "object missing"))?;
            object.content_type = Some(options.content_type.clone());
            *self.metadata_updates.lock().unwrap() += 1;
            Ok(())
        }

        async fn download_file(&self, key: &BlobKey, destination: &Path) -> BackendResult<()> {
            let data = self
                .objects
                .lock()
                .unwrap()
                .get(key.as_str())
                .map(|object| object.data.clone())
                .ok_or_else(|| BackendError::new(BackendErrorKind::NotFound, "object missing"))?;
            fs::write(destination, data)
                .map_err(|error| BackendError::new(BackendErrorKind::Other, error.to_string()))
        }
    }
}
