use crate::config::RemoteConfig;
use crate::utils::get_s3_key;
use anyhow::{Context, Result, anyhow};
use s3::bucket::Bucket;
use s3::creds::Credentials;
// use s3::error::S3Error;
use s3::region::Region;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

pub struct RemoteClient {
    bucket: Box<Bucket>,
}

impl RemoteClient {
    pub fn new(config: &RemoteConfig) -> Result<Self> {
        let region = Region::Custom {
            region: config.region.clone(),
            endpoint: config.endpoint.clone(),
        };

        // Try to load credentials from environment
        let credentials = Credentials::from_env()
            .or_else(|_| Credentials::new(None, None, None, None, None))?;

        let bucket = Bucket::new(&config.bucket, region, credentials)?;

        Ok(Self { bucket })
    }

    pub async fn exists(&self, hash: &str) -> Result<bool> {
        let key = get_s3_key(hash);
        match self.bucket.head_object(&key).await {
            Ok((_, 200)) => Ok(true),
            Ok(_) => Ok(false), // Should not happen for HeadObject success usually
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("404") {
                    Ok(false)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    pub async fn upload_file(&self, hash: &str, path: &Path) -> Result<()> {
        let key = get_s3_key(hash);
        
        let mut file = File::open(path).await.context("Failed to open file for upload")?;
        let metadata = file.metadata().await?;
        let size = metadata.len();
        
        let mut buffer = Vec::with_capacity(size as usize);
        file.read_to_end(&mut buffer).await?;
        
        self.bucket.put_object(&key, &buffer).await.context("Failed to upload object")?;
        
        Ok(())
    }

    pub async fn download_file(&self, hash: &str, target_path: &Path) -> Result<()> {
        let key = get_s3_key(hash);
        
        let response = self.bucket.get_object(&key).await.context("Failed to download object")?;
        
        if response.status_code() != 200 {
            return Err(anyhow!("Download failed with status: {}", response.status_code()));
        }
        
        tokio::fs::write(target_path, response.bytes()).await.context("Failed to write downloaded file")?;
        
        Ok(())
    }
}
