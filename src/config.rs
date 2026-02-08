use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub core: CoreConfig,

    #[serde(default)]
    pub remote: HashMap<String, RemoteConfig>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CoreConfig {
    #[serde(default = "default_auto_add")]
    pub auto_add_to_gitignore: bool,
}

fn default_auto_add() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemoteConfig {
    pub provider: String,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        let root = crate::utils::find_project_root()?;
        let path = root.join(".shadow").join("config.toml");

        if !path.exists() {
            // Should not happen if find_project_root succeeded, unless config is missing inside .shadow
            return Err(anyhow!(
                "Configuration file not found at {:?}. Run 'git-shadow init' first.",
                path
            ));
        }

        let content = fs::read_to_string(&path).context("Failed to read .shadow/config.toml")?;
        let config: Config =
            toml::from_str(&content).context("Failed to parse .shadow/config.toml")?;

        Ok(config)
    }

    pub fn get_remote(&self, name: &str) -> Option<&RemoteConfig> {
        self.remote.get(name)
    }
}
