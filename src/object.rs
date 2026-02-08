use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectMetadata {
    pub hash: String, // e.g. "sha256:abc..."
    pub size: u64,
    pub algorithm: String, // "sha256"
    pub created_at: u64,
}

impl ObjectMetadata {
    pub fn new(hash: String, size: u64) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            hash,
            size,
            algorithm: "sha256".to_string(),
            created_at: now,
        }
    }
}
