use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub language: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
        }
    }
}
