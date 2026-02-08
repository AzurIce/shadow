use crate::object::ObjectMetadata;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Default)]
pub struct StagingIndex {
    // Map relative_path -> Metadata
    pub entries: HashMap<String, ObjectMetadata>,
}

impl StagingIndex {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".shadow").join("index.json");
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path).context("Failed to read staging index")?;
        let entries = serde_json::from_str(&content).unwrap_or_default();
        Ok(Self { entries })
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(".shadow").join("index.json");
        let content = serde_json::to_string_pretty(&self.entries)?;
        fs::write(path, content).context("Failed to write staging index")?;
        Ok(())
    }

    pub fn add(&mut self, path: String, metadata: ObjectMetadata) {
        self.entries.insert(path, metadata);
    }

    pub fn remove(&mut self, path: &str) {
        self.entries.remove(path);
    }
}
