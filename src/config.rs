use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use uuid::Uuid;

pub const CONFIG_FILE: &str = "shadow.toml";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub version: u32,
    pub repository_id: String,
    #[serde(default)]
    pub backend: Option<BackendConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BackendConfig {
    #[serde(rename = "type")]
    pub kind: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    #[serde(default = "default_prefix")]
    pub prefix: String,
}

fn default_prefix() -> String {
    "shadow".to_string()
}

impl Config {
    pub fn new() -> Self {
        Self {
            version: 1,
            repository_id: Uuid::new_v4().to_string(),
            backend: None,
        }
    }

    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(CONFIG_FILE);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported shadow.toml version: {}", self.version);
        }
        Uuid::parse_str(&self.repository_id).context("repository_id must be a UUID")?;
        if let Some(backend) = &self.backend {
            if backend.kind != "volcengine_tos" {
                bail!("unsupported backend type: {}", backend.kind);
            }
            if backend.endpoint.trim().is_empty()
                || backend.region.trim().is_empty()
                || backend.bucket.trim().is_empty()
            {
                bail!("backend endpoint, region, and bucket must not be empty");
            }
            validate_prefix(&backend.prefix)?;
        }
        Ok(())
    }

    pub fn initial_document(&self) -> String {
        format!(
            "version = 1\nrepository_id = \"{}\"\n\n# [backend]\n# type = \"volcengine_tos\"\n# endpoint = \"https://tos-cn-beijing.volces.com\"\n# region = \"cn-beijing\"\n# bucket = \"example-shadow\"\n# prefix = \"shadow\"\n",
            self.repository_id
        )
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_prefix(prefix: &str) -> Result<()> {
    if prefix.starts_with('/') || prefix.ends_with('/') {
        bail!("backend prefix must not start or end with '/'");
    }
    if prefix.split('/').any(|part| part == "..") {
        bail!("backend prefix must not contain '..' components");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_prefix() {
        assert!(validate_prefix("a/../b").is_err());
    }
}
