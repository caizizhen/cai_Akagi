use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    pub enabled: bool,
    pub addr: String,
    pub ca_dir: PathBuf,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            addr: "127.0.0.1:23410".to_string(),
            ca_dir: PathBuf::from("./ca"),
        }
    }
}
