use super::{BackendError, BackendErrorKind, BackendResult, BlobStore};
use crate::config::BackendConfig;
use crate::model::{BlobKey, BlobMetadata};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_core::future::BoxFuture;
use std::env;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use ve_tos_rust_sdk::asynchronous::multipart::MultipartAPI;
use ve_tos_rust_sdk::asynchronous::object::ObjectAPI;
use ve_tos_rust_sdk::asynchronous::tos::{self, AsyncRuntime, TosClientImpl};
use ve_tos_rust_sdk::credential::{CommonCredentials, CommonCredentialsProvider};
use ve_tos_rust_sdk::error::TosError;
use ve_tos_rust_sdk::multipart::{
    AbortMultipartUploadInput, CompleteMultipartUploadInput, CreateMultipartUploadInput,
    UploadPartFromFileInput, UploadedPart,
};
use ve_tos_rust_sdk::object::{GetObjectToFileInput, HeadObjectInput, PutObjectFromFileInput};

const MULTIPART_THRESHOLD: u64 = 64 * 1024 * 1024;
const PART_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default)]
struct TokioRuntime;

#[async_trait]
impl AsyncRuntime for TokioRuntime {
    type JoinError = tokio::task::JoinError;

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    fn spawn<'a, F>(&self, future: F) -> BoxFuture<'a, Result<F::Output, Self::JoinError>>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        Box::pin(Handle::current().spawn(future))
    }

    fn block_on<F: Future>(&self, future: F) -> F::Output {
        Handle::current().block_on(future)
    }
}

type Client =
    TosClientImpl<CommonCredentialsProvider<CommonCredentials>, CommonCredentials, TokioRuntime>;

pub struct TosStore {
    client: Arc<Client>,
    bucket: String,
    prefix: String,
}

impl TosStore {
    pub fn new(config: &BackendConfig) -> Result<Self> {
        let access_key = env::var("TOS_ACCESS_KEY").context("TOS_ACCESS_KEY is not set")?;
        let secret_key = env::var("TOS_SECRET_KEY").context("TOS_SECRET_KEY is not set")?;
        let security_token = env::var("TOS_SECURITY_TOKEN").unwrap_or_default();
        let client = tos::builder::<TokioRuntime>()
            .ak(access_key)
            .sk(secret_key)
            .security_token(security_token)
            .endpoint(&config.endpoint)
            .region(&config.region)
            .connection_timeout(10_000)
            .request_timeout(120_000)
            .max_retry_count(3)
            .build()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(Self {
            client: Arc::new(client),
            bucket: config.bucket.clone(),
            prefix: config.prefix.trim_matches('/').to_string(),
        })
    }

    fn full_key(&self, key: &BlobKey) -> String {
        if self.prefix.is_empty() {
            key.as_str().to_string()
        } else {
            format!("{}/{}", self.prefix, key.as_str())
        }
    }

    async fn upload_multipart(&self, key: &str, source: &Path, size: u64) -> BackendResult<()> {
        let created = self
            .client
            .create_multipart_upload(&CreateMultipartUploadInput::new(&self.bucket, key))
            .await
            .map_err(map_tos_error)?;
        let upload_id = created.upload_id().to_string();
        let source_path = source.to_str().ok_or_else(|| {
            BackendError::new(BackendErrorKind::Other, "source path is not UTF-8")
        })?;
        let part_size = PART_SIZE.max(size.div_ceil(10_000));
        let mut parts = Vec::new();
        let mut offset = 0_u64;
        let mut part_number = 1_isize;
        while offset < size {
            let length = part_size.min(size - offset);
            let mut input = UploadPartFromFileInput::new_with_part_number_file_path(
                &self.bucket,
                key,
                &upload_id,
                part_number,
                source_path,
            );
            input.set_offset(offset as i64);
            input.set_part_size(length as i64);
            match self.client.upload_part_from_file(&input).await {
                Ok(output) => parts.push(UploadedPart::new(part_number, output.etag())),
                Err(error) => {
                    let _ = self
                        .client
                        .abort_multipart_upload(&AbortMultipartUploadInput::new(
                            &self.bucket,
                            key,
                            &upload_id,
                        ))
                        .await;
                    return Err(map_tos_error(error));
                }
            }
            offset += length;
            part_number += 1;
        }
        self.client
            .complete_multipart_upload(&CompleteMultipartUploadInput::new_with_parts(
                &self.bucket,
                key,
                &upload_id,
                parts,
            ))
            .await
            .map_err(map_tos_error)?;
        Ok(())
    }
}

#[async_trait]
impl BlobStore for TosStore {
    async fn stat(&self, key: &BlobKey) -> BackendResult<Option<BlobMetadata>> {
        let key = self.full_key(key);
        match self
            .client
            .head_object(&HeadObjectInput::new(&self.bucket, &key))
            .await
        {
            Ok(output) => Ok(Some(BlobMetadata {
                size: output.content_length().max(0) as u64,
                etag: Some(output.etag().to_string()),
            })),
            Err(error) if is_not_found(&error) => Ok(None),
            Err(error) => Err(map_tos_error(error)),
        }
    }

    async fn upload_file(&self, key: &BlobKey, source: &Path, size: u64) -> BackendResult<()> {
        let key = self.full_key(key);
        if size >= MULTIPART_THRESHOLD {
            return self.upload_multipart(&key, source, size).await;
        }
        let source = source.to_str().ok_or_else(|| {
            BackendError::new(BackendErrorKind::Other, "source path is not UTF-8")
        })?;
        self.client
            .put_object_from_file(&PutObjectFromFileInput::new_with_file_path(
                &self.bucket,
                &key,
                source,
            ))
            .await
            .map_err(map_tos_error)?;
        Ok(())
    }

    async fn download_file(&self, key: &BlobKey, destination: &Path) -> BackendResult<()> {
        let key = self.full_key(key);
        let destination = destination.to_str().ok_or_else(|| {
            BackendError::new(BackendErrorKind::Other, "destination path is not UTF-8")
        })?;
        self.client
            .get_object_to_file(&GetObjectToFileInput::new(&self.bucket, &key, destination))
            .await
            .map_err(map_tos_error)?;
        Ok(())
    }
}

fn is_not_found(error: &TosError) -> bool {
    error
        .as_server_error()
        .is_some_and(|server| server.status_code() == 404)
}

fn map_tos_error(error: TosError) -> BackendError {
    if let Some(server) = error.as_server_error() {
        let kind = match server.status_code() {
            401 => BackendErrorKind::Unauthorized,
            403 => BackendErrorKind::Forbidden,
            404 => BackendErrorKind::NotFound,
            408 => BackendErrorKind::Timeout,
            429 => BackendErrorKind::RateLimited,
            500..=599 => BackendErrorKind::Unavailable,
            _ => BackendErrorKind::Other,
        };
        return BackendError {
            kind,
            message: format!(
                "TOS status={} code={} request_id={} message={}",
                server.status_code(),
                server.code(),
                server.request_id(),
                server.message()
            ),
            request_id: Some(server.request_id().to_string()),
        };
    }
    BackendError::new(BackendErrorKind::Other, error.to_string())
}
